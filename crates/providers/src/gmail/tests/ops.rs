//! `apply`: modify, trash/untrash, snooze label, batchModify, labels, drafts, unsupported ops.

use pretty_assertions::assert_eq;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use super::*;
use crate::{
    LabelTarget, MailProvider, OpEffect, ProviderOp, RemoteDraftRef, RemoteMessageRef, RemoteThreadRef, SysLabel,
};

fn thread(id: &str, msgs: &[&str]) -> RemoteThreadRef {
    RemoteThreadRef {
        provider_thread_id: id.into(),
        messages: msgs.iter().map(|m| RemoteMessageRef { provider_message_id: (*m).into(), imap: vec![] }).collect(),
    }
}

fn small_thread(id: &str) -> String {
    THREAD_SMALL.replace("18c2b00000000003", id)
}

async fn mount_thread(server: &MockServer, id: &str) {
    Mock::given(method("GET"))
        .and(path(format!("{USER}/threads/{id}")))
        .respond_with(json(&small_thread(id)))
        .mount(server)
        .await;
}

fn post_id(r: &Result<OpEffect, ProviderError>) -> String {
    match r {
        Ok(OpEffect::Applied { post: Some(t), .. }) => t.provider_thread_id.clone(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn archive_is_threads_modify_with_a_post_read() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t1/modify")))
        .and(body_json(
            serde_json::json!({"addLabelIds": ["Label_1", "STARRED"], "removeLabelIds": ["INBOX", "UNREAD"]}),
        ))
        .respond_with(json(r#"{"id":"t1","messages":[{"id":"m1","threadId":"t1","labelIds":["STARRED","Label_1"]}]}"#))
        .expect(1)
        .mount(&server)
        .await;
    mount_thread(&server, "t1").await;
    let op = ProviderOp::Modify {
        thread: thread("t1", &["m1"]),
        add: vec![
            LabelTarget::Label { provider_label_id: "Label_1".into(), exclusive: false },
            LabelTarget::Sys(SysLabel::Starred),
        ],
        remove: vec![LabelTarget::Sys(SysLabel::Inbox), LabelTarget::Sys(SysLabel::Unread)],
    };
    let r = p.apply(&[("op1".into(), op)]).await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, "op1");
    assert_eq!(post_id(&r[0].1), "t1");
}

#[tokio::test]
async fn trash_and_untrash_use_the_dedicated_endpoints() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t1/trash")))
        .respond_with(json(r#"{"id":"t1"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t2/untrash")))
        .respond_with(json(r#"{"id":"t2"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t2/modify")))
        .and(body_json(serde_json::json!({"addLabelIds": ["INBOX"], "removeLabelIds": []})))
        .respond_with(json(r#"{"id":"t2"}"#))
        .expect(1)
        .mount(&server)
        .await;
    // A modify delta that adds sys:trash is a trash, plus the rest of the delta.
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t3/trash")))
        .respond_with(json(r#"{"id":"t3"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t3/modify")))
        .and(body_json(serde_json::json!({"addLabelIds": [], "removeLabelIds": ["INBOX"]})))
        .respond_with(json(r#"{"id":"t3"}"#))
        .expect(1)
        .mount(&server)
        .await;
    for t in ["t1", "t2", "t3"] {
        mount_thread(&server, t).await;
    }
    let r = p
        .apply(&[
            ("a".into(), ProviderOp::Trash { thread: thread("t1", &["m1"]) }),
            ("b".into(), ProviderOp::Untrash { thread: thread("t2", &["m2"]) }),
            (
                "c".into(),
                ProviderOp::Modify {
                    thread: thread("t3", &["m3"]),
                    add: vec![LabelTarget::Sys(SysLabel::Trash)],
                    remove: vec![LabelTarget::Sys(SysLabel::Inbox)],
                },
            ),
        ])
        .await;
    assert_eq!(
        r.iter().map(|(id, r)| (id.as_str(), post_id(r))).collect::<Vec<_>>(),
        vec![("a", "t1".into()), ("b", "t2".into()), ("c", "t3".into())]
    );
}

#[tokio::test]
async fn snooze_creates_the_label_lazily() {
    let (server, p, _) = setup().await;
    Mock::given(method("GET"))
        .and(path(format!("{USER}/labels")))
        .respond_with(json(r#"{"labels":[{"id":"INBOX","name":"INBOX","type":"system"}]}"#))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/labels")))
        .respond_with(json(r#"{"id":"Label_42","name":"Akatsuki/Snoozed","type":"user"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t1/modify")))
        .and(body_json(serde_json::json!({"addLabelIds": ["Label_42"], "removeLabelIds": ["INBOX"]})))
        .respond_with(json(r#"{"id":"t1"}"#))
        .expect(2)
        .mount(&server)
        .await;
    mount_thread(&server, "t1").await;
    let op = || ProviderOp::Modify {
        thread: thread("t1", &["m1"]),
        add: vec![LabelTarget::Sys(SysLabel::Snoozed)],
        remove: vec![LabelTarget::Sys(SysLabel::Inbox)],
    };
    // The second op reuses the cached id: labels.create runs once.
    let r = p.apply(&[("s1".into(), op()), ("s2".into(), op())]).await;
    assert!(r.iter().all(|(_, r)| r.is_ok()));
}

#[tokio::test]
async fn identical_deltas_use_batch_modify() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/messages/batchModify")))
        .and(body_json(serde_json::json!({
            "ids": ["m1", "m2a", "m2b", "m3", "m4", "m5"],
            "addLabelIds": [],
            "removeLabelIds": ["UNREAD"]
        })))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let parts: Vec<String> = ["t1", "t2", "t3", "t4"].iter().map(|t| small_thread(t)).collect();
    let mut resp: Vec<(u16, &str)> = parts.iter().map(|s| (200, s.as_str())).collect();
    resp.push((404, r#"{"error":{"code":404,"message":"Requested entity was not found."}}"#));
    Mock::given(method("POST"))
        .and(path("/batch/gmail/v1"))
        .respond_with(batch_response(&resp))
        .expect(1)
        .mount(&server)
        .await;
    // A different delta after the group runs on its own.
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/t6/modify")))
        .respond_with(json("{}"))
        .expect(1)
        .mount(&server)
        .await;
    mount_thread(&server, "t6").await;

    let read = |t: &str, m: &[&str]| ProviderOp::Modify {
        thread: thread(t, m),
        add: vec![],
        remove: vec![LabelTarget::Sys(SysLabel::Unread)],
    };
    let ops = vec![
        ("o1".into(), read("t1", &["m1"])),
        ("o2".into(), read("t2", &["m2a", "m2b"])),
        ("o3".into(), read("t3", &["m3"])),
        ("o4".into(), read("t4", &["m4"])),
        ("o5".into(), read("t5", &["m5"])),
        (
            "o6".into(),
            ProviderOp::Modify {
                thread: thread("t6", &["m6"]),
                add: vec![LabelTarget::Sys(SysLabel::Unread)],
                remove: vec![],
            },
        ),
    ];
    let r = p.apply(&ops).await;
    assert_eq!(r.len(), 6);
    assert_eq!(post_id(&r[0].1), "t1");
    assert_eq!(post_id(&r[3].1), "t4");
    assert_eq!(r[4].1, Err(ProviderError::NotFound), "t5 vanished");
    assert_eq!(post_id(&r[5].1), "t6");
}

#[tokio::test]
async fn batch_modify_failure_fails_every_op_in_the_group() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/messages/batchModify")))
        .respond_with(gmail_error(400, "invalidArgument", "Invalid label: SENT"))
        .mount(&server)
        .await;
    let ops: Vec<(String, ProviderOp)> = (0..5)
        .map(|i| {
            (
                format!("o{i}"),
                ProviderOp::Modify {
                    thread: thread(&format!("t{i}"), &["m"]),
                    add: vec![LabelTarget::Sys(SysLabel::Sent)],
                    remove: vec![],
                },
            )
        })
        .collect();
    let r = p.apply(&ops).await;
    assert_eq!(r.len(), 5);
    assert!(r.iter().all(|(_, r)| matches!(r, Err(ProviderError::Invalid(_)))));
}

#[tokio::test]
async fn empty_delta_is_moot_and_errors_are_per_op() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/threads/gone/modify")))
        .respond_with(gmail_error(404, "notFound", "Requested entity was not found."))
        .mount(&server)
        .await;
    let r = p
        .apply(&[
            ("moot".into(), ProviderOp::Modify { thread: thread("t1", &[]), add: vec![], remove: vec![] }),
            (
                "gone".into(),
                ProviderOp::Modify {
                    thread: thread("gone", &[]),
                    add: vec![LabelTarget::Sys(SysLabel::Starred)],
                    remove: vec![],
                },
            ),
            ("del".into(), ProviderOp::DeleteForever { thread: thread("t1", &[]) }),
            ("mv".into(), ProviderOp::Move { thread: thread("t1", &[]), to: LabelTarget::Sys(SysLabel::Inbox) }),
        ])
        .await;
    assert_eq!(r[0].1, Ok(OpEffect::Moot));
    assert_eq!(r[1].1, Err(ProviderError::NotFound));
    assert_eq!(r[2].1, Err(ProviderError::Unsupported("delete_forever")));
    assert_eq!(r[3].1, Err(ProviderError::Unsupported("move")));
}

#[tokio::test]
async fn label_ops() {
    let (server, p, _) = setup().await;
    Mock::given(method("POST"))
        .and(path(format!("{USER}/labels")))
        .respond_with(json(r##"{"id":"Label_50","name":"Travel","type":"user","color":{"backgroundColor":"#16a765","textColor":"#ffffff"}}"##))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path(format!("{USER}/labels/Label_50")))
        .and(body_json(serde_json::json!({"name": "Trips"})))
        .respond_with(json(r#"{"id":"Label_50","name":"Trips","type":"user"}"#))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{USER}/labels/Label_50")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{USER}/labels/Label_404")))
        .respond_with(gmail_error(404, "notFound", "Not Found"))
        .mount(&server)
        .await;
    let r = p
        .apply(&[
            ("c".into(), ProviderOp::LabelCreate { name: "Travel".into(), color: Some("#16a765".into()) }),
            ("r".into(), ProviderOp::LabelRename { provider_label_id: "Label_50".into(), name: "Trips".into() }),
            ("d".into(), ProviderOp::LabelDelete { provider_label_id: "Label_50".into() }),
            ("d404".into(), ProviderOp::LabelDelete { provider_label_id: "Label_404".into() }),
        ])
        .await;
    match &r[0].1 {
        Ok(OpEffect::Applied { created: Some(l), post: None }) => {
            assert_eq!(
                (l.provider_label_id.as_str(), l.name.as_str(), l.color.as_deref()),
                ("Label_50", "Travel", Some("#16a765"))
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(r[1].1, Ok(OpEffect::Applied { post: None, created: None }));
    assert_eq!(r[2].1, Ok(OpEffect::Applied { post: None, created: None }));
    assert_eq!(r[3].1, Ok(OpEffect::Moot));
}

#[tokio::test]
async fn delete_draft_op() {
    let (server, p, _) = setup().await;
    Mock::given(method("DELETE"))
        .and(path(format!("{USER}/drafts/r-1")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path(format!("{USER}/drafts/r-2")))
        .respond_with(gmail_error(404, "notFound", "Not Found"))
        .mount(&server)
        .await;
    let d = |id: &str| RemoteDraftRef { provider_draft_id: id.into(), provider_message_id: None };
    let r = p
        .apply(&[
            ("a".into(), ProviderOp::DeleteDraft { draft: d("r-1") }),
            ("b".into(), ProviderOp::DeleteDraft { draft: d("r-2") }),
        ])
        .await;
    assert_eq!(r[0].1, Ok(OpEffect::Applied { post: None, created: None }));
    assert_eq!(r[1].1, Ok(OpEffect::Moot));
}
