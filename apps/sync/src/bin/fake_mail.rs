//! `akatsuki-fake-mail`: the in-process fake IMAP and SMTP servers of `crates/providers`
//! (`imap::fake`) as a small loopback binary, for the local end-to-end harness
//! (`scripts/e2e-local.mjs`, `make e2e`; DECISIONS D-382). Built only with the `e2e-fakes` feature
//! and never shipped in an image.
//!
//! ```text
//! akatsuki-fake-mail --state <file.json> --ca-file <roots.pem>
//! ```
//!
//! It starts implicit-TLS IMAP (IDLE, MOVE, UIDPLUS, CONDSTORE, QRESYNC, SPECIAL-USE, LITERAL+)
//! and SMTP submission on random `127.0.0.1` ports with throwaway self-signed certificates, seeds
//! INBOX with synthetic mail (one hostile HTML message among them), writes both certificates to
//! `--ca-file` (the worker trusts them through `MAIL_CA_FILE`) and writes ports and the throwaway
//! login to `--state` (mode 0600). A loopback control API reports what the servers hold:
//!
//! - `GET /state`: every mailbox (uid, flags, subject, Message-ID) and every SMTP delivery.
//! - `POST /deliver` `{ "mailbox"?, "from", "subject", "text" }`: delivers one more message.
//!
//! All mail is invented; nothing here talks to a real server.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use akatsuki_providers::imap::fake::imap::{FakeImap, FakeImapOptions, FakeTls, World};
use akatsuki_providers::imap::fake::smtp::{FakeSmtp, FakeSmtpOptions, SmtpWorld};
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone)]
struct Ctl {
    imap: Arc<Mutex<World>>,
    smtp: Arc<Mutex<SmtpWorld>>,
    mailbox: String,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| i64::try_from(d.as_millis()).unwrap_or(0)).unwrap_or(0)
}

/// `Subject` and `Message-ID` from the header block (unfolded, undecoded; the seeds are ASCII).
fn headers(raw: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    let mut subject = String::new();
    let mut message_id = String::new();
    for line in head.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("subject:").map(|_| &line[8..]) {
            subject = v.trim().to_owned();
        } else if lower.starts_with("message-id:") {
            message_id = line[11..].trim().to_owned();
        }
    }
    (subject, message_id)
}

fn pem(der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(chunk));
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

fn rfc2822(ms: i64) -> String {
    // A fixed-zone RFC 5322 date without a calendar crate: days since the epoch → civil date.
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let wd = WEEKDAYS[usize::try_from(days.rem_euclid(7)).unwrap_or(0)];
    let month = MONTHS[usize::try_from(m - 1).unwrap_or(0)];
    format!("{wd}, {d:02} {month} {y} {:02}:{:02}:{:02} +0000", rem / 3600, (rem / 60) % 60, rem % 60)
}

struct Seed<'a> {
    from: &'a str,
    subject: &'a str,
    id: &'a str,
    in_reply_to: Option<&'a str>,
    content_type: &'a str,
    body: &'a str,
    age_min: i64,
    seen: bool,
}

fn raw_message(to: &str, s: &Seed<'_>, date_ms: i64) -> Vec<u8> {
    let mut m = format!(
        "From: {}\r\nTo: {to}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: <{}@e2e.akatsuki.test>\r\nMIME-Version: 1.0\r\n",
        s.from,
        s.subject,
        rfc2822(date_ms),
        s.id
    );
    if let Some(parent) = s.in_reply_to {
        m.push_str(&format!(
            "In-Reply-To: <{parent}@e2e.akatsuki.test>\r\nReferences: <{parent}@e2e.akatsuki.test>\r\n"
        ));
    }
    m.push_str(&format!("Content-Type: {}; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n", s.content_type));
    m.push_str(&s.body.replace('\n', "\r\n"));
    m.push_str("\r\n");
    m.into_bytes()
}

/// The synthetic inbox. Message `e2e-hostile` carries everything the sanitizer must remove.
const HOSTILE_HTML: &str = r#"<html><head><style>body{color:red} .x{background:url(https://tracker.example.com/bg.png)}</style>
<script>document.title='pwned'</script></head>
<body onload="alert(1)"><h1>Quarterly report</h1>
<p>Numbers are up. <a href="javascript:alert(2)">Open the dashboard</a> or <a href="https://example.com/report">read it online</a>.</p>
<img src="https://images.example.com/chart.png" alt="Chart" width="400" height="200">
<img src="https://tracker.example.com/open.gif" width="1" height="1" alt="">
<form action="https://phish.example.com/login"><input name="password"><button>Sign in</button></form>
<iframe src="https://example.com/embed"></iframe>
<div onclick="steal()">Thanks, Bob</div></body></html>"#;

fn seeds() -> Vec<Seed<'static>> {
    vec![
        Seed {
            from: "Alice Example <alice@example.com>",
            subject: "Welcome to the e2e inbox",
            id: "e2e-welcome",
            in_reply_to: None,
            content_type: "text/plain",
            body: "Hello!\n\nThis mailbox is a local fake used by the Akatsuki end-to-end test.\n\nAlice",
            age_min: 300,
            seen: true,
        },
        Seed {
            from: "Bob Example <bob@example.com>",
            subject: "Quarterly report (HTML)",
            id: "e2e-hostile",
            in_reply_to: None,
            content_type: "text/html",
            body: HOSTILE_HTML,
            age_min: 120,
            seen: false,
        },
        Seed {
            from: "Carol Example <carol@example.com>",
            subject: "Lunch on Friday?",
            id: "e2e-lunch",
            in_reply_to: None,
            content_type: "text/plain",
            body: "Are you free for lunch on Friday at noon?\n\nCarol",
            age_min: 90,
            seen: false,
        },
        Seed {
            from: "Dave Example <dave@example.com>",
            subject: "Re: Lunch on Friday?",
            id: "e2e-lunch-reply",
            in_reply_to: Some("e2e-lunch"),
            content_type: "text/plain",
            body: "Count me in.\n\nDave\n\n> Are you free for lunch on Friday at noon?",
            age_min: 60,
            seen: false,
        },
    ]
}

async fn state(State(c): State<Ctl>) -> Json<Value> {
    let mailboxes: serde_json::Map<String, Value> = {
        let w = c.imap.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        w.mailboxes
            .iter()
            .map(|(name, b)| {
                let msgs: Vec<Value> = b
                    .msgs
                    .iter()
                    .map(|m| {
                        let (subject, message_id) = headers(&m.raw);
                        json!({ "uid": m.uid, "flags": m.flags, "subject": subject, "messageId": message_id })
                    })
                    .collect();
                (name.clone(), Value::Array(msgs))
            })
            .collect()
    };
    let delivered: Vec<Value> = {
        let w = c.smtp.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        w.delivered
            .iter()
            .map(|d| {
                let (subject, message_id) = headers(&d.data);
                let data = String::from_utf8_lossy(&d.data).to_ascii_lowercase();
                json!({
                    "from": d.from, "rcpts": d.rcpts, "subject": subject, "messageId": message_id,
                    "html": data.contains("text/html"), "hasScript": data.contains("<script"),
                    "hasBccHeader": data.contains("\r\nbcc:"),
                })
            })
            .collect()
    };
    Json(json!({ "mailboxes": mailboxes, "delivered": delivered }))
}

#[derive(Deserialize)]
struct Deliver {
    mailbox: Option<String>,
    from: String,
    subject: String,
    text: String,
}

async fn deliver(State(c): State<Ctl>, Json(d): Json<Deliver>) -> Json<Value> {
    let now = now_ms();
    let id = format!("e2e-{now}");
    let seed = Seed {
        from: &d.from,
        subject: &d.subject,
        id: &id,
        in_reply_to: None,
        content_type: "text/plain",
        body: &d.text,
        age_min: 0,
        seen: false,
    };
    let raw = raw_message(&c.mailbox, &seed, now);
    let mailbox = d.mailbox.unwrap_or_else(|| "INBOX".to_owned());
    let mut w = c.imap.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !w.mailboxes.contains_key(&mailbox) {
        return Json(json!({ "error": "no such mailbox" }));
    }
    let uid = w.deliver(&mailbox, &raw, &[], now);
    Json(json!({ "uid": uid, "messageId": format!("<{id}@e2e.akatsuki.test>") }))
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn write_private(path: &str, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let tmp = format!("{path}.tmp");
    let mut f = opts.open(&tmp)?;
    f.write_all(content)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let (Some(state_file), Some(ca_file)) = (arg("--state"), arg("--ca-file")) else {
        anyhow::bail!("usage: akatsuki-fake-mail --state <file.json> --ca-file <roots.pem>");
    };
    let suffix = hex::encode(akatsuki_sync::crypto::random_bytes::<4>().unwrap_or([0; 4]));
    let username = format!("e2e-{suffix}");
    let password = format!("e2e-pass-{suffix}");
    let mailbox = format!("{username}@example.com");

    let imap_opts = FakeImapOptions {
        tls: FakeTls::Implicit,
        username: username.clone(),
        password: password.clone(),
        ..FakeImapOptions::full()
    };
    let imap = FakeImap::start(imap_opts).await;
    {
        let mut w = imap.world();
        let now = now_ms();
        for s in seeds() {
            let at = now - s.age_min * 60_000;
            let flags: &[&str] = if s.seen { &["\\Seen"] } else { &[] };
            w.deliver("INBOX", &raw_message(&mailbox, &s, at), flags, at);
        }
    }
    let smtp = FakeSmtp::start(FakeSmtpOptions {
        tls: FakeTls::Implicit,
        username: username.clone(),
        password: password.clone(),
        ..FakeSmtpOptions::default()
    })
    .await;

    let mut roots = String::new();
    for t in imap.tls.iter().chain(smtp.tls.iter()) {
        roots.push_str(&pem(t.cert.as_ref()));
    }
    write_private(&ca_file, roots.as_bytes())?;

    let ctl = Ctl { imap: imap.world.clone(), smtp: smtp.world.clone(), mailbox: mailbox.clone() };
    let app = Router::new().route("/state", get(state)).route("/deliver", post(deliver)).with_state(ctl);
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let control = listener.local_addr()?;

    let info = json!({
        "host": "127.0.0.1",
        "imapPort": imap.addr.port(),
        "smtpPort": smtp.addr.port(),
        "controlUrl": format!("http://{control}"),
        "email": mailbox,
        "username": username,
        "password": password,
        "caFile": ca_file,
        "pid": std::process::id(),
    });
    write_private(&state_file, serde_json::to_vec_pretty(&info)?.as_slice())?;
    // Ports only on stdout; the throwaway login stays in the 0600 state file.
    println!(
        "{}",
        json!({ "ready": true, "imapPort": imap.addr.port(), "smtpPort": smtp.addr.port(), "controlUrl": format!("http://{control}") })
    );

    let server = axum::serve(listener, app);
    tokio::select! {
        r = server => r?,
        _ = tokio::signal::ctrl_c() => {}
    }
    drop((imap, smtp));
    Ok(())
}
