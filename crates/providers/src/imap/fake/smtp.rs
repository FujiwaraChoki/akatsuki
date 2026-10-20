//! A scripted SMTP submission server on a loopback `TcpListener`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::imap::FakeTls;
use super::tls::{ServerIo, TestTls};

/// What the server does.
#[derive(Clone, Debug)]
pub struct FakeSmtpOptions {
    pub tls: FakeTls,
    /// `AUTH` mechanisms advertised.
    pub mechs: Vec<&'static str>,
    pub username: String,
    pub password: String,
    pub token: String,
    pub size_limit: Option<usize>,
    /// Recipients answered with `550 5.1.1`.
    pub reject_rcpt: Vec<String>,
    /// Reply code to the final `.` (250 accepts).
    pub data_reply: u16,
    /// Close the connection after the message instead of replying (ambiguous outcome).
    pub drop_after_data: bool,
}

impl Default for FakeSmtpOptions {
    fn default() -> Self {
        Self {
            tls: FakeTls::None,
            mechs: vec!["PLAIN", "LOGIN", "XOAUTH2"],
            username: "jane".into(),
            password: "secret".into(),
            token: "tok".into(),
            size_limit: Some(10_000_000),
            reject_rcpt: Vec::new(),
            data_reply: 250,
            drop_after_data: false,
        }
    }
}

/// One accepted message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub from: String,
    pub rcpts: Vec<String>,
    /// The message after un-dot-stuffing.
    pub data: Vec<u8>,
}

/// Shared server state.
#[derive(Debug, Default)]
pub struct SmtpWorld {
    pub delivered: Vec<Delivered>,
    /// Commands received (AUTH payloads redacted).
    pub log: Vec<String>,
}

/// A running fake server.
pub struct FakeSmtp {
    pub addr: SocketAddr,
    pub world: Arc<Mutex<SmtpWorld>>,
    pub tls: Option<TestTls>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeSmtp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeSmtp {
    pub async fn start(opts: FakeSmtpOptions) -> FakeSmtp {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let world = Arc::new(Mutex::new(SmtpWorld::default()));
        let tls = (opts.tls != FakeTls::None).then(TestTls::new);
        let acceptor = tls.as_ref().map(|t| t.acceptor.clone());
        let w = world.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((s, _)) = listener.accept().await else { break };
                let (w, o, a) = (w.clone(), opts.clone(), acceptor.clone());
                tokio::spawn(async move {
                    let _ = serve(s, o, w, a).await;
                });
            }
        });
        FakeSmtp { addr, world, tls, task }
    }

    pub fn world(&self) -> MutexGuard<'_, SmtpWorld> {
        self.world.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

async fn send(r: &mut BufReader<ServerIo>, s: &str) -> std::io::Result<()> {
    let w = r.get_mut();
    w.write_all(s.as_bytes()).await?;
    w.flush().await
}

async fn line(r: &mut BufReader<ServerIo>) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let n = r.read_until(b'\n', &mut buf).await?;
    Ok((n > 0).then(|| String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_owned()))
}

fn path_arg(l: &str) -> String {
    l.split_once('<').and_then(|(_, r)| r.split_once('>')).map(|(a, _)| a.to_owned()).unwrap_or_default()
}

async fn serve(
    stream: TcpStream,
    o: FakeSmtpOptions,
    w: Arc<Mutex<SmtpWorld>>,
    acceptor: Option<TlsAcceptor>,
) -> std::io::Result<()> {
    let io = if o.tls == FakeTls::Implicit {
        ServerIo::Tls(Box::new(acceptor.clone().expect("acceptor").accept(stream).await?))
    } else {
        ServerIo::Plain(stream)
    };
    let mut tls = io.is_tls();
    let mut r = BufReader::new(io);
    let log =
        |w: &Arc<Mutex<SmtpWorld>>, s: String| w.lock().unwrap_or_else(std::sync::PoisonError::into_inner).log.push(s);
    send(&mut r, "220 fake ESMTP ready\r\n").await?;
    let mut authed = false;
    let mut from = String::new();
    let mut rcpts: Vec<String> = Vec::new();
    while let Some(l) = line(&mut r).await? {
        let up = l.to_ascii_uppercase();
        let verb = up.split_whitespace().next().unwrap_or_default().to_owned();
        log(&w, if verb == "AUTH" { up.split_whitespace().take(2).collect::<Vec<_>>().join(" ") } else { l.clone() });
        match verb.as_str() {
            "EHLO" | "HELO" => {
                let mut s = String::from("250-fake greets you\r\n");
                if let Some(n) = o.size_limit {
                    s.push_str(&format!("250-SIZE {n}\r\n"));
                }
                s.push_str("250-8BITMIME\r\n250-SMTPUTF8\r\n");
                if o.tls == FakeTls::StartTls && !tls {
                    s.push_str("250-STARTTLS\r\n");
                }
                s.push_str(&format!("250 AUTH {}\r\n", o.mechs.join(" ")));
                send(&mut r, &s).await?;
            }
            "STARTTLS" => {
                send(&mut r, "220 2.0.0 go ahead\r\n").await?;
                let io = std::mem::replace(&mut r, BufReader::new(ServerIo::Closed)).into_inner();
                let ServerIo::Plain(tcp) = io else { return Ok(()) };
                r = BufReader::new(ServerIo::Tls(Box::new(acceptor.clone().expect("acceptor").accept(tcp).await?)));
                tls = true;
            }
            "AUTH" => {
                if o.tls == FakeTls::StartTls && !tls {
                    send(&mut r, "530 5.7.0 must issue STARTTLS first\r\n").await?;
                    continue;
                }
                let mut words = l.split_whitespace().skip(1);
                let mech = words.next().unwrap_or_default().to_ascii_uppercase();
                let initial = words.next().map(str::to_owned);
                let ok = match mech.as_str() {
                    "PLAIN" => {
                        let d = B64.decode(initial.unwrap_or_default()).unwrap_or_default();
                        let parts: Vec<&[u8]> = d.split(|b| *b == 0).collect();
                        parts.len() == 3 && parts[1] == o.username.as_bytes() && parts[2] == o.password.as_bytes()
                    }
                    "LOGIN" => {
                        send(&mut r, "334 VXNlcm5hbWU6\r\n").await?;
                        let u = B64.decode(line(&mut r).await?.unwrap_or_default()).unwrap_or_default();
                        send(&mut r, "334 UGFzc3dvcmQ6\r\n").await?;
                        let p = B64.decode(line(&mut r).await?.unwrap_or_default()).unwrap_or_default();
                        u == o.username.as_bytes() && p == o.password.as_bytes()
                    }
                    "XOAUTH2" => {
                        let d = B64.decode(initial.unwrap_or_default()).unwrap_or_default();
                        let ok = d == format!("user={}\x01auth=Bearer {}\x01\x01", o.username, o.token).into_bytes();
                        if !ok {
                            send(&mut r, &format!("334 {}\r\n", B64.encode(br#"{"status":"401"}"#))).await?;
                            let _ = line(&mut r).await?;
                        }
                        ok
                    }
                    _ => false,
                };
                authed = ok;
                send(&mut r, if ok { "235 2.7.0 accepted\r\n" } else { "535 5.7.8 bad credentials\r\n" }).await?;
            }
            "MAIL" => {
                if !authed {
                    send(&mut r, "530 5.7.0 authentication required\r\n").await?;
                    continue;
                }
                from = path_arg(&l);
                rcpts.clear();
                send(&mut r, "250 2.1.0 ok\r\n").await?;
            }
            "RCPT" => {
                let to = path_arg(&l);
                if o.reject_rcpt.iter().any(|x| x.eq_ignore_ascii_case(&to)) {
                    send(&mut r, "550 5.1.1 no such user\r\n").await?;
                } else {
                    rcpts.push(to);
                    send(&mut r, "250 2.1.5 ok\r\n").await?;
                }
            }
            "DATA" => {
                send(&mut r, "354 end with <CRLF>.<CRLF>\r\n").await?;
                let mut data = Vec::new();
                loop {
                    let mut buf = Vec::new();
                    if r.read_until(b'\n', &mut buf).await? == 0 {
                        return Ok(());
                    }
                    if buf == b".\r\n" {
                        break;
                    }
                    let b = if buf.starts_with(b"..") { &buf[1..] } else { &buf[..] };
                    data.extend_from_slice(b);
                }
                if o.drop_after_data {
                    return Ok(());
                }
                if o.data_reply == 250 {
                    w.lock().unwrap_or_else(std::sync::PoisonError::into_inner).delivered.push(Delivered {
                        from: from.clone(),
                        rcpts: rcpts.clone(),
                        data,
                    });
                    send(&mut r, "250 2.0.0 queued\r\n").await?;
                } else {
                    send(&mut r, &format!("{} rejected\r\n", o.data_reply)).await?;
                }
            }
            "RSET" | "NOOP" => send(&mut r, "250 ok\r\n").await?,
            "QUIT" => {
                send(&mut r, "221 bye\r\n").await?;
                return Ok(());
            }
            _ => send(&mut r, "502 5.5.2 unknown\r\n").await?,
        }
    }
    Ok(())
}
