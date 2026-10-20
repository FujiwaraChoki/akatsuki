//! One authenticated IMAP connection.
//!
//! `async-imap` provides the connection, the response framing (literals included), LOGIN,
//! AUTHENTICATE and IDLE. Every other command goes through [`ImapSession::exec`], which writes the
//! command itself (with literals, `LITERAL+` when offered) and collects every untagged response
//! until its tagged completion, so extension data that `async-imap`'s typed helpers drop
//! (`VANISHED`, `COPYUID`, `APPENDUID`, `HIGHESTMODSEQ`, `X-GM-THRID`, QRESYNC `SELECT`) is kept.

use std::time::Duration;

use async_imap::{Authenticator, Client, Connection, Session};
use imap_proto::{
    AttributeValue, Capability, MailboxDatum, MessageSection, NameAttribute, Response, ResponseCode, SectionPath,
    Status, StatusAttribute,
};
use tokio::io::AsyncWriteExt;

use super::config::{Credentials, ImapAccountConfig, Security, is_loopback_host};
use super::message::{BodyInfo, analyze_structure};
use super::proto::{self, Cmd, Part};
use super::tls::{MailStream, TransportError, tcp_connect, upgrade};
use super::utf7;
use crate::{ImapCaps, LimitScope, ProviderError};

type Inner = Session<MailStream>;

/// A response code carried by an `OK`/`NO` response, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Code {
    AppendUid { validity: u32, uids: Vec<u32> },
    CopyUid { validity: u32, src: Vec<u32>, dst: Vec<u32> },
    UidValidity(u32),
    UidNext(u32),
    HighestModSeq(u64),
    PermanentFlags(Vec<String>),
    ReadOnly,
    ReadWrite,
    TryCreate,
    Capabilities(Vec<String>),
    Other,
}

impl Code {
    fn from_proto(c: &ResponseCode<'_>) -> Self {
        match c {
            ResponseCode::AppendUid(v, set) => Code::AppendUid { validity: *v, uids: proto::expand_uid_set(set) },
            ResponseCode::CopyUid(v, a, b) => {
                Code::CopyUid { validity: *v, src: proto::expand_uid_set(a), dst: proto::expand_uid_set(b) }
            }
            ResponseCode::UidValidity(v) => Code::UidValidity(*v),
            ResponseCode::UidNext(v) => Code::UidNext(*v),
            ResponseCode::HighestModSeq(v) => Code::HighestModSeq(*v),
            ResponseCode::PermanentFlags(f) => Code::PermanentFlags(f.iter().map(|s| s.to_string()).collect()),
            ResponseCode::ReadOnly => Code::ReadOnly,
            ResponseCode::ReadWrite => Code::ReadWrite,
            ResponseCode::TryCreate => Code::TryCreate,
            ResponseCode::Capabilities(c) => Code::Capabilities(caps_to_strings(c)),
            _ => Code::Other,
        }
    }
}

/// One `BODY[...]` section from a FETCH response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Section {
    /// `""` for the whole message, `HEADER`, `TEXT`, `1.2`, `1.2.MIME`.
    pub key: String,
    /// The `<origin>` of a partial fetch.
    pub origin: Option<u32>,
    /// Bytes (still transfer-encoded for body parts).
    pub data: Vec<u8>,
}

/// One FETCH response, owned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FetchItem {
    pub seq: u32,
    pub uid: Option<u32>,
    pub flags: Option<Vec<String>>,
    pub modseq: Option<u64>,
    pub internal_date: Option<i64>,
    pub size: Option<u32>,
    pub structure: Option<BodyInfo>,
    pub sections: Vec<Section>,
    pub gm_thrid: Option<u64>,
}

impl FetchItem {
    /// The data of a section by key.
    pub fn section(&self, key: &str) -> Option<&[u8]> {
        self.sections.iter().find(|s| s.key.eq_ignore_ascii_case(key)).map(|s| s.data.as_slice())
    }
}

/// One `LIST` entry, owned. `name` is decoded UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ListEntry {
    pub name: String,
    pub delimiter: Option<String>,
    /// Lowercased attributes (`\noselect`, `\sent`, …).
    pub attrs: Vec<String>,
}

/// `STATUS` values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MailboxStatus {
    pub messages: Option<u32>,
    pub uid_next: Option<u32>,
    pub uid_validity: Option<u32>,
    pub highest_modseq: Option<u64>,
    pub unseen: Option<u32>,
}

/// An untagged response, reduced to what the adapter uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resp {
    Caps(Vec<String>),
    Exists(u32),
    Expunge(u32),
    Vanished {
        earlier: bool,
        uids: Vec<u32>,
    },
    Fetch(FetchItem),
    List(ListEntry),
    Search(Vec<u32>),
    Status {
        mailbox: String,
        status: MailboxStatus,
    },
    Flags(Vec<String>),
    /// Untagged `OK [code]`.
    Code(Code),
    /// Untagged `OK`/`NO` text that carries an unparsed bracketed code (`[NOMODSEQ]`).
    Info(String),
    Bye(String),
    Other,
}

/// The collected result of one command.
#[derive(Clone, Debug, Default)]
pub(crate) struct Exec {
    pub resps: Vec<Resp>,
    /// Code of the tagged `OK`.
    pub code: Option<Code>,
}

impl Exec {
    /// Every code, untagged and tagged.
    pub fn codes(&self) -> impl Iterator<Item = &Code> {
        self.resps.iter().filter_map(|r| if let Resp::Code(c) = r { Some(c) } else { None }).chain(self.code.iter())
    }

    /// FETCH responses.
    pub fn fetches(self) -> Vec<FetchItem> {
        self.resps.into_iter().filter_map(|r| if let Resp::Fetch(f) = r { Some(f) } else { None }).collect()
    }
}

/// Errors on an established connection.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ImapError {
    /// The connection broke or timed out: it must be dropped.
    #[error("connection: {0}")]
    Connection(String),
    /// Tagged `NO`.
    #[error("no: {info}")]
    No { info: String, code: Option<Code> },
    /// Tagged `BAD`.
    #[error("bad: {0}")]
    Bad(String),
}

impl ImapError {
    /// The connection can no longer be used.
    pub fn is_fatal(&self) -> bool {
        matches!(self, ImapError::Connection(_))
    }

    /// The §5.5 classification.
    pub fn into_provider(self) -> ProviderError {
        match self {
            ImapError::Connection(m) => ProviderError::Transient(format!("imap {m}"), None),
            ImapError::Bad(m) => ProviderError::Invalid(format!("imap BAD {}", short(&m))),
            ImapError::No { info, code } => classify_no(&info, code.as_ref()),
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(160).collect()
}

/// Classifies a tagged `NO` by its RFC 5530 response code.
pub(crate) fn classify_no(info: &str, code: Option<&Code>) -> ProviderError {
    if matches!(code, Some(Code::TryCreate)) {
        return ProviderError::Conflict("mailbox does not exist".into());
    }
    let up = info.to_ascii_uppercase();
    let has = |c: &str| up.contains(&format!("[{c}]")) || up.starts_with(c);
    if has("AUTHENTICATIONFAILED") || has("AUTHORIZATIONFAILED") || has("EXPIRED") {
        ProviderError::Auth { reauth_required: true }
    } else if has("UNAVAILABLE") || has("INUSE") || has("SERVERBUG") {
        ProviderError::Transient(format!("imap NO {}", short(info)), None)
    } else if has("LIMIT") {
        ProviderError::RateLimited { retry_after: None, scope: LimitScope::Account }
    } else if has("OVERQUOTA") {
        ProviderError::Quota("mailbox is full".into())
    } else if has("NONEXISTENT") {
        ProviderError::NotFound
    } else if has("ALREADYEXISTS") {
        ProviderError::Conflict("mailbox already exists".into())
    } else if has("CANNOT") || has("NOPERM") || has("PRIVACYREQUIRED") || has("CONTACTADMIN") {
        ProviderError::Invalid(format!("imap NO {}", short(info)))
    } else {
        ProviderError::Transient(format!("imap NO {}", short(info)), None)
    }
}

/// Why connecting failed (`POST /v1/connect/imap` codes, §7.2).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectError {
    /// No TCP connection.
    #[error("unreachable: {0}")]
    Unreachable(String),
    /// TLS or STARTTLS failed, or plaintext was refused.
    #[error("tls: {0}")]
    Tls(String),
    /// The server rejected the login.
    #[error("authentication failed")]
    Auth,
    /// Anything else (greeting, BYE, protocol error, `[UNAVAILABLE]`).
    #[error("transient: {0}")]
    Transient(String),
}

impl ConnectError {
    /// The §15 code for the connect endpoint (`smtp` picks the SMTP auth code).
    pub fn code(&self, smtp: bool) -> akatsuki_core::wire::ErrorCode {
        use akatsuki_core::wire::ErrorCode as C;
        match self {
            ConnectError::Unreachable(_) | ConnectError::Transient(_) => C::Unreachable,
            ConnectError::Tls(_) => C::TlsFailed,
            ConnectError::Auth if smtp => C::SmtpAuthFailed,
            ConnectError::Auth => C::ImapAuthFailed,
        }
    }

    /// As a sync-time provider error.
    pub fn into_provider(self) -> ProviderError {
        match self {
            ConnectError::Auth => ProviderError::Auth { reauth_required: true },
            other => ProviderError::Transient(other.to_string(), None),
        }
    }
}

impl From<TransportError> for ConnectError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Unreachable(m) => ConnectError::Unreachable(m),
            TransportError::Tls(m) => ConnectError::Tls(m),
            TransportError::Io(m) => ConnectError::Transient(m),
        }
    }
}

/// What the server advertises.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ServerCaps {
    /// Uppercased capability atoms (`AUTH=PLAIN` included).
    pub raw: Vec<String>,
    pub imap: ImapCaps,
    pub literal_plus: bool,
    pub login_disabled: bool,
    pub starttls: bool,
    pub enable: bool,
}

impl ServerCaps {
    pub fn from_list(list: &[String]) -> Self {
        let raw: Vec<String> = list.iter().map(|s| s.to_ascii_uppercase()).collect();
        let has = |c: &str| raw.iter().any(|r| r == c);
        ServerCaps {
            imap: ImapCaps::from_capabilities(&raw),
            literal_plus: has("LITERAL+"),
            login_disabled: has("LOGINDISABLED"),
            starttls: has("STARTTLS"),
            enable: has("ENABLE"),
            raw,
        }
    }

    pub fn has(&self, cap: &str) -> bool {
        self.raw.iter().any(|r| r.eq_ignore_ascii_case(cap))
    }

    pub fn has_auth(&self, mech: &str) -> bool {
        self.has(&format!("AUTH={mech}"))
    }
}

fn caps_to_strings(c: &[Capability<'_>]) -> Vec<String> {
    c.iter()
        .map(|c| match c {
            Capability::Imap4rev1 => "IMAP4REV1".to_owned(),
            Capability::Auth(a) => format!("AUTH={}", a.to_ascii_uppercase()),
            Capability::Atom(a) => a.to_ascii_uppercase(),
        })
        .collect()
}

/// What the currently selected mailbox looks like.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selected {
    pub name: String,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub highest_modseq: Option<u64>,
    pub exists: u32,
    pub flags: Vec<String>,
    pub permanent_flags: Vec<String>,
    pub read_only: bool,
    /// QRESYNC: `VANISHED (EARLIER)` UIDs.
    pub vanished: Vec<u32>,
    /// QRESYNC: changed messages (UID, FLAGS, MODSEQ).
    pub changed: Vec<FetchItem>,
}

impl Selected {
    /// Keywords may be stored (`PERMANENTFLAGS` contains `\*`, §9.5).
    pub fn keywords_allowed(&self) -> bool {
        self.permanent_flags.iter().any(|f| f == "\\*")
    }
}

/// How to select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectMode {
    Plain,
    Condstore,
    Qresync { uid_validity: u32, modseq: u64 },
}

struct OneShot(Option<Vec<u8>>);

impl Authenticator for OneShot {
    type Response = Vec<u8>;
    fn process(&mut self, _challenge: &[u8]) -> Vec<u8> {
        // The first challenge gets the credentials; a second one (an error report in XOAUTH2)
        // gets an empty response so the server sends its tagged NO.
        self.0.take().unwrap_or_default()
    }
}

async fn write_bytes(conn: &mut Connection<MailStream>, bytes: &[u8]) -> Result<(), ImapError> {
    let s = conn.get_mut();
    s.write_all(bytes).await.map_err(|e| ImapError::Connection(e.kind().to_string()))?;
    s.flush().await.map_err(|e| ImapError::Connection(e.kind().to_string()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Ok,
    No,
    Bad,
    Bye,
    Other,
}

impl St {
    fn from_proto(s: &Status) -> Self {
        match s {
            Status::Ok => St::Ok,
            Status::No => St::No,
            Status::Bad => St::Bad,
            Status::Bye => St::Bye,
            _ => St::Other,
        }
    }
}

enum Read {
    Untagged(Resp),
    Continue,
    Done { tag: String, status: St, info: String, code: Option<Code> },
}

async fn read_one(conn: &mut Connection<MailStream>, timeout: Duration) -> Result<Read, ImapError> {
    let res = match tokio::time::timeout(timeout, conn.read_response()).await {
        Err(_) => return Err(ImapError::Connection("read timed out".into())),
        Ok(Err(e)) => return Err(ImapError::Connection(e.kind().to_string())),
        Ok(Ok(None)) => return Err(ImapError::Connection("connection closed".into())),
        Ok(Ok(Some(r))) => r,
    };
    Ok(match res.parsed() {
        Response::Continue(_) => Read::Continue,
        Response::Done { tag, status, outcome } => Read::Done {
            tag: tag.0.clone(),
            status: St::from_proto(status),
            info: outcome.information.as_deref().unwrap_or_default().to_owned(),
            code: outcome.code.as_ref().map(Code::from_proto),
        },
        other => Read::Untagged(convert(other)),
    })
}

fn convert(r: &Response<'_>) -> Resp {
    match r {
        Response::Capabilities(c) => Resp::Caps(caps_to_strings(c)),
        Response::Expunge(n) => Resp::Expunge(*n),
        Response::Vanished { earlier, uids } => {
            let mut out = Vec::new();
            for r in uids {
                let (a, b) = (*r.start(), *r.end());
                if b >= a && b - a <= 10_000_000 {
                    out.extend(a..=b);
                }
            }
            Resp::Vanished { earlier: *earlier, uids: out }
        }
        Response::Fetch(seq, attrs) => Resp::Fetch(convert_fetch(*seq, attrs)),
        Response::MailboxData(d) => match d {
            MailboxDatum::Exists(n) => Resp::Exists(*n),
            MailboxDatum::Flags(f) => Resp::Flags(f.iter().map(|s| s.to_string()).collect()),
            MailboxDatum::List(l) => Resp::List(ListEntry {
                name: utf7::decode(&l.name),
                delimiter: l.delimiter.as_ref().map(|d| d.to_string()),
                attrs: l.name_attributes.iter().map(name_attr).collect(),
            }),
            MailboxDatum::Search(v) | MailboxDatum::Sort(v) => Resp::Search(v.clone()),
            MailboxDatum::Status { mailbox, status } => {
                let mut s = MailboxStatus::default();
                for a in status {
                    match a {
                        StatusAttribute::Messages(n) => s.messages = Some(*n),
                        StatusAttribute::UidNext(n) => s.uid_next = Some(*n),
                        StatusAttribute::UidValidity(n) => s.uid_validity = Some(*n),
                        StatusAttribute::HighestModSeq(n) => s.highest_modseq = Some(*n),
                        StatusAttribute::Unseen(n) => s.unseen = Some(*n),
                        _ => {}
                    }
                }
                Resp::Status { mailbox: utf7::decode(mailbox), status: s }
            }
            _ => Resp::Other,
        },
        Response::Data { status, outcome } => {
            if matches!(status, Status::Bye) {
                return Resp::Bye(outcome.information.as_deref().unwrap_or_default().to_owned());
            }
            match &outcome.code {
                Some(c) => Resp::Code(Code::from_proto(c)),
                None => match outcome.information.as_deref() {
                    Some(i) if i.starts_with('[') => Resp::Info(i.to_owned()),
                    _ => Resp::Other,
                },
            }
        }
        _ => Resp::Other,
    }
}

fn name_attr(a: &NameAttribute<'_>) -> String {
    match a {
        NameAttribute::NoInferiors => "\\noinferiors".into(),
        NameAttribute::NoSelect => "\\noselect".into(),
        NameAttribute::Marked => "\\marked".into(),
        NameAttribute::Unmarked => "\\unmarked".into(),
        NameAttribute::All => "\\all".into(),
        NameAttribute::Archive => "\\archive".into(),
        NameAttribute::Drafts => "\\drafts".into(),
        NameAttribute::Flagged => "\\flagged".into(),
        NameAttribute::Junk => "\\junk".into(),
        NameAttribute::Sent => "\\sent".into(),
        NameAttribute::Trash => "\\trash".into(),
        NameAttribute::Extension(e) => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn section_key(s: Option<&SectionPath>) -> String {
    let msg = |m: &MessageSection| match m {
        MessageSection::Header => "HEADER",
        MessageSection::Mime => "MIME",
        MessageSection::Text => "TEXT",
    };
    match s {
        None => String::new(),
        Some(SectionPath::Full(m)) => msg(m).to_owned(),
        Some(SectionPath::Part(path, sub)) => {
            let p = path.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
            match sub {
                None => p,
                Some(m) => format!("{p}.{}", msg(m)),
            }
        }
    }
}

fn convert_fetch(seq: u32, attrs: &[AttributeValue<'_>]) -> FetchItem {
    let mut f = FetchItem { seq, ..Default::default() };
    for a in attrs {
        match a {
            AttributeValue::Uid(u) => f.uid = Some(*u),
            AttributeValue::Flags(fl) => f.flags = Some(fl.iter().map(|s| s.to_string()).collect()),
            AttributeValue::ModSeq(m) => f.modseq = Some(*m),
            AttributeValue::InternalDate(d) => f.internal_date = proto::parse_internal_date(d),
            AttributeValue::Rfc822Size(n) => f.size = Some(*n),
            AttributeValue::BodyStructure(b) => f.structure = Some(analyze_structure(b)),
            AttributeValue::GmailThrId(t) => f.gm_thrid = Some(*t),
            AttributeValue::BodySection { section, index, data } => f.sections.push(Section {
                key: section_key(section.as_ref()),
                origin: *index,
                data: data.as_deref().unwrap_or_default().to_vec(),
            }),
            AttributeValue::Rfc822(data) => f.sections.push(Section {
                key: String::new(),
                origin: None,
                data: data.as_deref().unwrap_or_default().to_vec(),
            }),
            AttributeValue::Rfc822Header(data) => f.sections.push(Section {
                key: "HEADER".into(),
                origin: None,
                data: data.as_deref().unwrap_or_default().to_vec(),
            }),
            _ => {}
        }
    }
    f
}

/// Runs one command on a raw connection (used before and after authentication).
async fn exec_on(
    conn: &mut Connection<MailStream>,
    tag: &str,
    cmd: &Cmd,
    literal_plus: bool,
    timeout: Duration,
) -> Result<Exec, ImapError> {
    let mut out = Exec::default();
    let mut line: Vec<u8> = format!("{tag} ").into_bytes();
    for part in &cmd.parts {
        match part {
            Part::Text(t) => line.extend_from_slice(t.as_bytes()),
            Part::Literal(bytes) if literal_plus => {
                line.extend_from_slice(format!("{{{}+}}\r\n", bytes.len()).as_bytes());
                line.extend_from_slice(bytes);
            }
            Part::Literal(bytes) => {
                line.extend_from_slice(format!("{{{}}}\r\n", bytes.len()).as_bytes());
                write_bytes(conn, &line).await?;
                line.clear();
                loop {
                    match read_one(conn, timeout).await? {
                        Read::Continue => break,
                        Read::Untagged(r) => out.resps.push(r),
                        Read::Done { tag: t, status, info, code } if t == tag => {
                            return Err(done_error(status, info, code));
                        }
                        Read::Done { .. } => {}
                    }
                }
                line.extend_from_slice(bytes);
            }
        }
    }
    line.extend_from_slice(b"\r\n");
    write_bytes(conn, &line).await?;
    loop {
        match read_one(conn, timeout).await? {
            Read::Untagged(Resp::Bye(info)) => {
                // A BYE before the tagged reply means the server is closing the connection.
                out.resps.push(Resp::Bye(info));
            }
            Read::Untagged(r) => out.resps.push(r),
            Read::Continue => {
                return Err(ImapError::Connection("unexpected continuation".into()));
            }
            Read::Done { tag: t, status, info, code } if t == tag => {
                return match status {
                    St::Ok => {
                        out.code = code;
                        Ok(out)
                    }
                    other => Err(done_error(other, info, code)),
                };
            }
            Read::Done { .. } => {}
        }
    }
}

fn done_error(status: St, info: String, code: Option<Code>) -> ImapError {
    match status {
        St::No => ImapError::No { info, code },
        St::Bad => ImapError::Bad(info),
        St::Bye => ImapError::Connection(format!("BYE {}", short(&info))),
        _ => ImapError::Connection(format!("unexpected status {status:?}")),
    }
}

fn caps_from(exec: &Exec) -> Option<Vec<String>> {
    exec.resps
        .iter()
        .find_map(|r| if let Resp::Caps(c) = r { Some(c.clone()) } else { None })
        .or_else(|| exec.codes().find_map(|c| if let Code::Capabilities(c) = c { Some(c.clone()) } else { None }))
}

/// An authenticated connection.
pub(crate) struct ImapSession {
    inner: Inner,
    next_tag: u32,
    pub caps: ServerCaps,
    pub selected: Option<Selected>,
    pub timeout: Duration,
}

impl std::fmt::Debug for ImapSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapSession")
            .field("selected", &self.selected.as_ref().map(|s| &s.name))
            .finish_non_exhaustive()
    }
}

impl ImapSession {
    /// Connects, secures and authenticates (§6.7 test login uses this too).
    pub async fn connect(cfg: &ImapAccountConfig) -> Result<ImapSession, ConnectError> {
        let ep = &cfg.imap;
        let t = cfg.timeouts;
        if ep.security == Security::Plaintext && !is_loopback_host(&ep.host) {
            return Err(ConnectError::Tls("plaintext IMAP is only allowed to a loopback host".into()));
        }
        let tcp = tcp_connect(&ep.host, ep.port, t.connect, cfg.tls.allow_private_hosts).await?;
        let stream = match ep.security {
            Security::Implicit => upgrade(tcp, &ep.host, &cfg.tls, t.connect).await?,
            Security::StartTls | Security::Plaintext => MailStream::Plain(tcp),
        };
        let mut client = Client::new(stream);
        let mut tag = 0u32;
        let mut next = || {
            tag += 1;
            format!("k{tag}")
        };
        // Greeting.
        match read_one(&mut client, t.command).await.map_err(|e| ConnectError::Transient(e.to_string()))? {
            Read::Untagged(Resp::Bye(_)) => return Err(ConnectError::Transient("server said BYE".into())),
            Read::Untagged(_) => {}
            _ => return Err(ConnectError::Transient("bad greeting".into())),
        }
        if ep.security == Security::StartTls {
            exec_on(&mut client, &next(), &Cmd::new("STARTTLS"), false, t.command).await.map_err(|e| {
                if e.is_fatal() {
                    ConnectError::Transient(e.to_string())
                } else {
                    ConnectError::Tls("STARTTLS refused".into())
                }
            })?;
            let tcp = match client.into_inner() {
                MailStream::Plain(tcp) => tcp,
                MailStream::Tls(_) => return Err(ConnectError::Tls("already secured".into())),
            };
            client = Client::new(upgrade(tcp, &ep.host, &cfg.tls, t.connect).await?);
        }
        let caps_exec = exec_on(&mut client, &next(), &Cmd::new("CAPABILITY"), false, t.command)
            .await
            .map_err(|e| ConnectError::Transient(e.to_string()))?;
        let pre = ServerCaps::from_list(&caps_from(&caps_exec).unwrap_or_default());

        let inner = match &cfg.credentials {
            Credentials::Password { username, password } => {
                if pre.has_auth("PLAIN") || pre.login_disabled {
                    let blob = format!("\0{username}\0{password}").into_bytes();
                    client.authenticate("PLAIN", OneShot(Some(blob))).await.map_err(|(e, _)| auth_error(e))?
                } else {
                    client.login(username, password).await.map_err(|(e, _)| auth_error(e))?
                }
            }
            Credentials::OAuth2 { username, tokens } => {
                let token = tokens.access_token().await.map_err(|e| match e {
                    ProviderError::Auth { .. } => ConnectError::Auth,
                    other => ConnectError::Transient(other.to_string()),
                })?;
                let blob = format!("user={username}\x01auth=Bearer {}\x01\x01", token.token).into_bytes();
                match client.authenticate("XOAUTH2", OneShot(Some(blob))).await {
                    Ok(s) => s,
                    Err((e, _)) => {
                        let err = auth_error(e);
                        if err == ConnectError::Auth {
                            tokens.invalidate().await;
                        }
                        return Err(err);
                    }
                }
            }
        };
        let mut s = ImapSession { inner, next_tag: tag, caps: pre, selected: None, timeout: t.command };
        let exec = s.exec(&Cmd::new("CAPABILITY")).await.map_err(|e| ConnectError::Transient(e.to_string()))?;
        if let Some(c) = caps_from(&exec) {
            s.caps = ServerCaps::from_list(&c);
        }
        if s.caps.imap.qresync && s.caps.enable && s.exec(&Cmd::new("ENABLE QRESYNC")).await.is_err() {
            s.caps.imap.qresync = false;
        }
        Ok(s)
    }

    /// Runs one command.
    pub async fn exec(&mut self, cmd: &Cmd) -> Result<Exec, ImapError> {
        self.next_tag += 1;
        let tag = format!("k{}", self.next_tag);
        let lp = self.caps.literal_plus;
        let res = exec_on(&mut self.inner, &tag, cmd, lp, self.timeout).await;
        if let Err(e) = &res {
            tracing::debug!(command = %cmd.verb(), error = %e, "imap command failed");
        }
        if res.as_ref().is_ok_and(|e| e.resps.iter().any(|r| matches!(r, Resp::Bye(_)))) {
            return Err(ImapError::Connection("server closed the connection".into()));
        }
        res
    }

    /// `SELECT` (or `EXAMINE`) with CONDSTORE / QRESYNC parameters.
    pub async fn select(&mut self, name: &str, mode: SelectMode, read_only: bool) -> Result<&Selected, ImapError> {
        self.selected = None;
        let mut cmd = Cmd::new(if read_only { "EXAMINE" } else { "SELECT" }).mailbox(name).map_err(prov_to_bad)?;
        cmd = match mode {
            SelectMode::Plain => cmd,
            SelectMode::Condstore => cmd.raw("(CONDSTORE)"),
            SelectMode::Qresync { uid_validity, modseq } => cmd.raw(format!("(QRESYNC ({uid_validity} {modseq}))")),
        };
        let exec = self.exec(&cmd).await?;
        let mut sel = Selected { name: name.to_owned(), read_only, ..Default::default() };
        let mut nomodseq = false;
        for r in &exec.resps {
            match r {
                Resp::Exists(n) => sel.exists = *n,
                Resp::Flags(f) => sel.flags = f.clone(),
                Resp::Code(Code::UidValidity(v)) => sel.uid_validity = *v,
                Resp::Code(Code::UidNext(v)) => sel.uid_next = *v,
                Resp::Code(Code::HighestModSeq(v)) => sel.highest_modseq = Some(*v),
                Resp::Code(Code::PermanentFlags(f)) => sel.permanent_flags = f.clone(),
                Resp::Info(i) if i.to_ascii_uppercase().starts_with("[NOMODSEQ]") => nomodseq = true,
                Resp::Vanished { uids, .. } => sel.vanished.extend(uids),
                Resp::Fetch(f) => sel.changed.push(f.clone()),
                _ => {}
            }
        }
        if nomodseq {
            sel.highest_modseq = None;
        }
        if matches!(exec.code, Some(Code::ReadOnly)) {
            sel.read_only = true;
        }
        if sel.uid_next == 0 {
            // Servers that omit UIDNEXT: ask STATUS-free via a search of the last message.
            sel.uid_next = self.max_uid().await?.map_or(1, |u| u + 1);
        }
        self.selected = Some(sel);
        Ok(self.selected.as_ref().unwrap_or_else(|| unreachable!()))
    }

    async fn max_uid(&mut self) -> Result<Option<u32>, ImapError> {
        let exec = self.exec(&Cmd::new("UID SEARCH").raw("UID *")).await?;
        Ok(searched(&exec).into_iter().max())
    }

    /// `STATUS`.
    pub async fn status(&mut self, name: &str, condstore: bool) -> Result<MailboxStatus, ImapError> {
        let items =
            if condstore { "(MESSAGES UIDNEXT UIDVALIDITY HIGHESTMODSEQ)" } else { "(MESSAGES UIDNEXT UIDVALIDITY)" };
        let exec = self.exec(&Cmd::new("STATUS").mailbox(name).map_err(prov_to_bad)?.raw(items)).await?;
        Ok(exec
            .resps
            .into_iter()
            .find_map(|r| if let Resp::Status { status, .. } = r { Some(status) } else { None })
            .unwrap_or_default())
    }

    /// `LIST "" "*"`.
    pub async fn list(&mut self) -> Result<Vec<ListEntry>, ImapError> {
        let exec = self.exec(&Cmd::new("LIST \"\" \"*\"")).await?;
        Ok(exec.resps.into_iter().filter_map(|r| if let Resp::List(l) = r { Some(l) } else { None }).collect())
    }

    /// `UID SEARCH`.
    pub async fn uid_search(&mut self, criteria: Cmd) -> Result<Vec<u32>, ImapError> {
        let mut cmd = Cmd::new("UID SEARCH");
        cmd.parts.extend(prefix_space(criteria.parts));
        let exec = self.exec(&cmd).await?;
        let mut v = searched(&exec);
        v.sort_unstable();
        v.dedup();
        Ok(v)
    }

    /// `UID FETCH <set> <items> [<modifiers>]`, in chunks of 500 UIDs. Only responses for the
    /// requested UIDs are returned.
    pub async fn uid_fetch(
        &mut self,
        uids: &[u32],
        items: &str,
        modifiers: Option<&str>,
    ) -> Result<Vec<FetchItem>, ImapError> {
        let mut out = Vec::new();
        let mut sorted: Vec<u32> = uids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        for chunk in sorted.chunks(500) {
            let mut cmd = Cmd::new("UID FETCH").raw(proto::uid_set(chunk)).raw(items);
            if let Some(m) = modifiers {
                cmd = cmd.raw(m);
            }
            let exec = self.exec(&cmd).await?;
            out.extend(exec.fetches().into_iter().filter(|f| f.uid.is_some_and(|u| chunk.binary_search(&u).is_ok())));
        }
        Ok(out)
    }

    /// `UID FETCH 1:* <items> <modifiers>` (CHANGEDSINCE scans).
    pub async fn uid_fetch_all(&mut self, items: &str, modifiers: &str) -> Result<Exec, ImapError> {
        self.exec(&Cmd::new("UID FETCH 1:*").raw(items).raw(modifiers)).await
    }

    /// `UID STORE <set> <op> (<flags>)`.
    pub async fn uid_store(&mut self, uids: &[u32], op: &str, flags: &[String]) -> Result<(), ImapError> {
        if uids.is_empty() || flags.is_empty() {
            return Ok(());
        }
        let list = format!("({})", flags.join(" "));
        for chunk in uids.chunks(1000) {
            self.exec(&Cmd::new("UID STORE").raw(proto::uid_set(chunk)).raw(op).raw(&list)).await?;
        }
        Ok(())
    }

    /// `UID MOVE` (RFC 6851). Returns the `COPYUID` mapping if the server sent one.
    pub async fn uid_move(&mut self, uids: &[u32], dest: &str) -> Result<Option<(u32, Vec<(u32, u32)>)>, ImapError> {
        let exec =
            self.exec(&Cmd::new("UID MOVE").raw(proto::uid_set(uids)).mailbox(dest).map_err(prov_to_bad)?).await?;
        Ok(copyuid(&exec))
    }

    /// `UID COPY`. Returns the `COPYUID` mapping if the server sent one.
    pub async fn uid_copy(&mut self, uids: &[u32], dest: &str) -> Result<Option<(u32, Vec<(u32, u32)>)>, ImapError> {
        let exec =
            self.exec(&Cmd::new("UID COPY").raw(proto::uid_set(uids)).mailbox(dest).map_err(prov_to_bad)?).await?;
        Ok(copyuid(&exec))
    }

    /// The subset of `uids` that still exists in the selected mailbox.
    pub async fn existing(&mut self, uids: &[u32]) -> Result<Vec<u32>, ImapError> {
        let mut sorted: Vec<u32> = uids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut out = Vec::new();
        for chunk in sorted.chunks(1000) {
            let exec = self.exec(&Cmd::new("UID SEARCH").raw(format!("UID {}", proto::uid_set(chunk)))).await?;
            out.extend(searched(&exec).into_iter().filter(|u| chunk.binary_search(u).is_ok()));
        }
        out.sort_unstable();
        out.dedup();
        Ok(out)
    }

    /// Removes messages: `\Deleted` then `UID EXPUNGE` (UIDPLUS) or `EXPUNGE`.
    pub async fn delete_uids(&mut self, uids: &[u32]) -> Result<(), ImapError> {
        if uids.is_empty() {
            return Ok(());
        }
        self.uid_store(uids, "+FLAGS.SILENT", &["\\Deleted".to_owned()]).await?;
        if self.caps.imap.uidplus {
            self.exec(&Cmd::new("UID EXPUNGE").raw(proto::uid_set(uids))).await?;
        } else {
            // Without UIDPLUS, EXPUNGE also removes other \Deleted messages in this mailbox; that is
            // what every IMAP client does on such servers.
            self.exec(&Cmd::new("EXPUNGE")).await?;
        }
        Ok(())
    }

    /// `APPEND`. Returns `(uidvalidity, uid)` from `APPENDUID` when the server has UIDPLUS.
    pub async fn append(
        &mut self,
        mailbox: &str,
        flags: &[&str],
        bytes: Vec<u8>,
    ) -> Result<Option<(u32, u32)>, ImapError> {
        let mut cmd = Cmd::new("APPEND").mailbox(mailbox).map_err(prov_to_bad)?;
        if !flags.is_empty() {
            cmd = cmd.raw(format!("({})", flags.join(" ")));
        }
        let exec = self.exec(&cmd.literal(bytes)).await?;
        Ok(exec.codes().find_map(|c| match c {
            Code::AppendUid { validity, uids } => uids.first().map(|u| (*validity, *u)),
            _ => None,
        }))
    }

    /// Runs a simple mailbox command (`CREATE`, `DELETE`, `SUBSCRIBE`).
    pub async fn mailbox_cmd(&mut self, verb: &str, name: &str) -> Result<(), ImapError> {
        self.exec(&Cmd::new(verb).mailbox(name).map_err(prov_to_bad)?).await.map(|_| ())
    }

    /// `RENAME`.
    pub async fn rename(&mut self, from: &str, to: &str) -> Result<(), ImapError> {
        self.exec(&Cmd::new("RENAME").mailbox(from).map_err(prov_to_bad)?.mailbox(to).map_err(prov_to_bad)?)
            .await
            .map(|_| ())
    }

    /// `LOGOUT` (best effort).
    pub async fn logout(mut self) {
        let _ = self.exec(&Cmd::new("LOGOUT")).await;
    }

    /// Hands the connection to `async-imap`'s IDLE and takes it back. Returns `true` if the server
    /// reported a change before `wait` elapsed.
    pub async fn idle(self, wait: Duration) -> Result<(ImapSession, bool), ImapError> {
        use async_imap::extensions::idle::IdleResponse;
        let ImapSession { inner, next_tag, caps, selected, timeout } = self;
        let mut handle = inner.idle();
        handle.init().await.map_err(|e| ImapError::Connection(format!("IDLE: {e}")))?;
        let changed = {
            // The library's own timeout restarts on every keepalive, so the deadline is enforced
            // here through its stop source.
            let (fut, stop) = handle.wait_with_timeout(wait);
            tokio::pin!(fut);
            let res = tokio::select! {
                r = &mut fut => r,
                () = tokio::time::sleep(wait) => {
                    drop(stop);
                    fut.await
                }
            };
            match res.map_err(|e| ImapError::Connection(format!("IDLE: {e}")))? {
                IdleResponse::NewData(_) => true,
                IdleResponse::Timeout | IdleResponse::ManualInterrupt => false,
            }
        };
        let inner = tokio::time::timeout(timeout, handle.done())
            .await
            .map_err(|_| ImapError::Connection("DONE timed out".into()))?
            .map_err(|e| ImapError::Connection(format!("DONE: {e}")))?;
        // Responses that arrived during IDLE went to the library's channel; drain it.
        while inner.unsolicited_responses.try_recv().is_ok() {}
        Ok((ImapSession { inner, next_tag, caps, selected, timeout }, changed))
    }
}

fn prefix_space(parts: Vec<Part>) -> Vec<Part> {
    let mut out = Vec::with_capacity(parts.len() + 1);
    out.push(Part::Text(" ".into()));
    for p in parts {
        match (out.last_mut(), p) {
            (Some(Part::Text(t)), Part::Text(s)) => t.push_str(&s),
            (_, p) => out.push(p),
        }
    }
    out
}

fn searched(exec: &Exec) -> Vec<u32> {
    exec.resps.iter().filter_map(|r| if let Resp::Search(v) = r { Some(v.clone()) } else { None }).flatten().collect()
}

fn copyuid(exec: &Exec) -> Option<(u32, Vec<(u32, u32)>)> {
    exec.codes().find_map(|c| match c {
        Code::CopyUid { validity, src, dst } if src.len() == dst.len() => {
            Some((*validity, src.iter().copied().zip(dst.iter().copied()).collect()))
        }
        _ => None,
    })
}

fn prov_to_bad(e: ProviderError) -> ImapError {
    ImapError::Bad(e.to_string())
}

fn auth_error(e: async_imap::error::Error) -> ConnectError {
    use async_imap::error::Error as E;
    match e {
        E::No(info) | E::Bad(info) => {
            let up = info.to_ascii_uppercase();
            if up.contains("UNAVAILABLE") || up.contains("SERVERBUG") {
                ConnectError::Transient("server unavailable".into())
            } else {
                ConnectError::Auth
            }
        }
        E::Validate(_) => ConnectError::Auth,
        other => ConnectError::Transient(other.to_string()),
    }
}
