//! A scripted IMAP4rev1 server on a loopback `TcpListener`, speaking enough of the protocol and
//! its extensions (CONDSTORE, QRESYNC, MOVE, UIDPLUS, SPECIAL-USE, IDLE, LITERAL+, X-GM-EXT-1,
//! STARTTLS, AUTH=PLAIN, AUTH=XOAUTH2) for the adapter tests. State lives in a shared [`World`]
//! that tests mutate directly (deliver mail, flip flags, reset UIDVALIDITY, inject failures).

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use mail_parser::{MessageParser, MimeHeaders, PartType};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use super::tls::{ServerIo, TestTls};
use crate::imap::proto::{format_internal_date, parse_internal_date};
use crate::imap::utf7;

/// How the server is secured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FakeTls {
    None,
    Implicit,
    StartTls,
}

/// Server behaviour.
#[derive(Clone, Debug)]
pub struct FakeImapOptions {
    /// Extension capabilities (`IDLE`, `MOVE`, …).
    pub caps: Vec<&'static str>,
    pub auth_plain: bool,
    pub auth_xoauth2: bool,
    pub username: String,
    pub password: String,
    pub token: String,
    pub tls: FakeTls,
    pub delimiter: &'static str,
}

impl FakeImapOptions {
    /// Every extension the adapter uses.
    pub fn full() -> Self {
        Self {
            caps: vec!["IDLE", "MOVE", "UIDPLUS", "CONDSTORE", "QRESYNC", "ENABLE", "SPECIAL-USE", "LITERAL+"],
            auth_plain: true,
            auth_xoauth2: true,
            username: "jane".into(),
            password: "secret".into(),
            token: "tok".into(),
            tls: FakeTls::None,
            delimiter: "/",
        }
    }

    /// Plain IMAP4rev1: no extensions at all.
    pub fn minimal() -> Self {
        Self { caps: vec![], auth_plain: false, auth_xoauth2: false, ..Self::full() }
    }

    pub fn with_caps(mut self, caps: &[&'static str]) -> Self {
        self.caps = caps.to_vec();
        self
    }

    fn has(&self, c: &str) -> bool {
        self.caps.iter().any(|x| x.eq_ignore_ascii_case(c))
    }
}

/// One stored message.
#[derive(Clone, Debug)]
pub struct Msg {
    pub uid: u32,
    pub flags: BTreeSet<String>,
    pub internal_date: i64,
    pub raw: Vec<u8>,
    pub modseq: u64,
    pub thrid: u64,
}

/// One mailbox.
#[derive(Clone, Debug)]
pub struct Mbox {
    pub attrs: Vec<String>,
    pub validity: u32,
    pub uid_next: u32,
    pub msgs: Vec<Msg>,
    /// `(uid, modseq)` of expunged messages, for `VANISHED (EARLIER)`.
    pub vanished: Vec<(u32, u64)>,
    pub highest_modseq: u64,
}

/// A scripted failure for the next command with this verb.
#[derive(Clone, Debug)]
pub struct Failure {
    /// `"UID MOVE"`, `"APPEND"`, … (uppercase).
    pub verb: String,
    /// `"NO [UNAVAILABLE] try later"`, or `"DROP"` to close the connection.
    pub reply: String,
}

/// Server state shared by all connections and the test.
#[derive(Debug, Default)]
pub struct World {
    /// Keyed by the UTF-8 name.
    pub mailboxes: BTreeMap<String, Mbox>,
    pub modseq: u64,
    next_validity: u32,
    /// Commands received (verb and non-secret arguments).
    pub log: Vec<String>,
    pub failures: Vec<Failure>,
    /// Bumped by [`FakeImap::kick`]: connections of older generations close.
    pub generation: u64,
}

impl World {
    fn bump(&mut self) -> u64 {
        self.modseq += 1;
        self.modseq
    }

    /// Adds a mailbox.
    pub fn add_mailbox(&mut self, name: &str, attrs: &[&str]) {
        self.next_validity += 1;
        let v = 1000 + self.next_validity;
        self.mailboxes.entry(name.to_owned()).or_insert(Mbox {
            attrs: attrs.iter().map(|s| (*s).to_owned()).collect(),
            validity: v,
            uid_next: 1,
            msgs: Vec::new(),
            vanished: Vec::new(),
            highest_modseq: 0,
        });
    }

    /// Delivers a message; returns its UID.
    pub fn deliver(&mut self, mbox: &str, raw: &[u8], flags: &[&str], date: i64) -> u32 {
        let m = self.bump();
        let thrid = 0x1000 + u64::from(self.mailboxes.get(mbox).map_or(0, |b| b.uid_next));
        let b = self.mailboxes.get_mut(mbox).expect("mailbox exists");
        let uid = b.uid_next;
        b.uid_next += 1;
        b.msgs.push(Msg {
            uid,
            flags: flags.iter().map(|s| (*s).to_owned()).collect(),
            internal_date: date,
            raw: raw.to_vec(),
            modseq: m,
            thrid,
        });
        b.highest_modseq = m;
        uid
    }

    /// Sets the X-GM-THRID of a message.
    pub fn set_thrid(&mut self, mbox: &str, uid: u32, thrid: u64) {
        if let Some(msg) = self.mailboxes.get_mut(mbox).and_then(|b| b.msgs.iter_mut().find(|m| m.uid == uid)) {
            msg.thrid = thrid;
        }
    }

    /// Replaces a message's flags.
    pub fn set_flags(&mut self, mbox: &str, uid: u32, flags: &[&str]) {
        let m = self.bump();
        let b = self.mailboxes.get_mut(mbox).expect("mailbox exists");
        if let Some(msg) = b.msgs.iter_mut().find(|x| x.uid == uid) {
            msg.flags = flags.iter().map(|s| (*s).to_owned()).collect();
            msg.modseq = m;
            b.highest_modseq = m;
        }
    }

    /// Expunges one message.
    pub fn expunge(&mut self, mbox: &str, uid: u32) {
        let m = self.bump();
        let b = self.mailboxes.get_mut(mbox).expect("mailbox exists");
        b.msgs.retain(|x| x.uid != uid);
        b.vanished.push((uid, m));
        b.highest_modseq = m;
    }

    /// New UIDVALIDITY; messages renumbered from 1.
    pub fn reset_validity(&mut self, mbox: &str) {
        self.next_validity += 1;
        let v = 5000 + self.next_validity;
        let b = self.mailboxes.get_mut(mbox).expect("mailbox exists");
        b.validity = v;
        for (i, m) in b.msgs.iter_mut().enumerate() {
            m.uid = u32::try_from(i).unwrap_or(0) + 1;
        }
        b.uid_next = u32::try_from(b.msgs.len()).unwrap_or(0) + 1;
        b.vanished.clear();
    }

    pub fn mbox(&self, name: &str) -> &Mbox {
        self.mailboxes.get(name).expect("mailbox exists")
    }

    pub fn msg(&self, name: &str, uid: u32) -> Option<&Msg> {
        self.mbox(name).msgs.iter().find(|m| m.uid == uid)
    }

    /// Injects a failure.
    pub fn fail_next(&mut self, verb: &str, reply: &str) {
        self.failures.push(Failure { verb: verb.to_ascii_uppercase(), reply: reply.to_owned() });
    }

    /// `true` if any logged command starts with `prefix`.
    pub fn saw(&self, prefix: &str) -> bool {
        self.log.iter().any(|l| l.starts_with(prefix))
    }
}

/// A running fake server.
pub struct FakeImap {
    pub addr: SocketAddr,
    pub world: Arc<Mutex<World>>,
    pub tls: Option<TestTls>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeImap {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeImap {
    /// Starts a server with INBOX, Sent, Drafts, Trash, Junk and Archive.
    pub async fn start(opts: FakeImapOptions) -> FakeImap {
        let mut w = World::default();
        let su = opts.has("SPECIAL-USE");
        w.add_mailbox("INBOX", &[]);
        for (name, attr) in [
            ("Sent", "\\Sent"),
            ("Drafts", "\\Drafts"),
            ("Trash", "\\Trash"),
            ("Junk", "\\Junk"),
            ("Archive", "\\Archive"),
        ] {
            let attrs: Vec<&str> = if su { vec![attr] } else { vec![] };
            w.add_mailbox(name, &attrs);
        }
        Self::start_with(opts, w).await
    }

    /// Starts a server with the given world.
    pub async fn start_with(opts: FakeImapOptions, world: World) -> FakeImap {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let world = Arc::new(Mutex::new(world));
        let tls = (opts.tls != FakeTls::None).then(TestTls::new);
        let acceptor = tls.as_ref().map(|t| t.acceptor.clone());
        let w = world.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { break };
                let (w, o, a) = (w.clone(), opts.clone(), acceptor.clone());
                tokio::spawn(async move {
                    let _ = serve(stream, o, w, a).await;
                });
            }
        });
        FakeImap { addr, world, tls, task }
    }

    pub fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Closes every open connection (they notice within ~30 ms).
    pub fn kick(&self) {
        self.world().generation += 1;
    }
}

// ───────────────────────── command parsing ─────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Atom(String),
    Str(String),
    Lit(Vec<u8>),
    List(Vec<Tok>),
}

impl Tok {
    fn text(&self) -> String {
        match self {
            Tok::Atom(s) | Tok::Str(s) => s.clone(),
            Tok::Lit(b) => String::from_utf8_lossy(b).into_owned(),
            Tok::List(_) => String::new(),
        }
    }
    fn upper(&self) -> String {
        self.text().to_ascii_uppercase()
    }
}

const LIT: char = '\u{1}';

fn tokenize(s: &str, lits: &[Vec<u8>]) -> Vec<Tok> {
    let chars: Vec<char> = s.chars().collect();
    let mut pos = 0;
    parse_list(&chars, &mut pos, lits, false)
}

fn parse_list(c: &[char], pos: &mut usize, lits: &[Vec<u8>], nested: bool) -> Vec<Tok> {
    let mut out = Vec::new();
    while *pos < c.len() {
        match c[*pos] {
            ' ' => *pos += 1,
            ')' if nested => {
                *pos += 1;
                return out;
            }
            '(' => {
                *pos += 1;
                out.push(Tok::List(parse_list(c, pos, lits, true)));
            }
            '"' => {
                *pos += 1;
                let mut s = String::new();
                while *pos < c.len() && c[*pos] != '"' {
                    if c[*pos] == '\\' && *pos + 1 < c.len() {
                        *pos += 1;
                    }
                    s.push(c[*pos]);
                    *pos += 1;
                }
                *pos += 1;
                out.push(Tok::Str(s));
            }
            LIT => {
                *pos += 1;
                let mut n = String::new();
                while *pos < c.len() && c[*pos] != LIT {
                    n.push(c[*pos]);
                    *pos += 1;
                }
                *pos += 1;
                let i: usize = n.parse().unwrap_or(0);
                out.push(Tok::Lit(lits.get(i).cloned().unwrap_or_default()));
            }
            _ => {
                let mut s = String::new();
                let mut depth = 0;
                while *pos < c.len() {
                    let ch = c[*pos];
                    if depth == 0 && (ch == ' ' || ch == ')' || ch == '(') {
                        break;
                    }
                    if ch == '[' {
                        depth += 1;
                    } else if ch == ']' {
                        depth -= 1;
                    }
                    s.push(ch);
                    *pos += 1;
                }
                out.push(Tok::Atom(s));
            }
        }
    }
    out
}

struct Conn {
    r: BufReader<ServerIo>,
    authed: bool,
    selected: Option<String>,
    read_only: bool,
    qresync: bool,
    tls: bool,
    generation: u64,
}

impl Conn {
    async fn send(&mut self, s: &str) -> std::io::Result<()> {
        let w = self.r.get_mut();
        w.write_all(s.as_bytes()).await?;
        w.flush().await
    }

    async fn send_bytes(&mut self, b: &[u8]) -> std::io::Result<()> {
        let w = self.r.get_mut();
        w.write_all(b).await?;
        w.flush().await
    }

    async fn read_line(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        let mut buf = Vec::new();
        let n = self.r.read_until(b'\n', &mut buf).await?;
        Ok((n > 0).then_some(buf))
    }

    /// Reads one command with its literals.
    async fn read_command(&mut self) -> std::io::Result<Option<(String, Vec<Tok>, String)>> {
        let mut text = String::new();
        let mut lits: Vec<Vec<u8>> = Vec::new();
        loop {
            let Some(line) = self.read_line().await? else { return Ok(None) };
            let line = String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_owned();
            if line.ends_with('}')
                && let Some(open) = line.rfind('{')
            {
                let spec = &line[open + 1..line.len() - 1];
                let plus = spec.ends_with('+');
                let n: usize = spec.trim_end_matches('+').parse().unwrap_or(0);
                text.push_str(&line[..open]);
                if !plus {
                    self.send("+ go ahead\r\n").await?;
                }
                let mut buf = vec![0u8; n];
                self.r.read_exact(&mut buf).await?;
                text.push(LIT);
                text.push_str(&lits.len().to_string());
                text.push(LIT);
                lits.push(buf);
                continue;
            }
            text.push_str(&line);
            break;
        }
        let mut toks = tokenize(&text, &lits);
        if toks.is_empty() {
            return Ok(Some((String::new(), Vec::new(), String::new())));
        }
        let tag = toks.remove(0).text();
        let shown: String = text.split(LIT).enumerate().map(|(i, p)| if i % 2 == 1 { "{lit}" } else { p }).collect();
        let shown = shown.split_once(' ').map(|(_, r)| r.to_owned()).unwrap_or_default();
        Ok(Some((tag, toks, shown)))
    }
}

// ───────────────────────── helpers ─────────────────────────

fn q(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn caps_line(o: &FakeImapOptions, tls: bool) -> String {
    let mut v = vec!["IMAP4rev1".to_owned()];
    v.extend(o.caps.iter().map(|s| (*s).to_owned()));
    if o.tls == FakeTls::StartTls && !tls {
        v.push("STARTTLS".into());
        v.push("LOGINDISABLED".into());
    }
    if o.auth_plain {
        v.push("AUTH=PLAIN".into());
    }
    if o.auth_xoauth2 {
        v.push("AUTH=XOAUTH2".into());
    }
    v.join(" ")
}

/// Parses an IMAP sequence set against `max` (the value of `*`).
fn parse_set(s: &str, max: u32) -> Vec<(u32, u32)> {
    s.split(',')
        .filter_map(|part| {
            let val = |x: &str| if x == "*" { Some(max) } else { x.parse::<u32>().ok() };
            match part.split_once(':') {
                Some((a, b)) => {
                    let (a, b) = (val(a)?, val(b)?);
                    Some((a.min(b), a.max(b)))
                }
                None => val(part).map(|v| (v, v)),
            }
        })
        .collect()
}

fn in_set(ranges: &[(u32, u32)], v: u32) -> bool {
    ranges.iter().any(|(a, b)| v >= *a && v <= *b)
}

fn compress(v: &[u32]) -> String {
    crate::imap::proto::uid_set(v)
}

fn day_start(s: &str) -> Option<i64> {
    parse_internal_date(&format!("{s} 00:00:00 +0000"))
}

fn header_end(raw: &[u8]) -> usize {
    raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(raw.len(), |p| p + 4)
}

fn header_fields(raw: &[u8], names: &[String], not: bool) -> Vec<u8> {
    let head = &raw[..header_end(raw)];
    let mut out = Vec::new();
    let mut keep = false;
    for line in head.split_inclusive(|b| *b == b'\n') {
        if line == b"\r\n" {
            break;
        }
        if line.first().is_some_and(|b| *b == b' ' || *b == b'\t') {
            if keep {
                out.extend_from_slice(line);
            }
            continue;
        }
        let name = line
            .split(|b| *b == b':')
            .next()
            .map(|n| String::from_utf8_lossy(n).trim().to_ascii_uppercase())
            .unwrap_or_default();
        keep = names.iter().any(|n| n.eq_ignore_ascii_case(&name)) != not;
        if keep {
            out.extend_from_slice(line);
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// Part path → mail-parser part index.
fn part_map(msg: &mail_parser::Message<'_>) -> BTreeMap<String, usize> {
    fn walk(msg: &mail_parser::Message<'_>, children: &[u32], prefix: &str, out: &mut BTreeMap<String, usize>) {
        for (i, c) in children.iter().enumerate() {
            let path = if prefix.is_empty() { format!("{}", i + 1) } else { format!("{prefix}.{}", i + 1) };
            out.insert(path.clone(), *c as usize);
            if let Some(PartType::Multipart(g)) = msg.parts.get(*c as usize).map(|p| &p.body) {
                walk(msg, g, &path, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    if let Some(root) = msg.parts.first() {
        match &root.body {
            PartType::Multipart(ch) => walk(msg, ch, "", &mut out),
            _ => {
                out.insert("1".into(), 0);
            }
        }
    }
    out
}

fn section_bytes(raw: &[u8], section: &str) -> Vec<u8> {
    let up = section.to_ascii_uppercase();
    if up.is_empty() {
        return raw.to_vec();
    }
    if up == "HEADER" {
        return raw[..header_end(raw)].to_vec();
    }
    if up == "TEXT" {
        return raw[header_end(raw)..].to_vec();
    }
    if let Some(rest) = up.strip_prefix("HEADER.FIELDS") {
        let not = rest.starts_with(".NOT");
        let list = rest.trim_start_matches(".NOT").trim().trim_start_matches('(').trim_end_matches(')');
        let names: Vec<String> = list.split_whitespace().map(str::to_owned).collect();
        return header_fields(raw, &names, not);
    }
    let Some(msg) = MessageParser::default().parse(raw) else { return Vec::new() };
    let map = part_map(&msg);
    let (path, mime) = match up.strip_suffix(".MIME") {
        Some(p) => (p.to_owned(), true),
        None => (up.clone(), false),
    };
    let Some(idx) = map.get(&path) else { return Vec::new() };
    let p = &msg.parts[*idx];
    let (a, b) = if mime { (p.offset_header, p.offset_body) } else { (p.offset_body, p.offset_end) };
    raw.get(a as usize..b as usize).map(<[u8]>::to_vec).unwrap_or_default()
}

fn bodystructure(raw: &[u8]) -> String {
    let Some(msg) = MessageParser::default().parse(raw) else {
        return "(\"text\" \"plain\" NIL NIL NIL \"7bit\" 0 0)".into();
    };
    part_bs(&msg, 0, 0)
}

fn params(attrs: Option<&[mail_parser::Attribute<'_>]>) -> String {
    match attrs {
        Some(a) if !a.is_empty() => {
            format!("({})", a.iter().map(|x| format!("{} {}", q(&x.name), q(&x.value))).collect::<Vec<_>>().join(" "))
        }
        _ => "NIL".into(),
    }
}

fn part_bs(msg: &mail_parser::Message<'_>, id: usize, depth: u32) -> String {
    let Some(part) = msg.parts.get(id) else { return "NIL".into() };
    if let PartType::Multipart(children) = &part.body
        && depth < 16
    {
        let inner: String = children.iter().map(|c| part_bs(msg, *c as usize, depth + 1)).collect();
        let sub = part.content_type().and_then(|c| c.subtype()).unwrap_or("mixed").to_owned();
        return format!("({inner} {})", q(&sub));
    }
    let ct = part.content_type();
    let (mut ty, mut sub) = (
        ct.map_or("text", |c| c.ctype()).to_ascii_lowercase(),
        ct.and_then(|c| c.subtype()).unwrap_or("plain").to_ascii_lowercase(),
    );
    if ty == "message" {
        ty = "application".into();
        sub = "octet-stream".into();
    }
    let p = params(ct.and_then(|c| c.attributes()));
    let enc = part.content_transfer_encoding().unwrap_or("7bit").to_ascii_lowercase();
    let size = part.offset_end.saturating_sub(part.offset_body);
    let dsp = match part.content_disposition() {
        Some(d) => format!("({} {})", q(d.ctype()), params(d.attributes())),
        None => "NIL".into(),
    };
    if ty == "text" {
        let body = msg.raw_message.get(part.offset_body as usize..part.offset_end as usize).unwrap_or_default();
        let lines = body.iter().filter(|b| **b == b'\n').count();
        format!("({} {} {p} NIL NIL {} {size} {lines} NIL {dsp} NIL)", q(&ty), q(&sub), q(&enc))
    } else {
        format!("({} {} {p} NIL NIL {} {size} NIL {dsp} NIL)", q(&ty), q(&sub), q(&enc))
    }
}

fn header_value(raw: &[u8], name: &str) -> String {
    let fields = header_fields(raw, &[name.to_ascii_uppercase()], false);
    let s = String::from_utf8_lossy(&fields).to_string();
    s.split_once(':').map(|(_, v)| v.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

fn flags_str(f: &BTreeSet<String>) -> String {
    format!("({})", f.iter().cloned().collect::<Vec<_>>().join(" "))
}

// ───────────────────────── search ─────────────────────────

fn matches(m: &Msg, seq: u32, max_uid: u32, toks: &[Tok], i: &mut usize) -> bool {
    let Some(t) = toks.get(*i) else { return true };
    *i += 1;
    let has = |f: &str| m.flags.iter().any(|x| x.eq_ignore_ascii_case(f));
    let next_text = |i: &mut usize| {
        let s = toks.get(*i).map(Tok::text).unwrap_or_default();
        *i += 1;
        s
    };
    let contains = |hay: &str, needle: &str| hay.to_lowercase().contains(&needle.to_lowercase());
    match t {
        Tok::List(inner) => {
            let mut j = 0;
            let mut ok = true;
            while j < inner.len() {
                ok &= matches(m, seq, max_uid, inner, &mut j);
            }
            ok
        }
        _ => match t.upper().as_str() {
            "ALL" => true,
            "CHARSET" => {
                *i += 1;
                matches(m, seq, max_uid, toks, i)
            }
            "UID" => {
                let set = next_text(i);
                in_set(&parse_set(&set, max_uid), m.uid)
            }
            "SEEN" => has("\\Seen"),
            "UNSEEN" => !has("\\Seen"),
            "FLAGGED" => has("\\Flagged"),
            "UNFLAGGED" => !has("\\Flagged"),
            "DRAFT" => has("\\Draft"),
            "DELETED" => has("\\Deleted"),
            "KEYWORD" => {
                let k = next_text(i);
                has(&k)
            }
            "SINCE" => {
                let d = next_text(i);
                day_start(&d).is_some_and(|s| m.internal_date >= s)
            }
            "BEFORE" => {
                let d = next_text(i);
                day_start(&d).is_some_and(|s| m.internal_date < s)
            }
            "HEADER" => {
                let name = next_text(i);
                let v = next_text(i);
                contains(&header_value(&m.raw, &name), &v)
            }
            "FROM" | "TO" | "CC" | "SUBJECT" => {
                let name = t.upper();
                let v = next_text(i);
                let decoded = MessageParser::default().parse(&m.raw).map(|p| match name.as_str() {
                    "SUBJECT" => p.subject().unwrap_or_default().to_owned(),
                    _ => header_value(&m.raw, &name),
                });
                contains(&decoded.unwrap_or_default(), &v)
            }
            "TEXT" | "BODY" => {
                let v = next_text(i);
                let text = MessageParser::default()
                    .parse(&m.raw)
                    .map(|p| format!("{} {}", p.subject().unwrap_or_default(), p.body_text(0).unwrap_or_default()))
                    .unwrap_or_default();
                contains(&text, &v) || contains(&String::from_utf8_lossy(&m.raw), &v)
            }
            "X-GM-THRID" => {
                let v = next_text(i);
                v.parse::<u64>().is_ok_and(|x| x == m.thrid)
            }
            "NOT" => !matches(m, seq, max_uid, toks, i),
            "OR" => {
                let a = matches(m, seq, max_uid, toks, i);
                let b = matches(m, seq, max_uid, toks, i);
                a || b
            }
            other => in_set(&parse_set(other, u32::MAX), seq),
        },
    }
}

// ───────────────────────── the session ─────────────────────────

fn lock(w: &Arc<Mutex<World>>) -> MutexGuard<'_, World> {
    w.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn serve(
    stream: TcpStream,
    o: FakeImapOptions,
    w: Arc<Mutex<World>>,
    acceptor: Option<TlsAcceptor>,
) -> std::io::Result<()> {
    let generation = lock(&w).generation;
    let io = if o.tls == FakeTls::Implicit {
        let a = acceptor.clone().expect("acceptor");
        ServerIo::Tls(Box::new(a.accept(stream).await?))
    } else {
        ServerIo::Plain(stream)
    };
    let tls = io.is_tls();
    let mut c = Conn {
        r: BufReader::new(io),
        authed: false,
        selected: None,
        read_only: false,
        qresync: false,
        tls,
        generation,
    };
    c.send(&format!("* OK [CAPABILITY {}] fake IMAP ready\r\n", caps_line(&o, c.tls))).await?;
    loop {
        if lock(&w).generation != c.generation {
            return Ok(());
        }
        let Some((tag, toks, shown)) = c.read_command().await? else { return Ok(()) };
        if lock(&w).generation != c.generation {
            return Ok(());
        }
        let words: Vec<String> = toks.iter().map(Tok::upper).collect();
        let mut verb = words.first().cloned().unwrap_or_default();
        let mut args: Vec<Tok> = toks.iter().skip(1).cloned().collect();
        if verb == "UID" && !args.is_empty() {
            verb = format!("UID {}", args.remove(0).upper());
        }
        let failure = {
            let mut wl = lock(&w);
            let logged = match verb.as_str() {
                "LOGIN" | "AUTHENTICATE" => verb.clone(),
                _ => shown.clone(),
            };
            wl.log.push(logged);
            let pos = wl.failures.iter().position(|f| f.verb == verb);
            pos.map(|p| wl.failures.remove(p))
        };
        if let Some(f) = failure {
            if f.reply == "DROP" {
                return Ok(());
            }
            c.send(&format!("{tag} {}\r\n", f.reply)).await?;
            continue;
        }
        match verb.as_str() {
            "CAPABILITY" => {
                c.send(&format!("* CAPABILITY {}\r\n{tag} OK done\r\n", caps_line(&o, c.tls))).await?;
            }
            "NOOP" | "CHECK" | "SUBSCRIBE" | "UNSUBSCRIBE" => c.send(&format!("{tag} OK done\r\n")).await?,
            "LOGOUT" => {
                c.send(&format!("* BYE bye\r\n{tag} OK done\r\n")).await?;
                return Ok(());
            }
            "STARTTLS" => {
                if o.tls != FakeTls::StartTls || c.tls {
                    c.send(&format!("{tag} BAD no\r\n")).await?;
                    continue;
                }
                c.send(&format!("{tag} OK begin TLS\r\n")).await?;
                let io = std::mem::replace(&mut c.r, BufReader::new(ServerIo::Closed)).into_inner();
                let ServerIo::Plain(tcp) = io else { return Ok(()) };
                let a = acceptor.clone().expect("acceptor");
                c.r = BufReader::new(ServerIo::Tls(Box::new(a.accept(tcp).await?)));
                c.tls = true;
            }
            "LOGIN" => {
                if o.tls == FakeTls::StartTls && !c.tls {
                    c.send(&format!("{tag} NO [PRIVACYREQUIRED] use STARTTLS\r\n")).await?;
                } else if args.len() == 2 && args[0].text() == o.username && args[1].text() == o.password {
                    c.authed = true;
                    c.send(&format!("{tag} OK [CAPABILITY {}] logged in\r\n", caps_line(&o, c.tls))).await?;
                } else {
                    c.send(&format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")).await?;
                }
            }
            "AUTHENTICATE" => {
                let mech = args.first().map(Tok::upper).unwrap_or_default();
                c.send("+ \r\n").await?;
                let line = c.read_line().await?.unwrap_or_default();
                let decoded = B64.decode(String::from_utf8_lossy(&line).trim()).unwrap_or_default();
                let ok = match mech.as_str() {
                    "PLAIN" if o.auth_plain => {
                        let parts: Vec<&[u8]> = decoded.split(|b| *b == 0).collect();
                        parts.len() == 3 && parts[1] == o.username.as_bytes() && parts[2] == o.password.as_bytes()
                    }
                    "XOAUTH2" if o.auth_xoauth2 => {
                        let s = String::from_utf8_lossy(&decoded).to_string();
                        s == format!("user={}\x01auth=Bearer {}\x01\x01", o.username, o.token)
                    }
                    _ => false,
                };
                if ok {
                    c.authed = true;
                    c.send(&format!("{tag} OK authenticated\r\n")).await?;
                } else {
                    if mech == "XOAUTH2" {
                        c.send(&format!("+ {}\r\n", B64.encode(br#"{"status":"401"}"#))).await?;
                        let _ = c.read_line().await?;
                    }
                    c.send(&format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")).await?;
                }
            }
            _ if !c.authed => c.send(&format!("{tag} BAD log in first\r\n")).await?,
            "ENABLE" => {
                if words.iter().any(|x| x == "QRESYNC") && o.has("QRESYNC") {
                    c.qresync = true;
                    c.send(&format!("* ENABLED QRESYNC\r\n{tag} OK enabled\r\n")).await?;
                } else {
                    c.send(&format!("{tag} OK nothing enabled\r\n")).await?;
                }
            }
            "LIST" | "LSUB" => {
                let mut out = String::new();
                for (name, b) in &lock(&w).mailboxes {
                    out.push_str(&format!(
                        "* {verb} ({}) {} {}\r\n",
                        b.attrs.join(" "),
                        q(o.delimiter),
                        q(&utf7::encode(name))
                    ));
                }
                out.push_str(&format!("{tag} OK done\r\n"));
                c.send(&out).await?;
            }
            "STATUS" => {
                let name = utf7::decode(&args.first().map(Tok::text).unwrap_or_default());
                let items: Vec<String> = match args.get(1) {
                    Some(Tok::List(l)) => l.iter().map(Tok::upper).collect(),
                    _ => Vec::new(),
                };
                let reply = {
                    let wl = lock(&w);
                    wl.mailboxes.get(&name).map(|b| {
                        let vals: Vec<String> = items
                            .iter()
                            .filter_map(|i| {
                                Some(match i.as_str() {
                                    "MESSAGES" => format!("MESSAGES {}", b.msgs.len()),
                                    "UIDNEXT" => format!("UIDNEXT {}", b.uid_next),
                                    "UIDVALIDITY" => format!("UIDVALIDITY {}", b.validity),
                                    "UNSEEN" => format!(
                                        "UNSEEN {}",
                                        b.msgs.iter().filter(|m| !m.flags.contains("\\Seen")).count()
                                    ),
                                    "HIGHESTMODSEQ" if o.has("CONDSTORE") => {
                                        format!("HIGHESTMODSEQ {}", b.highest_modseq.max(1))
                                    }
                                    _ => return None,
                                })
                            })
                            .collect();
                        format!("* STATUS {} ({})\r\n{tag} OK done\r\n", q(&utf7::encode(&name)), vals.join(" "))
                    })
                };
                c.send(&reply.unwrap_or_else(|| format!("{tag} NO [NONEXISTENT] no such mailbox\r\n"))).await?;
            }
            "SELECT" | "EXAMINE" => {
                let name = utf7::decode(&args.first().map(Tok::text).unwrap_or_default());
                let mut qr: Option<(u32, u64)> = None;
                if let Some(Tok::List(p)) = args.get(1) {
                    let up: Vec<String> = p.iter().map(Tok::upper).collect();
                    if up.first().map(String::as_str) == Some("QRESYNC")
                        && let Some(Tok::List(v)) = p.get(1)
                    {
                        let v0 = v.first().map(Tok::text).and_then(|x| x.parse().ok()).unwrap_or(0);
                        let m0 = v.get(1).map(Tok::text).and_then(|x| x.parse().ok()).unwrap_or(0);
                        qr = Some((v0, m0));
                    }
                }
                let reply = {
                    let wl = lock(&w);
                    wl.mailboxes.get(&name).map(|b| {
                        let mut kws: BTreeSet<String> = ["\\Answered", "\\Flagged", "\\Deleted", "\\Seen", "\\Draft"]
                            .iter()
                            .map(|s| (*s).to_owned())
                            .collect();
                        for m in &b.msgs {
                            kws.extend(m.flags.iter().filter(|f| !f.starts_with('\\')).cloned());
                        }
                        let mut s = format!("* FLAGS ({})\r\n", kws.iter().cloned().collect::<Vec<_>>().join(" "));
                        s.push_str("* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] ok\r\n");
                        s.push_str(&format!("* {} EXISTS\r\n* 0 RECENT\r\n", b.msgs.len()));
                        s.push_str(&format!(
                            "* OK [UIDVALIDITY {}] ok\r\n* OK [UIDNEXT {}] ok\r\n",
                            b.validity, b.uid_next
                        ));
                        if o.has("CONDSTORE") {
                            s.push_str(&format!("* OK [HIGHESTMODSEQ {}] ok\r\n", b.highest_modseq.max(1)));
                        }
                        if let Some((v, m)) = qr
                            && v == b.validity
                        {
                            let gone: Vec<u32> = b.vanished.iter().filter(|(_, ms)| *ms > m).map(|(u, _)| *u).collect();
                            if !gone.is_empty() {
                                s.push_str(&format!("* VANISHED (EARLIER) {}\r\n", compress(&gone)));
                            }
                            for (i, msg) in b.msgs.iter().enumerate() {
                                if msg.modseq > m {
                                    s.push_str(&format!(
                                        "* {} FETCH (UID {} FLAGS {} MODSEQ ({}))\r\n",
                                        i + 1,
                                        msg.uid,
                                        flags_str(&msg.flags),
                                        msg.modseq
                                    ));
                                }
                            }
                        }
                        let ro = if verb == "EXAMINE" { "READ-ONLY" } else { "READ-WRITE" };
                        s.push_str(&format!("{tag} OK [{ro}] {verb} completed\r\n"));
                        s
                    })
                };
                match reply {
                    Some(r) => {
                        c.selected = Some(name);
                        c.read_only = verb == "EXAMINE";
                        c.send(&r).await?;
                    }
                    None => {
                        c.selected = None;
                        c.send(&format!("{tag} NO [NONEXISTENT] no such mailbox\r\n")).await?;
                    }
                }
            }
            "CREATE" => {
                let name = utf7::decode(&args.first().map(Tok::text).unwrap_or_default());
                let reply = {
                    let mut wl = lock(&w);
                    if wl.mailboxes.contains_key(&name) {
                        format!("{tag} NO [ALREADYEXISTS] exists\r\n")
                    } else {
                        wl.add_mailbox(&name, &[]);
                        format!("{tag} OK created\r\n")
                    }
                };
                c.send(&reply).await?;
            }
            "DELETE" => {
                let name = utf7::decode(&args.first().map(Tok::text).unwrap_or_default());
                let removed = lock(&w).mailboxes.remove(&name).is_some();
                c.send(&if removed {
                    format!("{tag} OK deleted\r\n")
                } else {
                    format!("{tag} NO [NONEXISTENT] no such mailbox\r\n")
                })
                .await?;
            }
            "RENAME" => {
                let from = utf7::decode(&args.first().map(Tok::text).unwrap_or_default());
                let to = utf7::decode(&args.get(1).map(Tok::text).unwrap_or_default());
                let reply = {
                    let mut wl = lock(&w);
                    match wl.mailboxes.remove(&from) {
                        Some(mut b) => {
                            wl.next_validity += 1;
                            b.validity = 9000 + wl.next_validity;
                            wl.mailboxes.insert(to, b);
                            format!("{tag} OK renamed\r\n")
                        }
                        None => format!("{tag} NO [NONEXISTENT] no such mailbox\r\n"),
                    }
                };
                c.send(&reply).await?;
            }
            "APPEND" => {
                let name = utf7::decode(&args.first().map(Tok::text).unwrap_or_default());
                let mut flags: Vec<String> = Vec::new();
                let mut date = akatsuki_core::time::now_ms();
                let mut data = Vec::new();
                for a in args.iter().skip(1) {
                    match a {
                        Tok::List(l) => flags = l.iter().map(Tok::text).collect(),
                        Tok::Str(s) => date = parse_internal_date(s).unwrap_or(date),
                        Tok::Lit(b) => data = b.clone(),
                        Tok::Atom(_) => {}
                    }
                }
                let reply = {
                    let mut wl = lock(&w);
                    if wl.mailboxes.contains_key(&name) {
                        let refs: Vec<&str> = flags.iter().map(String::as_str).collect();
                        let uid = wl.deliver(&name, &data, &refs, date);
                        let v = wl.mbox(&name).validity;
                        if o.has("UIDPLUS") {
                            format!("{tag} OK [APPENDUID {v} {uid}] appended\r\n")
                        } else {
                            format!("{tag} OK appended\r\n")
                        }
                    } else {
                        format!("{tag} NO [TRYCREATE] no such mailbox\r\n")
                    }
                };
                c.send(&reply).await?;
            }
            "IDLE" => {
                if !o.has("IDLE") {
                    c.send(&format!("{tag} BAD no IDLE\r\n")).await?;
                    continue;
                }
                c.send("+ idling\r\n").await?;
                let count = |w: &Arc<Mutex<World>>, sel: &Option<String>| {
                    sel.as_ref().and_then(|s| lock(w).mailboxes.get(s).map(|b| b.msgs.len())).unwrap_or(0)
                };
                let mut last = count(&w, &c.selected);
                loop {
                    let ready = tokio::select! {
                        r = c.r.fill_buf() => Some(r.map(|b| b.is_empty())?),
                        () = tokio::time::sleep(Duration::from_millis(30)) => None,
                    };
                    if lock(&w).generation != c.generation {
                        return Ok(());
                    }
                    match ready {
                        Some(true) => return Ok(()),
                        Some(false) => {
                            let line = c.read_line().await?.unwrap_or_default();
                            if String::from_utf8_lossy(&line).trim().eq_ignore_ascii_case("DONE") {
                                c.send(&format!("{tag} OK IDLE terminated\r\n")).await?;
                            } else {
                                c.send(&format!("{tag} BAD expected DONE\r\n")).await?;
                            }
                            break;
                        }
                        None => {
                            let now = count(&w, &c.selected);
                            if now != last {
                                last = now;
                                c.send(&format!("* {now} EXISTS\r\n")).await?;
                            }
                        }
                    }
                }
            }
            _ if c.selected.is_none() => c.send(&format!("{tag} BAD select a mailbox first\r\n")).await?,
            "UID SEARCH" | "SEARCH" => {
                let uid_mode = verb == "UID SEARCH";
                let sel = c.selected.clone().unwrap_or_default();
                let hits: Vec<u32> = {
                    let wl = lock(&w);
                    let b = wl.mbox(&sel);
                    let max_uid = b.msgs.last().map_or(0, |m| m.uid);
                    b.msgs
                        .iter()
                        .enumerate()
                        .filter(|(i, m)| {
                            let seq = u32::try_from(*i).unwrap_or(0) + 1;
                            let mut j = 0;
                            let mut ok = true;
                            while j < args.len() {
                                ok &= matches(m, seq, max_uid, &args, &mut j);
                            }
                            ok
                        })
                        .map(|(i, m)| if uid_mode { m.uid } else { u32::try_from(i).unwrap_or(0) + 1 })
                        .collect()
                };
                let list: Vec<String> = hits.iter().map(u32::to_string).collect();
                c.send(&format!(
                    "* SEARCH{}{}\r\n{tag} OK done\r\n",
                    if list.is_empty() { "" } else { " " },
                    list.join(" ")
                ))
                .await?;
            }
            "UID FETCH" | "FETCH" => {
                let uid_mode = verb == "UID FETCH";
                let sel = c.selected.clone().unwrap_or_default();
                let set = args.first().map(Tok::text).unwrap_or_default();
                let items: Vec<String> = match args.get(1) {
                    Some(Tok::List(l)) => l.iter().map(Tok::text).collect(),
                    Some(t) => match t.upper().as_str() {
                        "ALL" | "FAST" | "FULL" => vec!["FLAGS".into(), "INTERNALDATE".into(), "RFC822.SIZE".into()],
                        _ => vec![t.text()],
                    },
                    None => Vec::new(),
                };
                let mut changed_since: Option<u64> = None;
                let mut want_vanished = false;
                if let Some(Tok::List(m)) = args.get(2) {
                    let up: Vec<String> = m.iter().map(Tok::upper).collect();
                    if let Some(p) = up.iter().position(|x| x == "CHANGEDSINCE") {
                        changed_since = up.get(p + 1).and_then(|x| x.parse().ok());
                    }
                    want_vanished = up.iter().any(|x| x == "VANISHED");
                }
                let mut out: Vec<u8> = Vec::new();
                {
                    let mut wl = lock(&w);
                    let b = wl.mailboxes.get_mut(&sel).expect("selected");
                    let max = if uid_mode {
                        b.msgs.last().map_or(0, |m| m.uid)
                    } else {
                        u32::try_from(b.msgs.len()).unwrap_or(0)
                    };
                    let ranges = parse_set(&set, max);
                    if want_vanished && let Some(m) = changed_since {
                        let gone: Vec<u32> = b
                            .vanished
                            .iter()
                            .filter(|(u, ms)| *ms > m && in_set(&ranges, *u))
                            .map(|(u, _)| *u)
                            .collect();
                        if !gone.is_empty() {
                            out.extend_from_slice(format!("* VANISHED (EARLIER) {}\r\n", compress(&gone)).as_bytes());
                        }
                    }
                    for (i, m) in b.msgs.iter().enumerate() {
                        let seq = u32::try_from(i).unwrap_or(0) + 1;
                        if !in_set(&ranges, if uid_mode { m.uid } else { seq }) {
                            continue;
                        }
                        if changed_since.is_some_and(|cs| m.modseq <= cs) {
                            continue;
                        }
                        let mut parts: Vec<Vec<u8>> = Vec::new();
                        if uid_mode || items.iter().any(|x| x.eq_ignore_ascii_case("UID")) {
                            parts.push(format!("UID {}", m.uid).into_bytes());
                        }
                        for it in &items {
                            let up = it.to_ascii_uppercase();
                            match up.as_str() {
                                "UID" => {}
                                "FLAGS" => parts.push(format!("FLAGS {}", flags_str(&m.flags)).into_bytes()),
                                "INTERNALDATE" => parts.push(
                                    format!("INTERNALDATE {}", q(&format_internal_date(m.internal_date))).into_bytes(),
                                ),
                                "RFC822.SIZE" => parts.push(format!("RFC822.SIZE {}", m.raw.len()).into_bytes()),
                                "MODSEQ" => parts.push(format!("MODSEQ ({})", m.modseq).into_bytes()),
                                "X-GM-THRID" => parts.push(format!("X-GM-THRID {}", m.thrid).into_bytes()),
                                "BODYSTRUCTURE" | "BODY" => {
                                    parts.push(format!("BODYSTRUCTURE {}", bodystructure(&m.raw)).into_bytes())
                                }
                                "RFC822" => {
                                    let mut p = format!("RFC822 {{{}}}\r\n", m.raw.len()).into_bytes();
                                    p.extend_from_slice(&m.raw);
                                    parts.push(p);
                                }
                                _ if up.starts_with("BODY[") || up.starts_with("BODY.PEEK[") => {
                                    let open = it.find('[').unwrap_or(0);
                                    let close = it.rfind(']').unwrap_or(it.len());
                                    let section = &it[open + 1..close];
                                    let partial = &it[close + 1..];
                                    let mut data = section_bytes(&m.raw, section);
                                    let mut name = format!("BODY[{section}]");
                                    if let Some(p) = partial.strip_prefix('<').and_then(|p| p.strip_suffix('>')) {
                                        let (o, n) = p.split_once('.').unwrap_or((p, "4294967295"));
                                        let o: usize = o.parse().unwrap_or(0);
                                        let n: usize = n.parse().unwrap_or(usize::MAX);
                                        data = data
                                            .get(o.min(data.len())..)
                                            .unwrap_or_default()
                                            .iter()
                                            .take(n)
                                            .copied()
                                            .collect();
                                        name.push_str(&format!("<{o}>"));
                                    }
                                    let mut p = format!("{name} {{{}}}\r\n", data.len()).into_bytes();
                                    p.extend_from_slice(&data);
                                    parts.push(p);
                                }
                                _ => {}
                            }
                        }
                        if changed_since.is_some() && !items.iter().any(|x| x.eq_ignore_ascii_case("MODSEQ")) {
                            parts.push(format!("MODSEQ ({})", m.modseq).into_bytes());
                        }
                        out.extend_from_slice(format!("* {seq} FETCH (").as_bytes());
                        for (k, p) in parts.iter().enumerate() {
                            if k > 0 {
                                out.push(b' ');
                            }
                            out.extend_from_slice(p);
                        }
                        out.extend_from_slice(b")\r\n");
                    }
                }
                out.extend_from_slice(format!("{tag} OK done\r\n").as_bytes());
                c.send_bytes(&out).await?;
            }
            "UID STORE" | "STORE" => {
                let uid_mode = verb == "UID STORE";
                let sel = c.selected.clone().unwrap_or_default();
                let set = args.first().map(Tok::text).unwrap_or_default();
                let mut k = 1;
                if matches!(args.get(1), Some(Tok::List(_))) {
                    k = 2; // (UNCHANGEDSINCE n)
                }
                let op = args.get(k).map(Tok::upper).unwrap_or_default();
                let flags: Vec<String> = match args.get(k + 1) {
                    Some(Tok::List(l)) => l.iter().map(Tok::text).collect(),
                    Some(t) => vec![t.text()],
                    None => Vec::new(),
                };
                let silent = op.ends_with(".SILENT");
                let mut out = String::new();
                {
                    let mut wl = lock(&w);
                    let next = wl.modseq;
                    let mut ms = next;
                    let b = wl.mailboxes.get_mut(&sel).expect("selected");
                    let max = if uid_mode {
                        b.msgs.last().map_or(0, |m| m.uid)
                    } else {
                        u32::try_from(b.msgs.len()).unwrap_or(0)
                    };
                    let ranges = parse_set(&set, max);
                    for (i, m) in b.msgs.iter_mut().enumerate() {
                        let seq = u32::try_from(i).unwrap_or(0) + 1;
                        if !in_set(&ranges, if uid_mode { m.uid } else { seq }) {
                            continue;
                        }
                        let before = m.flags.clone();
                        if op.starts_with('+') {
                            m.flags.extend(flags.iter().cloned());
                        } else if op.starts_with('-') {
                            for f in &flags {
                                m.flags.retain(|x| !x.eq_ignore_ascii_case(f));
                            }
                        } else {
                            m.flags = flags.iter().cloned().collect();
                        }
                        if m.flags != before {
                            ms += 1;
                            m.modseq = ms;
                            b.highest_modseq = ms;
                        }
                        if !silent {
                            out.push_str(&format!("* {seq} FETCH (UID {} FLAGS {})\r\n", m.uid, flags_str(&m.flags)));
                        }
                    }
                    wl.modseq = ms;
                }
                out.push_str(&format!("{tag} OK stored\r\n"));
                c.send(&out).await?;
            }
            "UID COPY" | "UID MOVE" | "COPY" | "MOVE" => {
                let is_move = verb.ends_with("MOVE");
                if is_move && !o.has("MOVE") {
                    c.send(&format!("{tag} BAD no MOVE\r\n")).await?;
                    continue;
                }
                let uid_mode = verb.starts_with("UID");
                let sel = c.selected.clone().unwrap_or_default();
                let set = args.first().map(Tok::text).unwrap_or_default();
                let dest = utf7::decode(&args.get(1).map(Tok::text).unwrap_or_default());
                if !lock(&w).mailboxes.contains_key(&dest) {
                    c.send(&format!("{tag} NO [TRYCREATE] no such mailbox\r\n")).await?;
                    continue;
                }
                let mut out = String::new();
                {
                    let mut wl = lock(&w);
                    let b = wl.mbox(&sel);
                    let max = if uid_mode {
                        b.msgs.last().map_or(0, |m| m.uid)
                    } else {
                        u32::try_from(b.msgs.len()).unwrap_or(0)
                    };
                    let ranges = parse_set(&set, max);
                    let picked: Vec<(u32, u32, Msg)> = b
                        .msgs
                        .iter()
                        .enumerate()
                        .filter(|(i, m)| {
                            in_set(&ranges, if uid_mode { m.uid } else { u32::try_from(*i).unwrap_or(0) + 1 })
                        })
                        .map(|(i, m)| (u32::try_from(i).unwrap_or(0) + 1, m.uid, m.clone()))
                        .collect();
                    let mut src = Vec::new();
                    let mut dst = Vec::new();
                    for (_, uid, m) in &picked {
                        let flags: Vec<&str> = m.flags.iter().map(String::as_str).collect();
                        let new_uid = wl.deliver(&dest, &m.raw, &flags, m.internal_date);
                        let thrid = m.thrid;
                        wl.set_thrid(&dest, new_uid, thrid);
                        src.push(*uid);
                        dst.push(new_uid);
                    }
                    let dv = wl.mbox(&dest).validity;
                    let code = if o.has("UIDPLUS") && !src.is_empty() {
                        format!(
                            "[COPYUID {dv} {} {}] ",
                            src.iter().map(u32::to_string).collect::<Vec<_>>().join(","),
                            dst.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
                        )
                    } else {
                        String::new()
                    };
                    if is_move {
                        if !code.is_empty() {
                            out.push_str(&format!("* OK {code}moved\r\n"));
                        }
                        let mut seqs: Vec<u32> = picked.iter().map(|(s, ..)| *s).collect();
                        seqs.sort_unstable_by(|a, b| b.cmp(a));
                        for (_, uid, _) in &picked {
                            wl.expunge(&sel, *uid);
                        }
                        if c.qresync {
                            if !src.is_empty() {
                                out.push_str(&format!("* VANISHED {}\r\n", compress(&src)));
                            }
                        } else {
                            for s in seqs {
                                out.push_str(&format!("* {s} EXPUNGE\r\n"));
                            }
                        }
                        out.push_str(&format!("{tag} OK moved\r\n"));
                    } else {
                        out.push_str(&format!("{tag} OK {code}copied\r\n"));
                    }
                }
                c.send(&out).await?;
            }
            "EXPUNGE" | "UID EXPUNGE" => {
                if verb == "UID EXPUNGE" && !o.has("UIDPLUS") {
                    c.send(&format!("{tag} BAD no UIDPLUS\r\n")).await?;
                    continue;
                }
                let sel = c.selected.clone().unwrap_or_default();
                let ranges = (verb == "UID EXPUNGE")
                    .then(|| parse_set(&args.first().map(Tok::text).unwrap_or_default(), u32::MAX));
                let mut out = String::new();
                {
                    let mut wl = lock(&w);
                    let b = wl.mbox(&sel);
                    let gone: Vec<(u32, u32)> = b
                        .msgs
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| {
                            m.flags.contains("\\Deleted") && ranges.as_ref().is_none_or(|r| in_set(r, m.uid))
                        })
                        .map(|(i, m)| (u32::try_from(i).unwrap_or(0) + 1, m.uid))
                        .collect();
                    for (_, uid) in &gone {
                        wl.expunge(&sel, *uid);
                    }
                    if c.qresync {
                        let uids: Vec<u32> = gone.iter().map(|(_, u)| *u).collect();
                        if !uids.is_empty() {
                            out.push_str(&format!("* VANISHED {}\r\n", compress(&uids)));
                        }
                    } else {
                        for (s, _) in gone.iter().rev() {
                            out.push_str(&format!("* {s} EXPUNGE\r\n"));
                        }
                    }
                }
                out.push_str(&format!("{tag} OK expunged\r\n"));
                c.send(&out).await?;
            }
            "CLOSE" | "UNSELECT" => {
                c.selected = None;
                c.send(&format!("{tag} OK closed\r\n")).await?;
            }
            _ => c.send(&format!("{tag} BAD unknown command\r\n")).await?,
        }
        let _ = c.read_only;
    }
}
