//! Adapter tests against the in-process fake servers (`fake/`): no Docker, no network.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use futures::StreamExt as _;

use super::fake::imap::{FakeImap, FakeImapOptions, FakeTls, World};
use super::fake::smtp::{FakeSmtp, FakeSmtpOptions};
use super::*;
use crate::{
    AccessToken, ImapFolderCursor, LabelKind, LabelTarget, PushEvent, PushKind, PushSource, RemoteThreadRef, SysLabel,
    TokenSource,
};

const DAY: i64 = 86_400_000;

fn now() -> i64 {
    akatsuki_core::time::now_ms()
}

fn rfc2822(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms).unwrap().strftime("%a, %d %b %Y %H:%M:%S +0000").to_string()
}

fn mail(mid: &str, from: &str, subject: &str, refs: &[&str], date: i64, body: &str) -> Vec<u8> {
    let mut s = format!(
        "From: {from}\r\nTo: Jane <jane@example.org>\r\nSubject: {subject}\r\nDate: {}\r\nMessage-ID: <{mid}>\r\n",
        rfc2822(date)
    );
    if let Some(last) = refs.last() {
        let all: Vec<String> = refs.iter().map(|r| format!("<{r}>")).collect();
        s.push_str(&format!("In-Reply-To: <{last}>\r\nReferences: {}\r\n", all.join(" ")));
    }
    s.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n");
    s.push_str(body);
    s.push_str("\r\n");
    s.into_bytes()
}

fn with_attachment(mid: &str, date: i64) -> Vec<u8> {
    let text = B64.encode("Please find the report attached.\r\n\r\nOn Monday someone wrote:\r\n> old quoted text\r\n");
    let pdf = B64.encode(b"%PDF-1.4 fake pdf bytes");
    format!(
        "From: Alice <alice@example.com>\r\nTo: jane@example.org\r\nSubject: Report\r\nDate: {}\r\nMessage-ID: <{mid}>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
         --b1\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{text}\r\n\
         --b1\r\nContent-Type: application/pdf; name=\"r.pdf\"\r\nContent-Disposition: attachment; filename=\"r.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{pdf}\r\n--b1--\r\n",
        rfc2822(date)
    )
    .into_bytes()
}

fn security(t: FakeTls) -> Security {
    match t {
        FakeTls::None => Security::Plaintext,
        FakeTls::Implicit => Security::Implicit,
        FakeTls::StartTls => Security::StartTls,
    }
}

fn config(imap: &FakeImap, imap_tls: FakeTls, smtp: Option<(&FakeSmtp, FakeTls)>) -> ImapAccountConfig {
    let smtp_ep = match smtp {
        Some((s, t)) => ServerEndpoint::new("127.0.0.1", s.addr.port(), security(t)),
        None => ServerEndpoint::new("127.0.0.1", 9, Security::Plaintext),
    };
    let mut cfg = ImapAccountConfig::new(
        "acc1",
        "jane@example.org",
        ServerEndpoint::new("127.0.0.1", imap.addr.port(), security(imap_tls)),
        smtp_ep,
        Credentials::Password { username: "jane".into(), password: "secret".into() },
    );
    cfg.tls.extra_roots.extend(imap.tls.iter().map(|t| t.cert.clone()));
    // The fake servers listen on loopback (D-398).
    cfg.tls.allow_private_hosts = true;
    if let Some((s, _)) = smtp {
        cfg.tls.extra_roots.extend(s.tls.iter().map(|t| t.cert.clone()));
    }
    cfg.timeouts = Timeouts {
        connect: Duration::from_secs(5),
        command: Duration::from_secs(5),
        smtp_data_end: Duration::from_secs(5),
        idle_reissue: Duration::from_secs(2),
    };
    cfg.tuning = SyncTuning {
        flag_scan_every: Duration::ZERO,
        scan_window: Duration::from_secs(90 * 24 * 3600),
        status_sweep_every: Duration::from_millis(300),
        poll_every: Duration::from_millis(200),
        max_new_per_page: 500,
    };
    cfg.sent_auto_saved = Some(false);
    cfg
}

fn tref(t: &FetchedThread) -> RemoteThreadRef {
    RemoteThreadRef {
        provider_thread_id: t.provider_thread_id.clone(),
        messages: t
            .messages
            .iter()
            .map(|m| RemoteMessageRef { provider_message_id: m.provider_message_id.clone(), imap: m.imap.clone() })
            .collect(),
    }
}

async fn all_threads(p: &ImapProvider) -> Vec<FetchedThread> {
    let mut ids = Vec::new();
    let mut page = None;
    loop {
        let w = p.list_window(&WindowRequest::Since { after: 0, page: page.clone() }).await.unwrap();
        ids.extend(w.threads);
        match w.next_page {
            Some(n) => page = Some(n),
            None => break,
        }
    }
    p.fetch_threads(&ids, FetchDepth::Metadata).await.unwrap()
}

fn thread_with<'a>(ts: &'a [FetchedThread], subject: &str) -> &'a FetchedThread {
    ts.iter().find(|t| t.messages.iter().any(|m| m.subject == subject)).unwrap_or_else(|| panic!("no thread {subject}"))
}

async fn one(p: &ImapProvider, op: ProviderOp) -> Result<OpEffect, ProviderError> {
    p.apply(&[("op1".into(), op)]).await.remove(0).1
}

#[tokio::test]
async fn connects_over_implicit_tls_and_detects_capabilities() {
    let server = FakeImap::start(FakeImapOptions { tls: FakeTls::Implicit, ..FakeImapOptions::full() }).await;
    let p = ImapProvider::connect(config(&server, FakeTls::Implicit, None)).await.unwrap();
    let c = p.caps().imap.clone().unwrap();
    assert!(c.qresync && c.condstore && c.uid_move && c.uidplus && c.idle && c.special_use && !c.x_gm_ext);
    assert_eq!(p.caps().push, PushKind::ImapIdle);
    assert!(!p.caps().multi_label && p.caps().delete_forever && !p.caps().native_threads);
    assert!(p.capabilities().iter().any(|c| c == "QRESYNC"));
    let labels = p.list_labels().await.unwrap();
    let get = |id: &str| labels.iter().find(|l| l.provider_label_id == id).unwrap().clone();
    assert_eq!((get("INBOX").kind, get("INBOX").role), (LabelKind::System, Some(LabelRole::Inbox)));
    assert_eq!(get("Sent").role, Some(LabelRole::Sent));
    assert_eq!(get("Junk").role, Some(LabelRole::Spam));
    assert_eq!((get("Archive").kind, get("Archive").role), (LabelKind::Folder, Some(LabelRole::Archive)));
    assert!(labels.iter().all(|l| l.exclusive));
    let w = server.world();
    // AUTH=PLAIN is preferred over LOGIN; QRESYNC is enabled.
    assert!(w.saw("AUTHENTICATE") && !w.saw("LOGIN") && w.saw("ENABLE QRESYNC"));
}

#[tokio::test]
async fn starttls_login_and_folder_roles_without_extensions() {
    let server = FakeImap::start(FakeImapOptions { tls: FakeTls::StartTls, ..FakeImapOptions::minimal() }).await;
    let p = ImapProvider::connect(config(&server, FakeTls::StartTls, None)).await.unwrap();
    assert_eq!(p.caps().imap.clone().unwrap(), ImapCaps::default());
    assert!(matches!(p.caps().push, PushKind::Poll { .. }));
    let labels = p.list_labels().await.unwrap();
    let role = |id: &str| labels.iter().find(|l| l.provider_label_id == id).unwrap().role;
    // No SPECIAL-USE: roles come from the folder names.
    assert_eq!(role("Sent"), Some(LabelRole::Sent));
    assert_eq!(role("Trash"), Some(LabelRole::Trash));
    assert_eq!(role("Junk"), Some(LabelRole::Spam));
    assert_eq!(role("Archive"), Some(LabelRole::Archive));
    let w = server.world();
    assert!(w.saw("STARTTLS") && w.saw("LOGIN") && !w.saw("AUTHENTICATE"));
}

struct Tokens {
    token: String,
    invalidated: AtomicUsize,
}

#[async_trait]
impl TokenSource for Tokens {
    async fn access_token(&self) -> Result<AccessToken, ProviderError> {
        Ok(AccessToken { token: self.token.clone(), expires_at: i64::MAX })
    }
    async fn invalidate(&self) {
        self.invalidated.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn xoauth2_login_and_rejection() {
    let server = FakeImap::start(FakeImapOptions::full()).await;
    let good = Arc::new(Tokens { token: "tok".into(), invalidated: AtomicUsize::new(0) });
    let mut cfg = config(&server, FakeTls::None, None);
    cfg.credentials = Credentials::OAuth2 { username: "jane".into(), tokens: good.clone() };
    ImapProvider::connect(cfg.clone()).await.unwrap();
    let bad = Arc::new(Tokens { token: "expired".into(), invalidated: AtomicUsize::new(0) });
    cfg.credentials = Credentials::OAuth2 { username: "jane".into(), tokens: bad.clone() };
    let err = ImapProvider::connect(cfg).await.unwrap_err();
    assert_eq!(err, ProviderError::Auth { reauth_required: true });
    assert_eq!(bad.invalidated.load(Ordering::SeqCst), 1);
    assert_eq!(good.invalidated.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn connect_test_maps_failures_to_codes() {
    use akatsuki_core::wire::ErrorCode as C;
    let imap = FakeImap::start(FakeImapOptions { tls: FakeTls::Implicit, ..FakeImapOptions::full() }).await;
    let smtp = FakeSmtp::start(FakeSmtpOptions { tls: FakeTls::StartTls, ..Default::default() }).await;
    let cfg = config(&imap, FakeTls::Implicit, Some((&smtp, FakeTls::StartTls)));
    let report = test_connection(&cfg).await.unwrap();
    assert!(report.imap.qresync && !report.sent_auto_saved);
    assert!(smtp.world().log.iter().any(|l| l.starts_with("STARTTLS")));
    assert!(smtp.world().delivered.is_empty(), "the connect test never sends DATA");

    let mut bad = cfg.clone();
    bad.credentials = Credentials::Password { username: "jane".into(), password: "wrong".into() };
    assert_eq!(test_connection(&bad).await.unwrap_err().code, C::ImapAuthFailed);

    let smtp_bad = FakeSmtp::start(FakeSmtpOptions { password: "other".into(), ..Default::default() }).await;
    let cfg2 = config(&imap, FakeTls::Implicit, Some((&smtp_bad, FakeTls::None)));
    assert_eq!(test_connection(&cfg2).await.unwrap_err().code, C::SmtpAuthFailed);

    let mut untrusted = cfg.clone();
    untrusted.tls.extra_roots.clear();
    assert_eq!(test_connection(&untrusted).await.unwrap_err().code, C::TlsFailed);

    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut gone = cfg.clone();
    gone.imap.port = closed;
    assert_eq!(test_connection(&gone).await.unwrap_err().code, C::Unreachable);

    // Plaintext is refused for anything but loopback, before any connection is made.
    let mut remote = cfg.clone();
    remote.imap = ServerEndpoint::new("imap.example.invalid", 143, Security::Plaintext);
    assert_eq!(test_connection(&remote).await.unwrap_err().code, C::TlsFailed);
}

async fn seeded(opts: FakeImapOptions) -> (FakeImap, i64) {
    let server = FakeImap::start(opts).await;
    let t0 = now() - 5 * DAY;
    {
        let mut w = server.world();
        for i in 0..30 {
            let raw = mail(
                &format!("m{i}@x"),
                "Bob <bob@example.com>",
                &format!("Hello {i}"),
                &[],
                t0 + i * 1000,
                &format!("Body number {i}"),
            );
            w.deliver("INBOX", &raw, if i % 2 == 0 { &["\\Seen"] } else { &[] }, t0 + i * 1000);
        }
        let reply = mail("r1@x", "Carol <carol@example.com>", "Re: Hello 0", &["m0@x"], t0 + 40_000, "A reply");
        w.deliver("INBOX", &reply, &[], t0 + 40_000);
        w.deliver("INBOX", &with_attachment("att@x", t0 + 50_000), &["\\Flagged"], t0 + 50_000);
        let sent = mail("s1@x", "Jane <jane@example.org>", "Outgoing", &[], t0 + 60_000, "I wrote this");
        w.deliver("Sent", &sent, &["\\Seen"], t0 + 60_000);
    }
    (server, t0)
}

#[tokio::test]
async fn initial_sync_pages_snippets_attachments_and_threads() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let Cursor::Imap { folders } = p.baseline().await.unwrap() else { panic!() };
    let inbox = folders["INBOX"].clone();
    assert_eq!(inbox.uid_next, 33);
    assert!(inbox.highest_modseq.is_some());
    assert!(folders.contains_key("Sent") && folders.contains_key("Archive"));

    let first = p.list_window(&WindowRequest::Inbox { page: None }).await.unwrap();
    assert_eq!(first.threads.len(), 25);
    let second = p.list_window(&WindowRequest::Inbox { page: first.next_page.clone() }).await.unwrap();
    assert!(second.next_page.is_none());
    // 32 messages in 31 threads; the reply (newest) and its parent (oldest) land on different
    // pages, so that thread is listed twice.
    let mut ids = first.threads.clone();
    ids.extend(second.threads);
    assert_eq!(ids.len(), 32);
    ids.sort();
    ids.dedup();
    let threads = p.fetch_threads(&ids, FetchDepth::Metadata).await.unwrap();
    assert_eq!(threads.len(), 31);
    let hello0 = thread_with(&threads, "Hello 0");
    assert_eq!(hello0.messages.len(), 2, "the reply joins its parent through References");
    assert_eq!(hello0.provider_thread_id.len(), 26);
    let m0 = hello0.messages.iter().find(|m| m.subject == "Hello 0").unwrap();
    assert_eq!(m0.snippet, "Body number 0");
    assert!(!m0.sys.contains(&SysLabel::Unread) && m0.sys.contains(&SysLabel::Inbox));
    assert_eq!(m0.provider_label_ids, vec!["INBOX".to_owned()]);
    assert_eq!(m0.from.as_ref().unwrap().email, "bob@example.com");
    assert!(m0.provider_message_id.starts_with("INBOX:") && m0.imap.len() == 1);
    let report = &thread_with(&threads, "Report").messages[0];
    assert!(report.has_attachments);
    assert!(report.sys.contains(&SysLabel::Starred) && report.sys.contains(&SysLabel::Unread));
    assert_eq!(report.snippet, "Please find the report attached.", "base64 decoded, quoted history removed");
    assert!(!thread_with(&threads, "Hello 3").messages[0].has_attachments);

    // The window covers every synced folder, Sent included.
    let all = all_threads(&p).await;
    let out = thread_with(&all, "Outgoing");
    assert_eq!(out.messages[0].sys, vec![SysLabel::Sent]);
    assert_eq!(all.len(), 32);
}

#[tokio::test]
async fn delta_with_qresync() {
    let (server, t0) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let base = p.baseline().await.unwrap();
    let Cursor::Imap { folders: f0 } = &base else { panic!() };
    let v = f0["INBOX"].uid_validity;
    {
        let mut w = server.world();
        w.deliver(
            "INBOX",
            &mail("new@x", "Dan <dan@example.com>", "Fresh", &[], t0 + 90_000, "new mail"),
            &[],
            t0 + 90_000,
        );
        w.set_flags("INBOX", 2, &["\\Seen", "\\Flagged"]);
        w.expunge("INBOX", 3);
    }
    let page = p.changes_since(&base).await.unwrap();
    assert!(!page.labels_changed && !page.more);
    assert_eq!(page.deleted_messages, vec![format!("INBOX:{v}:3")]);
    let touched = p.fetch_threads(&page.touched_threads, FetchDepth::Metadata).await.unwrap();
    assert_eq!(touched.len(), 2);
    assert!(touched.iter().all(|t| t.partial), "IMAP threads are what this process saw (D-223)");
    assert!(thread_with(&touched, "Fresh").messages[0].sys.contains(&SysLabel::Unread));
    assert!(thread_with(&touched, "Hello 1").messages[0].sys.contains(&SysLabel::Starred));
    let Cursor::Imap { folders: f1 } = &page.next else { panic!() };
    assert_eq!(f1["INBOX"].uid_next, 34);
    assert!(f1["INBOX"].highest_modseq > f0["INBOX"].highest_modseq);
    assert!(server.world().saw(&format!("SELECT \"INBOX\" (QRESYNC ({v} ")));

    // Nothing changed since: an empty page, and unchanged folders are only STATUSed.
    let selects_before = server.world().log.iter().filter(|l| l.starts_with("SELECT")).count();
    let again = p.changes_since(&page.next).await.unwrap();
    assert!(again.touched_threads.is_empty() && again.deleted_messages.is_empty());
    assert_eq!(again.next, page.next);
    // Only the mailbox that is still selected is re-selected (no STATUS on a selected mailbox).
    assert_eq!(server.world().log.iter().filter(|l| l.starts_with("SELECT")).count(), selects_before + 1);
}

#[tokio::test]
async fn delta_with_condstore_only() {
    let opts = FakeImapOptions::full().with_caps(&["IDLE", "MOVE", "UIDPLUS", "CONDSTORE", "SPECIAL-USE"]);
    let (server, t0) = seeded(opts).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    assert!(!p.caps().imap.clone().unwrap().qresync);
    let base = p.baseline().await.unwrap();
    // The first pass records the expunge baseline.
    let first = p.changes_since(&base).await.unwrap();
    assert!(first.deleted_messages.is_empty());
    {
        let mut w = server.world();
        w.deliver(
            "INBOX",
            &mail("new@x", "Dan <dan@example.com>", "Fresh", &[], t0 + 90_000, "new mail"),
            &[],
            t0 + 90_000,
        );
        w.set_flags("INBOX", 2, &["\\Seen"]);
        w.expunge("INBOX", 3);
    }
    let page = p.changes_since(&first.next).await.unwrap();
    let v = server.world().mbox("INBOX").validity;
    assert_eq!(page.deleted_messages, vec![format!("INBOX:{v}:3")]);
    let touched = p.fetch_threads(&page.touched_threads, FetchDepth::Metadata).await.unwrap();
    assert!(touched.iter().any(|t| t.messages.iter().any(|m| m.subject == "Fresh")));
    let h1 = thread_with(&touched, "Hello 1");
    assert!(!h1.messages[0].sys.contains(&SysLabel::Unread));
    assert!(server.world().saw("UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE"));
}

#[tokio::test]
async fn delta_without_condstore_uses_uidnext_and_flag_scans() {
    let opts = FakeImapOptions::full().with_caps(&["UIDPLUS", "SPECIAL-USE"]);
    let (server, t0) = seeded(opts).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let base = p.baseline().await.unwrap();
    let Cursor::Imap { folders } = &base else { panic!() };
    assert_eq!(folders["INBOX"].highest_modseq, None);
    let first = p.changes_since(&base).await.unwrap();
    assert!(first.touched_threads.is_empty() && first.deleted_messages.is_empty());
    {
        let mut w = server.world();
        w.deliver(
            "INBOX",
            &mail("new@x", "Dan <dan@example.com>", "Fresh", &[], t0 + 90_000, "new mail"),
            &[],
            t0 + 90_000,
        );
        w.set_flags("INBOX", 2, &["\\Seen", "\\Flagged"]);
        w.expunge("INBOX", 5);
    }
    let page = p.changes_since(&first.next).await.unwrap();
    let v = server.world().mbox("INBOX").validity;
    assert_eq!(page.deleted_messages, vec![format!("INBOX:{v}:5")]);
    let touched = p.fetch_threads(&page.touched_threads, FetchDepth::Metadata).await.unwrap();
    assert_eq!(touched.len(), 2);
    assert!(thread_with(&touched, "Hello 1").messages[0].sys.contains(&SysLabel::Starred));
    let Cursor::Imap { folders } = &page.next else { panic!() };
    assert_eq!(folders["INBOX"].uid_next, 34);
}

#[tokio::test]
async fn paging_new_mail_sets_more() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let mut cfg = config(&server, FakeTls::None, None);
    cfg.tuning.max_new_per_page = 10;
    let p = ImapProvider::connect(cfg).await.unwrap();
    // A cursor from before anything arrived in INBOX.
    let mut base = p.baseline().await.unwrap();
    if let Cursor::Imap { folders } = &mut base {
        let e = folders.get_mut("INBOX").unwrap();
        *e = ImapFolderCursor { uid_next: 1, ..e.clone() };
    }
    let mut cursor = base;
    let mut pages = 0;
    let mut touched = 0;
    loop {
        let page = p.changes_since(&cursor).await.unwrap();
        touched += page.touched_threads.len();
        pages += 1;
        cursor = page.next;
        if !page.more {
            break;
        }
    }
    assert_eq!(pages, 4);
    assert_eq!(touched, 32, "32 messages in 31 threads; the reply's thread is reported twice across pages");
}

#[tokio::test]
async fn uidvalidity_change_expires_the_cursor() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let base = p.baseline().await.unwrap();
    let page1 = p.list_window(&WindowRequest::Inbox { page: None }).await.unwrap();
    server.world().reset_validity("INBOX");
    assert_eq!(p.changes_since(&base).await.unwrap_err(), ProviderError::CursorExpired);
    let err = p.list_window(&WindowRequest::Inbox { page: page1.next_page }).await.unwrap_err();
    assert_eq!(err, ProviderError::CursorExpired);
    // A new baseline works again.
    let fresh = p.baseline().await.unwrap();
    assert!(p.changes_since(&fresh).await.is_ok());
}

#[tokio::test]
async fn flag_ops_moves_trash_and_delete_with_move_extension() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let threads = all_threads(&p).await;
    let t = tref(thread_with(&threads, "Hello 1"));
    let uid = t.messages[0].imap[0].uid;

    let eff = one(
        &p,
        ProviderOp::Modify {
            thread: t.clone(),
            add: vec![
                LabelTarget::Sys(SysLabel::Starred),
                LabelTarget::Label { provider_label_id: "keyword:ProjectX".into(), exclusive: false },
            ],
            remove: vec![LabelTarget::Sys(SysLabel::Unread)],
        },
    )
    .await
    .unwrap();
    let flags = server.world().msg("INBOX", uid).unwrap().flags.clone();
    assert!(flags.contains("\\Flagged") && flags.contains("\\Seen") && flags.contains("ProjectX"));
    let OpEffect::Applied { post: Some(post), .. } = eff else { panic!("post state") };
    assert!(!post.partial, "an op post-state covers the op's whole thread ref (D-223)");
    let m = &post.messages[0];
    assert!(m.sys.contains(&SysLabel::Starred) && !m.sys.contains(&SysLabel::Unread));
    assert!(m.provider_label_ids.contains(&"keyword:ProjectX".to_owned()));
    // Keywords become labels once seen in INBOX FLAGS (PERMANENTFLAGS has \*).
    let labels = p.list_labels().await.unwrap();
    assert!(
        labels
            .iter()
            .any(|l| l.provider_label_id == "keyword:ProjectX" && l.kind == LabelKind::Keyword && !l.exclusive)
    );

    // Archive = move to the role-archive folder; COPYUID rewrites the message id.
    let eff = one(
        &p,
        ProviderOp::Move {
            thread: t.clone(),
            to: LabelTarget::Label { provider_label_id: "Archive".into(), exclusive: true },
        },
    )
    .await
    .unwrap();
    let OpEffect::Applied { post: Some(post), .. } = eff else { panic!() };
    let moved = &post.messages[0];
    assert!(moved.provider_message_id.starts_with("Archive:"), "{}", moved.provider_message_id);
    assert_eq!(post.provider_thread_id, t.provider_thread_id);
    assert!(server.world().msg("INBOX", uid).is_none());
    assert_eq!(server.world().mbox("Archive").msgs.len(), 1);
    assert!(server.world().saw("UID MOVE"));

    // Trash, untrash, delete forever.
    let t2 = tref(&post);
    let OpEffect::Applied { post: Some(post), .. } = one(&p, ProviderOp::Trash { thread: t2 }).await.unwrap() else {
        panic!()
    };
    assert!(post.messages[0].sys.contains(&SysLabel::Trash));
    let OpEffect::Applied { post: Some(post), .. } =
        one(&p, ProviderOp::Untrash { thread: tref(&post) }).await.unwrap()
    else {
        panic!()
    };
    assert!(post.messages[0].sys.contains(&SysLabel::Inbox));
    let gone = tref(&post);
    assert!(matches!(
        one(&p, ProviderOp::DeleteForever { thread: gone.clone() }).await.unwrap(),
        OpEffect::Applied { .. }
    ));
    assert_eq!(server.world().mbox("INBOX").msgs.len(), 31);
    // The UID is gone: NotFound (the engine completes the op as moot).
    assert_eq!(one(&p, ProviderOp::DeleteForever { thread: gone }).await.unwrap_err(), ProviderError::NotFound);

    // Removing a folder without a destination is not an IMAP operation.
    let t3 = tref(thread_with(&threads, "Hello 4"));
    let err = one(&p, ProviderOp::Modify { thread: t3, add: vec![], remove: vec![LabelTarget::Sys(SysLabel::Inbox)] })
        .await
        .unwrap_err();
    assert!(matches!(err, ProviderError::Invalid(_)));
}

#[tokio::test]
async fn move_falls_back_to_copy_and_expunge() {
    // UIDPLUS but no MOVE: UID COPY + \Deleted + UID EXPUNGE, ids from COPYUID.
    let (server, _) = seeded(FakeImapOptions::full().with_caps(&["UIDPLUS", "SPECIAL-USE"])).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let threads = all_threads(&p).await;
    let t = tref(thread_with(&threads, "Hello 7"));
    let OpEffect::Applied { post: Some(post), .. } =
        one(&p, ProviderOp::Move { thread: t, to: LabelTarget::Sys(SysLabel::Spam) }).await.unwrap()
    else {
        panic!()
    };
    assert!(post.messages[0].provider_message_id.starts_with("Junk:"));
    assert!(post.messages[0].sys.contains(&SysLabel::Spam));
    {
        let w = server.world();
        assert!(w.saw("UID COPY") && w.saw("UID EXPUNGE") && !w.saw("UID MOVE"));
    }

    // Neither MOVE nor UIDPLUS: COPY + EXPUNGE, the copy is found by Message-ID.
    let (server, _) = seeded(FakeImapOptions::minimal()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let threads = all_threads(&p).await;
    let t = tref(thread_with(&threads, "Hello 8"));
    let OpEffect::Applied { post: Some(post), .. } = one(
        &p,
        ProviderOp::Move { thread: t, to: LabelTarget::Label { provider_label_id: "Archive".into(), exclusive: true } },
    )
    .await
    .unwrap() else {
        panic!()
    };
    assert!(post.messages[0].provider_message_id.starts_with("Archive:"));
    assert_eq!(post.messages[0].rfc_message_id.as_deref(), Some("m8@x"));
    let w = server.world();
    assert!(w.saw("UID COPY") && w.saw("EXPUNGE") && !w.saw("UID EXPUNGE"));
    assert!(w.saw("UID SEARCH HEADER Message-ID"));
    assert_eq!(w.mbox("INBOX").msgs.len(), 31);
}

#[tokio::test]
async fn label_ops_and_snooze_folder() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let OpEffect::Applied { created: Some(l), .. } =
        one(&p, ProviderOp::LabelCreate { name: "Projects/2026".into(), color: None }).await.unwrap()
    else {
        panic!()
    };
    assert_eq!((l.provider_label_id.as_str(), l.kind, l.exclusive), ("Projects/2026", LabelKind::Folder, true));
    // A replayed create is not an error.
    assert!(one(&p, ProviderOp::LabelCreate { name: "Projects/2026".into(), color: None }).await.is_ok());
    let OpEffect::Applied { created: Some(r), .. } =
        one(&p, ProviderOp::LabelRename { provider_label_id: "Projects/2026".into(), name: "Projects/Done".into() })
            .await
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(r.provider_label_id, "Projects/Done");
    assert!(matches!(
        one(&p, ProviderOp::LabelDelete { provider_label_id: "Projects/Done".into() }).await.unwrap(),
        OpEffect::Applied { .. }
    ));
    assert_eq!(
        one(&p, ProviderOp::LabelDelete { provider_label_id: "Projects/Done".into() }).await.unwrap(),
        OpEffect::Moot
    );

    let snoozed = p.ensure_label(LabelRole::Snoozed, crate::labels::SNOOZED_LABEL_NAME).await.unwrap();
    assert_eq!((snoozed.provider_label_id.as_str(), snoozed.role), ("Akatsuki/Snoozed", Some(LabelRole::Snoozed)));
    let threads = all_threads(&p).await;
    let t = tref(thread_with(&threads, "Hello 5"));
    let OpEffect::Applied { post: Some(post), .. } =
        one(&p, ProviderOp::Move { thread: t, to: LabelTarget::Sys(SysLabel::Snoozed) }).await.unwrap()
    else {
        panic!()
    };
    assert!(post.messages[0].sys.contains(&SysLabel::Snoozed));
    assert_eq!(server.world().mbox("Akatsuki/Snoozed").msgs.len(), 1);
    // ensure_label is idempotent.
    assert_eq!(
        p.ensure_label(LabelRole::Snoozed, "Akatsuki/Snoozed").await.unwrap().provider_label_id,
        "Akatsuki/Snoozed"
    );
}

fn draft(mid: &str, bcc: bool) -> OutgoingMime {
    let mut s = format!(
        "From: Jane <jane@example.org>\r\nTo: Bob <bob@example.com>\r\nCc: carol@example.com\r\n{}Subject: Lunch\r\nDate: {}\r\nMessage-ID: <{mid}>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n.leading dot line\r\nSee you\r\n",
        if bcc { "Bcc: secret@example.net\r\n" } else { "" },
        rfc2822(now())
    );
    s.push_str("bye\r\n");
    OutgoingMime { rfc_message_id: mid.into(), bytes: s.into_bytes().into(), thread: None }
}

#[tokio::test]
async fn drafts_send_and_find_sent() {
    let imap = FakeImap::start(FakeImapOptions::full()).await;
    let smtp = FakeSmtp::start(FakeSmtpOptions::default()).await;
    let p = ImapProvider::connect(config(&imap, FakeTls::None, Some((&smtp, FakeTls::None)))).await.unwrap();
    let mid = "0192aaaa-bbbb@example.org";
    let d1 = p.save_draft(&draft(mid, true), None).await.unwrap();
    assert!(d1.provider_draft_id.starts_with("Drafts:"));
    let d2 = p.save_draft(&draft(mid, true), Some(&d1)).await.unwrap();
    assert_ne!(d1, d2);
    {
        let w = imap.world();
        assert_eq!(w.mbox("Drafts").msgs.len(), 1, "the previous version is expunged");
        assert!(w.mbox("Drafts").msgs[0].flags.contains("\\Draft"));
    }
    assert_eq!(p.find_sent(mid).await.unwrap(), None);
    assert!(matches!(p.send_draft(&d2, "other@example.org").await.unwrap_err(), ProviderError::Conflict(_)));

    let sent = p.send_draft(&d2, mid).await.unwrap();
    assert!(sent.provider_message_id.starts_with("Sent:"), "{}", sent.provider_message_id);
    assert!(sent.provider_thread_id.is_some());
    let delivered = smtp.world().delivered.clone();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].from, "jane@example.org");
    assert_eq!(delivered[0].rcpts, vec!["bob@example.com", "carol@example.com", "secret@example.net"]);
    let data = String::from_utf8_lossy(&delivered[0].data).to_string();
    assert!(!data.contains("Bcc:") && !data.contains("secret@example.net"), "Bcc never reaches the wire");
    assert!(data.contains("\r\n.leading dot line\r\n"), "dot-stuffing round trips");
    {
        let w = imap.world();
        assert!(w.mbox("Drafts").msgs.is_empty(), "the draft is removed after sending");
        let copy = &w.mbox("Sent").msgs[0];
        assert!(String::from_utf8_lossy(&copy.raw).contains("Bcc: secret@example.net"), "the Sent copy keeps Bcc");
        assert!(copy.flags.contains("\\Seen"));
    }
    let found = p.find_sent(mid).await.unwrap().unwrap();
    assert_eq!(found.provider_message_id, sent.provider_message_id);
    // The draft is gone: sending again finds nothing to send.
    assert_eq!(p.send_draft(&d2, mid).await.unwrap_err(), ProviderError::NotFound);
    assert_eq!(p.delete_draft(&d2).await.unwrap_err(), ProviderError::NotFound);
}

#[tokio::test]
async fn sent_auto_saved_skips_the_sent_append() {
    let imap = FakeImap::start(FakeImapOptions::full()).await;
    let smtp =
        FakeSmtp::start(FakeSmtpOptions { tls: FakeTls::Implicit, mechs: vec!["LOGIN"], ..Default::default() }).await;
    let mut cfg = config(&imap, FakeTls::None, Some((&smtp, FakeTls::Implicit)));
    cfg.sent_auto_saved = Some(true);
    let p = ImapProvider::connect(cfg).await.unwrap();
    assert!(p.caps().sent_auto_saved);
    let d = p.save_draft(&draft("auto@example.org", false), None).await.unwrap();
    let sent = p.send_draft(&d, "auto@example.org").await.unwrap();
    assert_eq!(sent.provider_message_id, "<auto@example.org>");
    assert!(imap.world().mbox("Sent").msgs.is_empty());
    assert!(smtp.world().log.iter().any(|l| l == "AUTH LOGIN"));
    assert_eq!(smtp.world().delivered.len(), 1);
}

#[tokio::test]
async fn send_failures_are_classified() {
    let imap = FakeImap::start(FakeImapOptions::full()).await;
    let reject =
        FakeSmtp::start(FakeSmtpOptions { reject_rcpt: vec!["carol@example.com".into()], ..Default::default() }).await;
    let p = ImapProvider::connect(config(&imap, FakeTls::None, Some((&reject, FakeTls::None)))).await.unwrap();
    let d = p.save_draft(&draft("f1@example.org", false), None).await.unwrap();
    let err = p.send_draft(&d, "f1@example.org").await.unwrap_err();
    assert!(matches!(err, ProviderError::Invalid(_)) && !is_ambiguous_send(&err));
    assert!(reject.world().delivered.is_empty());
    assert_eq!(imap.world().mbox("Drafts").msgs.len(), 1, "a failed send keeps the draft");

    let dropper = FakeSmtp::start(FakeSmtpOptions { drop_after_data: true, ..Default::default() }).await;
    let p = ImapProvider::connect(config(&imap, FakeTls::None, Some((&dropper, FakeTls::None)))).await.unwrap();
    let err = p.send_draft(&d, "f1@example.org").await.unwrap_err();
    assert!(is_ambiguous_send(&err), "{err:?}");
    assert!(err.retryable());

    let temp = FakeSmtp::start(FakeSmtpOptions { data_reply: 451, ..Default::default() }).await;
    let p = ImapProvider::connect(config(&imap, FakeTls::None, Some((&temp, FakeTls::None)))).await.unwrap();
    let err = p.send_draft(&d, "f1@example.org").await.unwrap_err();
    assert!(matches!(err, ProviderError::Transient(..)) && !is_ambiguous_send(&err));

    let auth = FakeSmtp::start(FakeSmtpOptions { password: "nope".into(), ..Default::default() }).await;
    let p = ImapProvider::connect(config(&imap, FakeTls::None, Some((&auth, FakeTls::None)))).await.unwrap();
    assert_eq!(p.send_draft(&d, "f1@example.org").await.unwrap_err(), ProviderError::Auth { reauth_required: true });
}

#[tokio::test]
async fn bodies_and_parts() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let threads = all_threads(&p).await;
    let m = &thread_with(&threads, "Report").messages[0];
    let r = RemoteMessageRef { provider_message_id: m.provider_message_id.clone(), imap: m.imap.clone() };
    let raw: Vec<u8> = p.fetch_raw(&r).await.unwrap().map(|c| c.unwrap().to_vec()).concat().await;
    assert_eq!(raw, server.world().msg("INBOX", m.imap[0].uid).unwrap().raw);
    let pdf: Vec<u8> = p.fetch_part(&r, "2").await.unwrap().map(|c| c.unwrap().to_vec()).concat().await;
    assert_eq!(pdf, b"%PDF-1.4 fake pdf bytes");
    let text: Vec<u8> = p.fetch_part(&r, "1").await.unwrap().map(|c| c.unwrap().to_vec()).concat().await;
    assert!(String::from_utf8_lossy(&text).starts_with("Please find the report attached."));
    assert!(matches!(p.fetch_part(&r, "1]<0.10>").await.err().unwrap(), ProviderError::Invalid(_)));
    // The provider id alone is enough.
    let by_id = RemoteMessageRef { provider_message_id: m.provider_message_id.clone(), imap: vec![] };
    assert!(p.fetch_raw(&by_id).await.is_ok());
    let stale = RemoteMessageRef { provider_message_id: "INBOX:1:1".into(), imap: vec![] };
    assert_eq!(p.fetch_raw(&stale).await.err().unwrap(), ProviderError::NotFound);
}

#[tokio::test]
async fn provider_search() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let q = |s: &str| akatsuki_search::parse(s, now(), "UTC").ast;
    let (ids, next) = p.search(&q("from:alice"), None).await.unwrap();
    assert_eq!(ids.len(), 1);
    assert!(next.is_none());
    let t = p.fetch_threads(&ids, FetchDepth::Metadata).await.unwrap();
    assert_eq!(t[0].messages[0].subject, "Report");
    let (ids, _) = p.search(&q("in:sent outgoing"), None).await.unwrap();
    assert_eq!(ids.len(), 1);
    let (ids, next) = p.search(&q("hello"), None).await.unwrap();
    assert_eq!(ids.len(), 30, "31 hits (30 + the reply) in 30 threads");
    assert!(next.is_none());
    let (ids, _) = p.search(&q("is:starred"), None).await.unwrap();
    assert_eq!(ids.len(), 1);
}

#[tokio::test]
async fn backfill_by_date() {
    let server = FakeImap::start(FakeImapOptions::full()).await;
    let n = now();
    for (i, age) in [10, 20, 40, 400].iter().enumerate() {
        let d = n - age * DAY;
        server.world().deliver(
            "INBOX",
            &mail(&format!("b{i}@x"), "Old <old@example.com>", &format!("Old {age}"), &[], d, "old"),
            &[],
            d,
        );
    }
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let page = p
        .list_window(&WindowRequest::Before { before: n - 15 * DAY, horizon: n - 365 * DAY, limit: 10 })
        .await
        .unwrap();
    let threads = p.fetch_threads(&page.threads, FetchDepth::Metadata).await.unwrap();
    let mut subjects: Vec<String> = threads.iter().map(|t| t.messages[0].subject.clone()).collect();
    subjects.sort();
    assert_eq!(subjects, vec!["Old 20", "Old 40"]);
    // INTERNALDATE has whole seconds.
    assert_eq!(page.oldest, Some((n - 40 * DAY) / 1000 * 1000));
    let page =
        p.list_window(&WindowRequest::Before { before: n - 15 * DAY, horizon: n - 365 * DAY, limit: 1 }).await.unwrap();
    assert_eq!(page.threads.len(), 1);
    assert_eq!(page.oldest, Some((n - 20 * DAY) / 1000 * 1000));
}

#[tokio::test]
async fn gmail_thread_ids_and_lookup_without_cache() {
    let server =
        FakeImap::start(FakeImapOptions::full().with_caps(&["IDLE", "MOVE", "UIDPLUS", "SPECIAL-USE", "X-GM-EXT-1"]))
            .await;
    let d = now() - DAY;
    {
        let mut w = server.world();
        let a = w.deliver("INBOX", &mail("g1@x", "A <a@example.com>", "Gmail", &[], d, "one"), &[], d);
        let b = w.deliver("INBOX", &mail("g2@x", "B <b@example.com>", "Unrelated subject", &[], d, "two"), &[], d);
        w.set_thrid("INBOX", a, 0xabc);
        w.set_thrid("INBOX", b, 0xabc);
    }
    let mut cfg = config(&server, FakeTls::None, None);
    cfg.sent_auto_saved = None;
    let p = ImapProvider::connect(cfg.clone()).await.unwrap();
    assert!(
        p.caps().native_threads && p.caps().sent_auto_saved,
        "X-GM-EXT-1 implies Gmail, which files sent mail itself"
    );
    let page = p.list_window(&WindowRequest::Inbox { page: None }).await.unwrap();
    assert_eq!(page.threads, vec![RemoteThreadId("abc".into())]);
    // A fresh adapter has no index: X-GM-THRID search finds the thread.
    let fresh = ImapProvider::connect(cfg).await.unwrap();
    let t = fresh.fetch_threads(&[RemoteThreadId("abc".into())], FetchDepth::Metadata).await.unwrap();
    assert_eq!(t[0].messages.len(), 2);
    assert!(t[0].partial, "synced folders only");
}

#[tokio::test]
async fn non_ascii_folder_names_round_trip() {
    let server = FakeImap::start(FakeImapOptions::full()).await;
    server.world().add_mailbox("Entwürfe", &[]);
    let d = now();
    server.world().deliver("Entwürfe", &mail("u1@x", "A <a@example.com>", "Grüße", &[], d, "Hallo"), &[], d);
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let labels = p.list_labels().await.unwrap();
    assert!(labels.iter().any(|l| l.provider_label_id == "Entwürfe"));
    let all = all_threads(&p).await;
    let m = &thread_with(&all, "Grüße").messages[0];
    assert!(m.provider_message_id.starts_with("Entwürfe:"));
    assert!(server.world().saw("SELECT \"Entw&APw-rfe\""));
}

#[tokio::test]
async fn server_errors_are_classified_and_connections_recover() {
    let (server, _) = seeded(FakeImapOptions::full()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let threads = all_threads(&p).await;
    let t = tref(thread_with(&threads, "Hello 9"));
    server.world().fail_next("UID STORE", "NO [UNAVAILABLE] busy");
    let err = one(
        &p,
        ProviderOp::Modify { thread: t.clone(), add: vec![LabelTarget::Sys(SysLabel::Starred)], remove: vec![] },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ProviderError::Transient(..)) && err.retryable());
    server.world().fail_next("UID STORE", "NO [LIMIT] slow down");
    let err = one(
        &p,
        ProviderOp::Modify { thread: t.clone(), add: vec![LabelTarget::Sys(SysLabel::Starred)], remove: vec![] },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ProviderError::RateLimited { .. }));
    server.world().fail_next("APPEND", "NO [OVERQUOTA] mailbox full");
    assert!(matches!(p.save_draft(&draft("q@example.org", false), None).await.unwrap_err(), ProviderError::Quota(_)));

    // A dropped connection is transient, and the next call reconnects.
    server.world().fail_next("UID STORE", "DROP");
    let ops = vec![
        (
            "a".to_owned(),
            ProviderOp::Modify { thread: t.clone(), add: vec![LabelTarget::Sys(SysLabel::Starred)], remove: vec![] },
        ),
        (
            "b".to_owned(),
            ProviderOp::Modify { thread: t.clone(), add: vec![LabelTarget::Sys(SysLabel::Starred)], remove: vec![] },
        ),
    ];
    let res = p.apply(&ops).await;
    assert!(matches!(res[0].1, Err(ProviderError::Transient(..))));
    assert!(res[1].1.is_ok(), "the second op runs on a new connection: {:?}", res[1].1);
    assert!(server.world().msg("INBOX", t.messages[0].imap[0].uid).unwrap().flags.contains("\\Flagged"));
}

#[tokio::test]
async fn idle_reports_inbox_and_folder_changes_and_reconnects() {
    let server = FakeImap::start(FakeImapOptions::full()).await;
    let cfg = config(&server, FakeTls::None, None);
    let p = ImapProvider::connect(cfg).await.unwrap();
    let mut idle = p.idle_source();
    let world = server.world.clone();
    let deliver = |w: &Arc<std::sync::Mutex<World>>, mbox: &str| {
        let d = now();
        w.lock().unwrap().deliver(mbox, &mail("i@x", "A <a@example.com>", "Idle", &[], d, "ping"), &[], d);
    };
    // Connect first (the first next() connects and starts idling).
    let w2 = world.clone();
    let h = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        deliver(&w2, "INBOX");
    });
    let ev = tokio::time::timeout(Duration::from_secs(5), idle.next()).await.unwrap().unwrap();
    assert_eq!(ev, PushEvent::FolderChanged { remote_folder_id: "INBOX".into() });
    h.await.unwrap();
    assert!(server.world().saw("IDLE"));

    // Another folder: found by the STATUS sweep.
    deliver(&world, "Archive");
    let ev = tokio::time::timeout(Duration::from_secs(5), idle.next()).await.unwrap().unwrap();
    assert_eq!(ev, PushEvent::FolderChanged { remote_folder_id: "Archive".into() });

    // The server drops the connection: an error, then `Reconnected`.
    server.kick();
    let r = tokio::time::timeout(Duration::from_secs(5), idle.next()).await.unwrap();
    assert!(r.is_err());
    let ev = tokio::time::timeout(Duration::from_secs(10), idle.next()).await.unwrap().unwrap();
    assert_eq!(ev, PushEvent::Reconnected);
}

#[tokio::test]
async fn polling_without_idle() {
    let server = FakeImap::start(FakeImapOptions::minimal()).await;
    let p = ImapProvider::connect(config(&server, FakeTls::None, None)).await.unwrap();
    let mut idle = p.idle_source();
    let w = server.world.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let d = now();
        w.lock().unwrap().deliver("INBOX", &mail("p@x", "A <a@example.com>", "Poll", &[], d, "ping"), &[], d);
    });
    let ev = tokio::time::timeout(Duration::from_secs(5), idle.next()).await.unwrap().unwrap();
    assert_eq!(ev, PushEvent::FolderChanged { remote_folder_id: "INBOX".into() });
    assert!(!server.world().saw("IDLE"));
}
