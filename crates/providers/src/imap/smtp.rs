//! SMTP submission (RFC 6409) for IMAP accounts (§9.5, §10.2).
//!
//! A small client over the same rustls transport as IMAP: implicit TLS (465) or STARTTLS (587),
//! `AUTH PLAIN` / `LOGIN` / `XOAUTH2`, `SIZE`, `8BITMIME`, `SMTPUTF8`, dot-stuffing. It is written
//! here rather than taken from a crate because the send pipeline needs to know exactly *where* a
//! failure happened: a failure before the message upload began certainly did not send (the op may
//! be retried), while a broken connection after `DATA` was accepted is ambiguous and must only be
//! verified, never re-sent (§5.6). [`SmtpError::ambiguous`] carries that distinction.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::config::{Credentials, ImapAccountConfig, Security, ServerEndpoint, is_loopback_host};
use super::tls::{MailStream, TransportError, tcp_connect, upgrade};
use crate::{LimitScope, ProviderError};

/// Where in the transaction an error happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// TCP / TLS / greeting / EHLO / STARTTLS.
    Connect,
    /// `AUTH`.
    Auth,
    /// `MAIL FROM` / `RCPT TO` / `DATA` (before upload).
    Envelope,
    /// Upload of the message and the final reply.
    Upload,
}

/// An SMTP failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SmtpError {
    /// Network, TLS or timeout.
    #[error("smtp {stage:?}: {message}")]
    Transport {
        /// Stage.
        stage: Stage,
        /// Detail (no message content).
        message: String,
        /// TLS-specific (for the connect endpoint's `tls_failed`).
        tls: bool,
    },
    /// The server answered with an error reply.
    #[error("smtp {stage:?}: {code} {text}")]
    Reply {
        /// Stage.
        stage: Stage,
        /// Reply code.
        code: u16,
        /// Reply text (server text, trimmed).
        text: String,
    },
    /// Local refusal (no recipients, too large, plaintext to a remote host).
    #[error("smtp: {0}")]
    Local(String),
}

/// Prefix of the [`ProviderError::Transient`] message for an ambiguous send (§5.6). The engine
/// checks it with [`is_ambiguous_send`] and switches the op to `verifyOnly`.
pub const AMBIGUOUS_SEND: &str = "send_ambiguous";

/// `true` if a send error leaves it unknown whether the message went out.
pub fn is_ambiguous_send(e: &ProviderError) -> bool {
    matches!(e, ProviderError::Transient(m, _) if m.starts_with(AMBIGUOUS_SEND))
}

impl SmtpError {
    /// Whether the message may have been accepted.
    pub fn ambiguous(&self) -> bool {
        matches!(self, SmtpError::Transport { stage: Stage::Upload, .. })
    }

    /// The §5.5 classification.
    pub fn into_provider(self) -> ProviderError {
        if self.ambiguous() {
            return ProviderError::Transient(format!("{AMBIGUOUS_SEND}: {self}"), None);
        }
        match &self {
            SmtpError::Transport { .. } => ProviderError::Transient(self.to_string(), None),
            SmtpError::Local(m) => ProviderError::Invalid(m.clone()),
            SmtpError::Reply { stage, code, text } => {
                let enhanced = text.split_whitespace().next().unwrap_or_default();
                match (*stage, *code) {
                    (Stage::Auth, 535 | 534 | 530 | 538) => ProviderError::Auth { reauth_required: true },
                    (_, 421) => ProviderError::Transient(self.to_string(), None),
                    (_, 450..=452) if enhanced.starts_with("4.7.") || text.to_ascii_lowercase().contains("rate") => {
                        ProviderError::RateLimited { retry_after: None, scope: LimitScope::Account }
                    }
                    (_, 400..=499) => ProviderError::Transient(self.to_string(), None),
                    (_, 552) => ProviderError::Quota(format!("message rejected: {code}")),
                    (_, 554) if enhanced.starts_with("5.7.") => ProviderError::Invalid(self.to_string()),
                    _ => ProviderError::Invalid(self.to_string()),
                }
            }
        }
    }

    /// `true` for credential rejections (the connect endpoint answers `smtp_auth_failed`).
    pub fn is_auth(&self) -> bool {
        matches!(self, SmtpError::Reply { stage: Stage::Auth, .. })
            || matches!(self, SmtpError::Local(m) if m.contains("AUTH"))
    }

    /// `true` for TLS failures (`tls_failed`).
    pub fn is_tls(&self) -> bool {
        matches!(self, SmtpError::Transport { tls: true, .. })
    }
}

/// A reply: code and lines.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Reply {
    code: u16,
    lines: Vec<String>,
}

impl Reply {
    fn text(&self) -> String {
        self.lines.join(" ").chars().take(200).collect()
    }
    fn positive(&self) -> bool {
        (200..400).contains(&self.code)
    }
}

/// An open SMTP connection.
struct Smtp {
    io: BufReader<MailStream>,
    timeout: Duration,
    ehlo: Vec<String>,
}

fn transport(stage: Stage, e: TransportError) -> SmtpError {
    let tls = matches!(e, TransportError::Tls(_));
    SmtpError::Transport { stage, message: e.to_string(), tls }
}

impl Smtp {
    async fn read_reply(&mut self, stage: Stage) -> Result<Reply, SmtpError> {
        self.read_reply_within(stage, self.timeout).await
    }

    async fn read_reply_within(&mut self, stage: Stage, timeout: Duration) -> Result<Reply, SmtpError> {
        let mut lines = Vec::new();
        loop {
            let mut buf = Vec::new();
            let n = match tokio::time::timeout(timeout, (&mut self.io).take(4096).read_until(b'\n', &mut buf)).await {
                Err(_) => return Err(SmtpError::Transport { stage, message: "reply timed out".into(), tls: false }),
                Ok(Err(e)) => return Err(SmtpError::Transport { stage, message: e.kind().to_string(), tls: false }),
                Ok(Ok(n)) => n,
            };
            if n == 0 {
                return Err(SmtpError::Transport { stage, message: "connection closed".into(), tls: false });
            }
            let line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_owned();
            if line.len() < 3 {
                return Err(SmtpError::Transport { stage, message: "malformed reply".into(), tls: false });
            }
            let code: u16 = line[..3].parse().map_err(|_| SmtpError::Transport {
                stage,
                message: "malformed reply".into(),
                tls: false,
            })?;
            let more = line.as_bytes().get(3) == Some(&b'-');
            lines.push(line.get(4..).unwrap_or_default().to_owned());
            if !more {
                return Ok(Reply { code, lines });
            }
            if lines.len() > 512 {
                return Err(SmtpError::Transport { stage, message: "reply too long".into(), tls: false });
            }
        }
    }

    async fn send_line(&mut self, stage: Stage, line: &str) -> Result<(), SmtpError> {
        let io = self.io.get_mut();
        let fut = async {
            io.write_all(line.as_bytes()).await?;
            io.write_all(b"\r\n").await?;
            io.flush().await
        };
        match tokio::time::timeout(self.timeout, fut).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(SmtpError::Transport { stage, message: e.kind().to_string(), tls: false }),
            Err(_) => Err(SmtpError::Transport { stage, message: "write timed out".into(), tls: false }),
        }
    }

    async fn cmd(&mut self, stage: Stage, line: &str) -> Result<Reply, SmtpError> {
        self.send_line(stage, line).await?;
        let r = self.read_reply(stage).await?;
        if r.positive() { Ok(r) } else { Err(SmtpError::Reply { stage, code: r.code, text: r.text() }) }
    }

    fn has(&self, ext: &str) -> bool {
        self.ehlo.iter().any(|l| l.split_whitespace().next().is_some_and(|k| k.eq_ignore_ascii_case(ext)))
    }

    fn auth_mechs(&self) -> Vec<String> {
        self.ehlo
            .iter()
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                let k = w.next()?;
                (k.eq_ignore_ascii_case("AUTH") || k.eq_ignore_ascii_case("AUTH="))
                    .then(|| w.map(str::to_ascii_uppercase).collect::<Vec<_>>())
            })
            .flatten()
            .collect()
    }

    fn size_limit(&self) -> Option<u64> {
        self.ehlo.iter().find_map(|l| {
            let mut w = l.split_whitespace();
            w.next().filter(|k| k.eq_ignore_ascii_case("SIZE"))?;
            w.next()?.parse().ok().filter(|n| *n > 0)
        })
    }

    async fn ehlo(&mut self, name: &str) -> Result<(), SmtpError> {
        let r = match self.cmd(Stage::Connect, &format!("EHLO {name}")).await {
            Ok(r) => r,
            Err(SmtpError::Reply { .. }) => self.cmd(Stage::Connect, &format!("HELO {name}")).await?,
            Err(e) => return Err(e),
        };
        self.ehlo = r.lines.into_iter().skip(1).collect();
        Ok(())
    }

    async fn connect(ep: &ServerEndpoint, cfg: &ImapAccountConfig) -> Result<Smtp, SmtpError> {
        if ep.security == Security::Plaintext && !is_loopback_host(&ep.host) {
            return Err(SmtpError::Transport {
                stage: Stage::Connect,
                message: "plaintext SMTP is only allowed to a loopback host".into(),
                tls: true,
            });
        }
        let t = cfg.timeouts;
        let tcp = tcp_connect(&ep.host, ep.port, t.connect, cfg.tls.allow_private_hosts)
            .await
            .map_err(|e| transport(Stage::Connect, e))?;
        let stream = match ep.security {
            Security::Implicit => {
                upgrade(tcp, &ep.host, &cfg.tls, t.connect).await.map_err(|e| transport(Stage::Connect, e))?
            }
            _ => MailStream::Plain(tcp),
        };
        let mut s = Smtp { io: BufReader::new(stream), timeout: t.command, ehlo: Vec::new() };
        let greet = s.read_reply(Stage::Connect).await?;
        if greet.code != 220 {
            return Err(SmtpError::Reply { stage: Stage::Connect, code: greet.code, text: greet.text() });
        }
        let name = ehlo_name(&cfg.email);
        s.ehlo(&name).await?;
        if ep.security == Security::StartTls {
            if !s.has("STARTTLS") {
                return Err(SmtpError::Transport {
                    stage: Stage::Connect,
                    message: "server does not offer STARTTLS".into(),
                    tls: true,
                });
            }
            s.cmd(Stage::Connect, "STARTTLS").await.map_err(|e| match e {
                SmtpError::Reply { code, text, .. } => SmtpError::Transport {
                    stage: Stage::Connect,
                    message: format!("STARTTLS refused: {code} {text}"),
                    tls: true,
                },
                other => other,
            })?;
            if !s.io.buffer().is_empty() {
                return Err(SmtpError::Transport {
                    stage: Stage::Connect,
                    message: "data after STARTTLS".into(),
                    tls: true,
                });
            }
            let tcp = match s.io.into_inner() {
                MailStream::Plain(t) => t,
                MailStream::Tls(_) => unreachable!("STARTTLS on a TLS stream"),
            };
            let tls = upgrade(tcp, &ep.host, &cfg.tls, t.connect).await.map_err(|e| transport(Stage::Connect, e))?;
            s = Smtp { io: BufReader::new(tls), timeout: t.command, ehlo: Vec::new() };
            s.ehlo(&name).await?;
        }
        Ok(s)
    }

    async fn auth(&mut self, creds: &Credentials) -> Result<(), SmtpError> {
        let mechs = self.auth_mechs();
        let has = |m: &str| mechs.iter().any(|x| x == m);
        match creds {
            Credentials::OAuth2 { username, tokens } => {
                let tok = tokens.access_token().await.map_err(|e| SmtpError::Transport {
                    stage: Stage::Auth,
                    message: format!("token: {e}"),
                    tls: false,
                })?;
                let blob = B64.encode(format!("user={username}\x01auth=Bearer {}\x01\x01", tok.token));
                self.send_line(Stage::Auth, &format!("AUTH XOAUTH2 {blob}")).await?;
                let mut r = self.read_reply(Stage::Auth).await?;
                if r.code == 334 {
                    // XOAUTH2 error report: answer with an empty line to get the final reply.
                    self.send_line(Stage::Auth, "").await?;
                    r = self.read_reply(Stage::Auth).await?;
                }
                if r.code == 235 {
                    Ok(())
                } else {
                    if matches!(r.code, 535 | 534) {
                        tokens.invalidate().await;
                    }
                    Err(SmtpError::Reply { stage: Stage::Auth, code: r.code, text: r.text() })
                }
            }
            Credentials::Password { username, password } => {
                if has("PLAIN") || !has("LOGIN") {
                    if mechs.is_empty() {
                        return Err(SmtpError::Local("server offers no AUTH mechanism".into()));
                    }
                    let blob = B64.encode(format!("\0{username}\0{password}"));
                    self.cmd(Stage::Auth, &format!("AUTH PLAIN {blob}")).await.map(|_| ())
                } else {
                    let r = self.cmd(Stage::Auth, "AUTH LOGIN").await?;
                    if r.code != 334 {
                        return Err(SmtpError::Reply { stage: Stage::Auth, code: r.code, text: r.text() });
                    }
                    let r = self.cmd(Stage::Auth, &B64.encode(username)).await?;
                    if r.code != 334 {
                        return Err(SmtpError::Reply { stage: Stage::Auth, code: r.code, text: r.text() });
                    }
                    let r = self.cmd(Stage::Auth, &B64.encode(password)).await?;
                    if r.code == 235 {
                        Ok(())
                    } else {
                        Err(SmtpError::Reply { stage: Stage::Auth, code: r.code, text: r.text() })
                    }
                }
            }
        }
    }

    async fn quit(mut self) {
        let _ = self.send_line(Stage::Connect, "QUIT").await;
        let _ = self.read_reply(Stage::Connect).await;
    }
}

fn ehlo_name(email: &str) -> String {
    // The account's domain; never the worker's host name (it would leak infrastructure).
    let d = email.rsplit_once('@').map(|(_, d)| d.trim()).unwrap_or_default();
    if !d.is_empty() && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
        d.to_ascii_lowercase()
    } else {
        "[127.0.0.1]".to_owned()
    }
}

fn valid_path(addr: &str) -> bool {
    !addr.is_empty()
        && addr.len() <= 320
        && addr.contains('@')
        && !addr.bytes().any(|b| b.is_ascii_control() || b == b'<' || b == b'>' || b == b' ')
}

/// Normalises line endings to CRLF and applies dot-stuffing (RFC 5321 §4.5.2).
pub fn dot_stuff(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 50 + 5);
    let mut at_line_start = true;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if at_line_start && b == b'.' {
            out.push(b'.');
        }
        match b {
            b'\r' if data.get(i + 1) == Some(&b'\n') => {
                out.extend_from_slice(b"\r\n");
                i += 2;
                at_line_start = true;
                continue;
            }
            b'\r' | b'\n' => {
                out.extend_from_slice(b"\r\n");
                at_line_start = true;
            }
            _ => {
                out.push(b);
                at_line_start = false;
            }
        }
        i += 1;
    }
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// Checks the SMTP login without sending anything (§6.7: "an SMTP AUTH (no DATA)").
pub async fn check_login(cfg: &ImapAccountConfig) -> Result<(), SmtpError> {
    let mut s = Smtp::connect(&cfg.smtp, cfg).await?;
    s.auth(&cfg.credentials).await?;
    s.quit().await;
    Ok(())
}

/// Submits one message. `data` must already be free of `Bcc:` (see [`strip_bcc`]).
pub async fn send(cfg: &ImapAccountConfig, from: &str, rcpts: &[String], data: &[u8]) -> Result<(), SmtpError> {
    if rcpts.is_empty() {
        return Err(SmtpError::Local("no recipients".into()));
    }
    if !valid_path(from) || rcpts.iter().any(|r| !valid_path(r)) {
        return Err(SmtpError::Local("invalid address".into()));
    }
    let mut s = Smtp::connect(&cfg.smtp, cfg).await?;
    s.auth(&cfg.credentials).await?;
    let size = data.len() as u64;
    if let Some(limit) = s.size_limit()
        && size > limit
    {
        s.quit().await;
        return Err(SmtpError::Reply {
            stage: Stage::Envelope,
            code: 552,
            text: "message exceeds the server's SIZE limit".into(),
        });
    }
    let utf8 = !from.is_ascii() || rcpts.iter().any(|r| !r.is_ascii());
    if utf8 && !s.has("SMTPUTF8") {
        s.quit().await;
        return Err(SmtpError::Local("server cannot deliver to internationalised addresses".into()));
    }
    let mut mail = format!("MAIL FROM:<{from}>");
    if s.has("SIZE") {
        mail.push_str(&format!(" SIZE={size}"));
    }
    if !data.is_ascii() && s.has("8BITMIME") {
        mail.push_str(" BODY=8BITMIME");
    }
    if utf8 {
        mail.push_str(" SMTPUTF8");
    }
    s.cmd(Stage::Envelope, &mail).await?;
    for r in rcpts {
        s.cmd(Stage::Envelope, &format!("RCPT TO:<{r}>")).await?;
    }
    let r = s.cmd(Stage::Envelope, "DATA").await?;
    if r.code != 354 {
        return Err(SmtpError::Reply { stage: Stage::Envelope, code: r.code, text: r.text() });
    }
    // From here on a broken connection is ambiguous (Stage::Upload).
    let body = dot_stuff(data);
    let write = async {
        let io = s.io.get_mut();
        io.write_all(&body).await?;
        io.flush().await
    };
    match tokio::time::timeout(cfg.timeouts.smtp_data_end, write).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(SmtpError::Transport { stage: Stage::Upload, message: e.kind().to_string(), tls: false });
        }
        Err(_) => {
            return Err(SmtpError::Transport { stage: Stage::Upload, message: "upload timed out".into(), tls: false });
        }
    }
    let r = s.read_reply_within(Stage::Upload, cfg.timeouts.smtp_data_end).await?;
    if r.code != 250 {
        // A definite rejection after the upload: the server did not take the message.
        return Err(SmtpError::Reply { stage: Stage::Upload, code: r.code, text: r.text() });
    }
    s.quit().await;
    Ok(())
}

/// Removes `Bcc:` header fields (and their folded continuation lines) from the header section.
pub fn strip_bcc(raw: &[u8]) -> Vec<u8> {
    let end = find_header_end(raw);
    let (head, body) = raw.split_at(end);
    let mut out = Vec::with_capacity(raw.len());
    let mut skipping = false;
    for line in head.split_inclusive(|b| *b == b'\n') {
        let folded = line.first().is_some_and(|b| *b == b' ' || *b == b'\t');
        if folded {
            if !skipping {
                out.extend_from_slice(line);
            }
            continue;
        }
        skipping = line.len() >= 4 && line[..4].eq_ignore_ascii_case(b"bcc:");
        if !skipping {
            out.extend_from_slice(line);
        }
    }
    out.extend_from_slice(body);
    out
}

fn find_header_end(raw: &[u8]) -> usize {
    if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
        return p + 2;
    }
    if let Some(p) = raw.windows(2).position(|w| w == b"\n\n") {
        return p + 1;
    }
    raw.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuffing() {
        assert_eq!(dot_stuff(b"a\n.b\r\n..c"), b"a\r\n..b\r\n...c\r\n.\r\n".to_vec());
        assert_eq!(dot_stuff(b".\r\n"), b"..\r\n.\r\n".to_vec());
        assert_eq!(dot_stuff(b"x\r\n"), b"x\r\n.\r\n".to_vec());
    }

    #[test]
    fn bcc_is_stripped_from_headers_only() {
        let raw = b"From: a@x\r\nBcc: secret@x,\r\n  other@x\r\nTo: b@x\r\n\r\nBcc: stays in body\r\n";
        let out = strip_bcc(raw);
        assert_eq!(out, b"From: a@x\r\nTo: b@x\r\n\r\nBcc: stays in body\r\n".to_vec());
    }

    #[test]
    fn classification() {
        let auth = SmtpError::Reply { stage: Stage::Auth, code: 535, text: "5.7.8 bad".into() };
        assert_eq!(auth.clone().into_provider(), ProviderError::Auth { reauth_required: true });
        assert!(auth.is_auth());
        let quota = SmtpError::Reply { stage: Stage::Upload, code: 552, text: "5.2.2 full".into() };
        assert!(matches!(quota.into_provider(), ProviderError::Quota(_)));
        let rcpt = SmtpError::Reply { stage: Stage::Envelope, code: 550, text: "5.1.1 no such user".into() };
        assert!(matches!(rcpt.into_provider(), ProviderError::Invalid(_)));
        let temp = SmtpError::Reply { stage: Stage::Envelope, code: 451, text: "4.3.0 try later".into() };
        let te = temp.into_provider();
        assert!(matches!(te, ProviderError::Transient(..)) && !is_ambiguous_send(&te));
        let rate = SmtpError::Reply { stage: Stage::Envelope, code: 450, text: "4.7.0 rate limit".into() };
        assert!(matches!(rate.into_provider(), ProviderError::RateLimited { .. }));
        let lost = SmtpError::Transport { stage: Stage::Upload, message: "reset".into(), tls: false };
        assert!(is_ambiguous_send(&lost.into_provider()));
        let before = SmtpError::Transport { stage: Stage::Envelope, message: "reset".into(), tls: false };
        assert!(!is_ambiguous_send(&before.into_provider()));
        assert_eq!(ehlo_name("me@Example.com"), "example.com");
        assert_eq!(ehlo_name("weird"), "[127.0.0.1]");
    }
}
