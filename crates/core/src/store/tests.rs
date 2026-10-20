//! Mirror tests: feed apply, idempotency, tombstones, rev gaps, resets, the overlay (optimistic
//! actions, ack, rollback, undo), folder providers, splits and the device caches.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::*;
use crate::convex::ThreadAction;
use crate::convex::client::SyncChangesResult;

const A: &str = "acc1";
const B: &str = "acc2";

fn account(id: &str, provider: &str, email: &str) -> MailAccountView {
    serde_json::from_value(json!({
        "_id": id, "_creationTime": 1.0, "provider": provider, "providerAccountId": email, "email": email,
        "sortOrder": 0, "status": "live", "syncPhase": "complete", "progress": {"threadsDone": 0},
        "windowStartAt": 0, "backfillHorizonAt": 0,
        "caps": {"multiLabel": provider == "gmail", "categories": false, "serverSearch": true, "deleteForever": provider != "gmail"},
        "createdAt": 1
    }))
    .unwrap()
}

fn thread(id: &str, account: &str, rev: u64, sort_at: i64, keys: &[&str]) -> Value {
    json!({
        "_id": id, "_creationTime": 1.0, "accountId": account, "providerThreadId": format!("p-{id}"),
        "subject": format!("Subject {id}"), "snippet": "snippet", "participants": [{"email": "ada@example.com", "name": "Ada"}],
        "lastFrom": {"email": "ada@example.com", "name": "Ada"}, "isBulk": false, "lastMessageAt": sort_at,
        "sortAt": sort_at, "messageCount": 1, "unreadCount": u32::from(keys.contains(&"sys:unread")), "draftCount": 0,
        "hasAttachments": false, "syncedLabelKeys": keys, "labelKeys": keys, "pendingOpCount": 0,
        "deletedPending": false, "triageState": "none", "rev": rev, "updatedAt": sort_at
    })
}

fn with(mut v: Value, key: &str, value: Value) -> Value {
    v[key] = value;
    v
}

fn message(id: &str, thread: &str, account: &str, date: i64) -> Value {
    json!({
        "_id": id, "_creationTime": 1.0, "accountId": account, "threadId": thread, "providerMessageId": format!("pm-{id}"),
        "references": [], "from": {"email": "ada@example.com"}, "to": [{"email": "me@example.com"}], "cc": [], "bcc": [],
        "replyTo": [], "subject": "Hi", "snippet": "snippet", "date": date, "labelKeys": ["sys:inbox"], "isUnread": false,
        "isDraft": false, "isOutbound": false, "hasAttachments": false
    })
}

fn label(id: &str, account: &str, name: &str, exclusive: bool, role: Option<&str>) -> Value {
    let key = match role {
        Some(r) if r != "archive" => format!("sys:{r}"),
        _ => format!("l:{id}"),
    };
    let mut v = json!({
        "_id": id, "_creationTime": 1.0, "accountId": account, "providerLabelId": format!("pl-{id}"), "name": name,
        "kind": if exclusive { "folder" } else { "user" }, "exclusive": exclusive, "hidden": false,
        "pendingDelete": false, "labelKey": key
    });
    if let Some(r) = role {
        v["role"] = json!(r);
    }
    v
}

fn batch(
    threads: Vec<Value>,
    messages: Vec<Value>,
    labels: Vec<Value>,
    tombstones: Vec<Value>,
    up_to: u64,
) -> SyncChangesResult {
    serde_json::from_value(json!({
        "threads": threads, "messages": messages, "labels": labels, "tombstones": tombstones,
        "upToRev": up_to, "hasMore": false, "resetRequired": false
    }))
    .unwrap()
}

fn store() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.set_accounts(&[account(A, "gmail", "me@example.com"), account(B, "graph", "me@outlook.example")]).unwrap();
    s
}

fn inbox() -> ListQuery {
    ListQuery { label_key: Some("sys:inbox".into()), ..ListQuery::default() }
}

fn archive() -> ThreadAction {
    ThreadAction::Modify { add: vec![], remove: vec!["sys:inbox".into()] }
}

fn revs(account: &str, rev: u64) -> BTreeMap<String, u64> {
    BTreeMap::from([(account.to_owned(), rev)])
}

#[test]
fn applies_a_batch_and_lists_newest_first() {
    let mut s = store();
    let out = s
        .apply_changes(
            A,
            0,
            &batch(
                vec![thread("t1", A, 1, 100, &["sys:inbox"]), thread("t2", A, 2, 200, &["sys:inbox", "sys:unread"])],
                vec![message("m1", "t1", A, 100), message("m2", "t2", A, 200), message("m3", "t2", A, 150)],
                vec![label("l1", A, "Work", false, None)],
                vec![],
                2,
            ),
        )
        .unwrap();
    assert_eq!(out, FeedOutcome::Applied { rev: 2, has_more: false, threads: vec!["t1".into(), "t2".into()] });
    assert_eq!(s.account_rev(A).unwrap(), 2);
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t2", "t1"]);
    let rows = s.summaries(&s.list_ids(&inbox()).unwrap()).unwrap();
    assert!(rows[0].unread && !rows[1].unread);
    assert_eq!(rows[0].subject, "Subject t2");
    let msgs = s.messages("t2").unwrap();
    assert_eq!(msgs.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["m3", "m2"], "oldest first");
    assert_eq!(s.labels(Some(A)).unwrap()[0].name, "Work");
    assert_eq!(s.list_ids(&ListQuery::default()).unwrap(), vec!["t2", "t1"], "all mail");
    assert_eq!(s.list_ids(&ListQuery { unread_only: true, ..inbox() }).unwrap(), vec!["t2"]);
    assert_eq!(s.unread_counts(&["sys:inbox".into()], None, 100).unwrap()["sys:inbox"], 1);
}

#[test]
fn replays_are_stale_and_older_thread_revs_never_win() {
    let mut s = store();
    let b1 = batch(vec![thread("t1", A, 3, 100, &["sys:inbox"])], vec![], vec![], vec![], 3);
    s.apply_changes(A, 0, &b1).unwrap();
    assert_eq!(s.apply_changes(A, 0, &b1).unwrap(), FeedOutcome::Stale { rev: 3 });
    // An overlapping batch carrying an older copy of t1 (rev 2) and a new thread.
    let b2 = batch(
        vec![thread("t1", A, 2, 100, &["sys:trash"]), thread("t2", A, 4, 50, &["sys:inbox"])],
        vec![],
        vec![],
        vec![],
        4,
    );
    assert!(matches!(s.apply_changes(A, 2, &b2).unwrap(), FeedOutcome::Applied { rev: 4, .. }));
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t1", "t2"], "t1 kept its newer state");
}

#[test]
fn a_batch_past_the_local_rev_is_a_gap_and_changes_nothing() {
    let mut s = store();
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 5)).unwrap();
    let gap = batch(vec![thread("t9", A, 9, 900, &["sys:inbox"])], vec![], vec![], vec![], 9);
    assert_eq!(s.apply_changes(A, 7, &gap).unwrap(), FeedOutcome::Gap { local_rev: 5 });
    assert_eq!(s.account_rev(A).unwrap(), 5);
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t1"]);
}

#[test]
fn tombstones_delete_threads_messages_and_labels() {
    let mut s = store();
    s.apply_changes(
        A,
        0,
        &batch(
            vec![thread("t1", A, 1, 100, &["sys:inbox", "l:l1"]), thread("t2", A, 1, 200, &["sys:inbox"])],
            vec![message("m1", "t1", A, 100), message("m2", "t2", A, 200), message("m3", "t2", A, 210)],
            vec![label("l1", A, "Work", false, None)],
            vec![],
            1,
        ),
    )
    .unwrap();
    s.put_body("m3", 1, Some("<p>x</p>"), Some("x"), None, u64::MAX).unwrap();
    s.add_overlay("op1", &["t1".into()], &archive()).unwrap();
    let out = s
        .apply_changes(
            A,
            1,
            &batch(
                vec![],
                vec![],
                vec![],
                vec![
                    json!({"table": "threads", "docId": "t1"}),
                    json!({"table": "messages", "docId": "m3"}),
                    json!({"table": "labels", "docId": "l1"}),
                ],
                2,
            ),
        )
        .unwrap();
    assert!(matches!(out, FeedOutcome::Applied { rev: 2, .. }));
    assert!(s.thread("t1").unwrap().is_none());
    assert!(s.messages("t1").unwrap().is_empty());
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t2"]);
    assert!(s.list_ids(&ListQuery { label_key: Some("l:l1".into()), ..ListQuery::default() }).unwrap().is_empty());
    assert_eq!(s.messages("t2").unwrap().len(), 1);
    assert!(s.body("m3", 1).unwrap().is_none(), "a deleted message's body is dropped");
    assert!(s.labels(Some(A)).unwrap().is_empty());
    assert!(s.overlay().unwrap().is_empty(), "overlay rows of a deleted thread go too");
}

#[test]
fn reset_wipes_the_account_but_keeps_local_intent() {
    let mut s = store();
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 4)).unwrap();
    s.apply_changes(B, 0, &batch(vec![thread("u1", B, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 2)).unwrap();
    s.add_overlay("op1", &["t1".into()], &archive()).unwrap();
    s.outbox_push("op1", "mail:apply", &json!({"clientOpId": "op1"})).unwrap();
    let mut reset = batch(vec![], vec![], vec![], vec![], 0);
    reset.reset_required = true;
    assert_eq!(s.apply_changes(A, 4, &reset).unwrap(), FeedOutcome::Reset);
    assert_eq!(s.account_rev(A).unwrap(), 0);
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["u1"], "the other account is untouched");
    assert_eq!(s.outbox().unwrap().len(), 1);
    assert_eq!(s.overlay().unwrap().len(), 1);
    // The re-pull brings t1 back, still archived by the pending overlay.
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 4)).unwrap();
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["u1"]);
}

#[test]
fn overlay_hides_at_once_and_is_dropped_when_the_mirror_catches_up() {
    let mut s = store();
    s.apply_changes(
        A,
        0,
        &batch(vec![thread("t1", A, 3, 100, &["sys:inbox", "sys:unread"])], vec![], vec![], vec![], 3),
    )
    .unwrap();
    let effects = s.add_overlay("op1", &["t1".into(), "missing".into()], &archive()).unwrap();
    assert_eq!(effects.len(), 1, "unknown threads are skipped");
    assert_eq!(effects["t1"].removed, vec!["sys:inbox"]);
    assert!(s.list_ids(&inbox()).unwrap().is_empty());
    assert!(s.summary("t1").unwrap().unwrap().pending);

    // mail.apply returned rev 5; the mirror is at 3, so the row stays.
    assert!(s.ack_overlay("op1", &revs(A, 5)).unwrap().is_empty());
    assert_eq!(s.overlay().unwrap()[0].min_rev, Some(5));
    // rev 4 is someone else's change: still pending.
    s.apply_changes(A, 3, &batch(vec![thread("t9", A, 4, 50, &["sys:inbox"])], vec![], vec![], vec![], 4)).unwrap();
    assert_eq!(s.overlay().unwrap().len(), 1);
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t9"]);
    // rev 5 carries the server's result (archived): the row is pruned and the state holds.
    s.apply_changes(A, 4, &batch(vec![thread("t1", A, 5, 100, &["sys:unread"])], vec![], vec![], vec![], 5)).unwrap();
    assert!(s.overlay().unwrap().is_empty());
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t9"]);
    assert!(!s.summary("t1").unwrap().unwrap().pending);
}

#[test]
fn ack_after_the_mirror_already_passed_prunes_at_once() {
    let mut s = store();
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 9)).unwrap();
    s.add_overlay("op1", &["t1".into()], &ThreadAction::Modify { add: vec!["sys:starred".into()], remove: vec![] })
        .unwrap();
    assert!(s.summary("t1").unwrap().unwrap().starred);
    assert_eq!(s.ack_overlay("op1", &revs(A, 7)).unwrap(), vec!["t1"]);
    assert!(s.overlay().unwrap().is_empty());
    assert!(!s.summary("t1").unwrap().unwrap().starred, "the mirror is the truth once it has caught up");
}

#[test]
fn a_failed_mutation_rolls_back_by_construction() {
    let mut s = store();
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 1)).unwrap();
    s.add_overlay("op1", &["t1".into()], &ThreadAction::Trash).unwrap();
    assert!(s.list_ids(&inbox()).unwrap().is_empty());
    assert_eq!(
        s.list_ids(&ListQuery { label_key: Some("sys:trash".into()), ..ListQuery::default() }).unwrap(),
        vec!["t1"]
    );
    assert!(s.list_ids(&ListQuery::default()).unwrap().is_empty(), "trash is not in all mail");
    assert_eq!(s.drop_overlay("op1").unwrap(), vec!["t1"]);
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t1"]);
}

#[test]
fn undo_inverts_the_recorded_effect() {
    let mut s = store();
    s.apply_changes(
        A,
        0,
        &batch(vec![thread("t1", A, 1, 100, &["sys:inbox", "sys:unread"])], vec![], vec![], vec![], 1),
    )
    .unwrap();
    s.add_overlay("op1", &["t1".into()], &archive()).unwrap();
    s.ack_overlay("op1", &revs(A, 2)).unwrap();
    // The server processed the archive (rev 2) before the user pressed undo.
    s.apply_changes(A, 1, &batch(vec![thread("t1", A, 2, 100, &["sys:unread"])], vec![], vec![], vec![], 2)).unwrap();
    assert!(s.list_ids(&inbox()).unwrap().is_empty());
    assert_eq!(s.add_undo("op1", "op2").unwrap(), vec!["t1"]);
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t1"]);
    assert!(s.summary("t1").unwrap().unwrap().unread);
    // A no-op action has nothing to undo.
    s.add_overlay("op3", &["t1".into()], &ThreadAction::Modify { add: vec!["sys:unread".into()], remove: vec![] })
        .unwrap();
    assert!(s.add_undo("op3", "op4").unwrap().is_empty());
}

#[test]
fn folder_providers_move_between_exclusive_folders() {
    let mut s = store();
    s.apply_changes(
        B,
        0,
        &batch(
            vec![thread("u1", B, 1, 100, &["sys:inbox"])],
            vec![],
            vec![label("arch", B, "Archive", true, Some("archive")), label("tag", B, "Blue", false, None)],
            vec![],
            1,
        ),
    )
    .unwrap();
    s.add_overlay("op1", &["u1".into()], &ThreadAction::Move { to: "l:arch".into() }).unwrap();
    assert!(s.list_ids(&inbox()).unwrap().is_empty());
    assert_eq!(
        s.list_ids(&ListQuery { label_key: Some("l:arch".into()), ..ListQuery::default() }).unwrap(),
        vec!["u1"]
    );
    s.add_overlay("op2", &["u1".into()], &ThreadAction::Move { to: "sys:inbox".into() }).unwrap();
    assert_eq!(s.summary("u1").unwrap().unwrap().label_keys, vec!["sys:inbox"], "the archive folder was left");
}

#[test]
fn inbox_threads_carry_their_split_row() {
    let mut s = store();
    let t = with(thread("t1", A, 1, 100, &["sys:inbox"]), "split", json!("split:vip"));
    s.apply_changes(A, 0, &batch(vec![t, thread("t2", A, 1, 90, &["sys:inbox"])], vec![], vec![], vec![], 1)).unwrap();
    let split = |key: &str| ListQuery { label_key: Some(key.into()), ..ListQuery::default() };
    assert_eq!(s.list_ids(&split("split:vip")).unwrap(), vec!["t1"]);
    assert_eq!(s.list_ids(&split("split:other")).unwrap(), vec!["t2"], "no split means split:other");
    s.add_overlay("op1", &["t1".into()], &archive()).unwrap();
    assert!(s.list_ids(&split("split:vip")).unwrap().is_empty(), "archived threads leave their split");
}

#[test]
fn snoozed_lists_sort_by_wake_time() {
    let mut s = store();
    let a = with(thread("t1", A, 1, 100, &["sys:snoozed"]), "snoozedUntil", json!(5_000));
    let b = with(thread("t2", A, 1, 200, &["sys:snoozed"]), "snoozedUntil", json!(1_000));
    s.apply_changes(A, 0, &batch(vec![a, b], vec![], vec![], vec![], 1)).unwrap();
    let ids = s.list_ids(&ListQuery { label_key: Some("sys:snoozed".into()), ..ListQuery::default() }).unwrap();
    assert_eq!(ids, vec!["t1", "t2"], "latest wake first, like every list");
}

#[test]
fn reminders_list_soonest_first() {
    let mut s = store();
    let a = with(thread("t1", A, 1, 100, &["sys:inbox"]), "reminderAt", json!(9_000));
    let b = with(thread("t2", A, 1, 200, &[]), "reminderAt", json!(3_000));
    s.apply_changes(A, 0, &batch(vec![a, b, thread("t3", A, 1, 300, &["sys:inbox"])], vec![], vec![], vec![], 1))
        .unwrap();
    assert_eq!(s.reminder_ids(None).unwrap(), vec!["t2", "t1"]);
    assert!(s.reminder_ids(Some(B)).unwrap().is_empty());
}

#[test]
fn per_account_lists_and_unified_counts() {
    let mut s = store();
    s.apply_changes(
        A,
        0,
        &batch(vec![thread("t1", A, 1, 100, &["sys:inbox", "sys:unread"])], vec![], vec![], vec![], 1),
    )
    .unwrap();
    s.apply_changes(
        B,
        0,
        &batch(vec![thread("u1", B, 1, 200, &["sys:inbox", "sys:unread"])], vec![], vec![], vec![], 1),
    )
    .unwrap();
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["u1", "t1"]);
    assert_eq!(s.list_ids(&ListQuery { account_id: Some(A.into()), ..inbox() }).unwrap(), vec!["t1"]);
    assert_eq!(s.unread_counts(&["sys:inbox".into()], None, 100).unwrap()["sys:inbox"], 2);
    assert_eq!(s.unread_counts(&["sys:inbox".into()], Some(B), 100).unwrap()["sys:inbox"], 1);
    assert_eq!(s.unread_counts(&["sys:inbox".into()], None, 1).unwrap()["sys:inbox"], 1, "capped");
}

#[test]
fn removed_accounts_lose_their_mirror() {
    let mut s = store();
    s.apply_changes(B, 0, &batch(vec![thread("u1", B, 1, 200, &["sys:inbox"])], vec![], vec![], vec![], 3)).unwrap();
    s.set_accounts(&[account(A, "gmail", "me@example.com")]).unwrap();
    assert_eq!(s.accounts().unwrap().len(), 1);
    assert!(s.list_ids(&inbox()).unwrap().is_empty());
    assert_eq!(s.account_rev(B).unwrap(), 0);
}

#[test]
fn outbox_keeps_order_and_dedupes() {
    let mut s = store();
    s.outbox_push("op1", "mail:apply", &json!({"a": 1})).unwrap();
    s.outbox_push("op2", "mail:undo", &json!({"b": 2})).unwrap();
    s.outbox_push("op1", "mail:apply", &json!({"a": 99})).unwrap();
    s.outbox_attempted("op1").unwrap();
    let all = s.outbox().unwrap();
    assert_eq!(all.iter().map(|e| e.client_op_id.as_str()).collect::<Vec<_>>(), vec!["op1", "op2"]);
    assert_eq!(all[0].args, json!({"a": 1}));
    assert_eq!(all[0].attempts, 1);
    s.outbox_remove("op1").unwrap();
    assert_eq!(s.outbox().unwrap().len(), 1);
}

#[test]
fn local_drafts_survive_a_reset_and_go_with_a_wipe() {
    let mut s = store();
    s.put_local_draft("d1", &json!({"subject": "First"})).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(3));
    s.put_local_draft("d2", &json!({"subject": "Second"})).unwrap();
    s.put_local_draft("d1", &json!({"subject": "First, edited"})).unwrap();
    let all = s.local_drafts().unwrap();
    assert_eq!(all.iter().map(|d| d.client_draft_id.as_str()).collect::<Vec<_>>(), vec!["d1", "d2"], "newest first");
    assert_eq!(s.local_draft("d1").unwrap().unwrap().doc["subject"], "First, edited");
    // A feed reset wipes the account's mirror rows, never what the user typed.
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 2)).unwrap();
    let mut reset = batch(vec![], vec![], vec![], vec![], 2);
    reset.reset_required = true;
    s.apply_changes(A, 2, &reset).unwrap();
    assert_eq!(s.local_drafts().unwrap().len(), 2);
    s.delete_local_draft("d2").unwrap();
    assert!(s.local_draft("d2").unwrap().is_none());
    s.wipe().unwrap();
    assert!(s.local_drafts().unwrap().is_empty(), "sign-out deletes the compose store");
}

#[test]
fn bodies_are_compressed_versioned_and_evicted_lru() {
    let mut s = store();
    let html = "<p>".to_owned() + &"hello ".repeat(2000) + "</p>";
    s.put_body("m1", 1, Some(&html), Some("hello"), Some(b"raw bytes"), u64::MAX).unwrap();
    assert!(s.body_cache_bytes().unwrap() < 2000, "zstd shrinks repetitive html");
    let b = s.body("m1", 1).unwrap().unwrap();
    assert_eq!(b.html.as_deref(), Some(html.as_str()));
    assert_eq!(b.raw.as_deref(), Some(&b"raw bytes"[..]));
    assert!(s.body("m1", 2).unwrap().is_none(), "another sanitizer version is a miss");
    assert_eq!(s.raw_body("m1").unwrap().as_deref(), Some(&b"raw bytes"[..]));

    s.put_body("m2", 1, Some("two"), None, None, u64::MAX).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    s.body("m1", 1).unwrap(); // m1 is now the most recent
    let m1_size = s.body_cache_bytes().unwrap();
    assert_eq!(s.evict_bodies(m1_size - 1).unwrap(), 1);
    assert!(s.body("m2", 1).unwrap().is_none(), "the least recently used body went first");
    assert!(s.body("m1", 1).unwrap().is_some());
}

#[test]
fn ai_cache_and_wipe() {
    let mut s = store();
    let key = Store::ai_cache_key("summarize", "abc", 1, "anthropic/claude-sonnet-5.5");
    s.put_ai_cached(&key, &json!({"tldr": "x"})).unwrap();
    assert_eq!(s.ai_cached(&key).unwrap(), Some(json!({"tldr": "x"})));
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:inbox"])], vec![], vec![], vec![], 1)).unwrap();
    s.wipe().unwrap();
    assert!(s.ai_cached(&key).unwrap().is_none());
    assert_eq!(s.thread_count().unwrap(), 0);
    assert!(s.accounts().unwrap().is_empty());
}

#[test]
fn an_older_schema_is_dropped_on_open() {
    let path = std::env::temp_dir().join(format!(
        "akatsuki-store-test-{}-{}.sqlite",
        std::process::id(),
        crate::time::now_ms()
    ));
    {
        let mut s = Store::open(&path).unwrap();
        s.set_accounts(&[account(A, "gmail", "me@example.com")]).unwrap();
        s.conn.execute("UPDATE meta SET value = '0' WHERE key = 'schema_version'", []).unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert!(s.accounts().unwrap().is_empty());
    drop(s);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn rows_without_a_revision_drop_once_the_server_shows_them() {
    let mut s = store();
    s.apply_changes(A, 0, &batch(vec![thread("t1", A, 1, 100, &["sys:unread"])], vec![], vec![], vec![], 1)).unwrap();
    // Undo of an archive: the inverse brings the thread back to the inbox.
    s.add_overlay("u1", &["t1".into()], &ThreadAction::Modify { add: vec!["sys:inbox".into()], remove: vec![] })
        .unwrap();
    assert!(s.ack_overlay_when_converged("u1").unwrap().is_empty(), "the server has not caught up");
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t1"]);
    // An unrelated change: still waiting.
    s.apply_changes(
        A,
        1,
        &batch(vec![thread("t1", A, 2, 100, &["sys:unread", "sys:starred"])], vec![], vec![], vec![], 2),
    )
    .unwrap();
    assert_eq!(s.overlay().unwrap().len(), 1);
    // The undo lands on the server: the row is redundant and goes.
    s.apply_changes(
        A,
        2,
        &batch(vec![thread("t1", A, 3, 100, &["sys:inbox", "sys:unread"])], vec![], vec![], vec![], 3),
    )
    .unwrap();
    assert!(s.overlay().unwrap().is_empty());
    assert_eq!(s.list_ids(&inbox()).unwrap(), vec!["t1"]);
}
