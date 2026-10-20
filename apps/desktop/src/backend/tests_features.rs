//! The new desktop features end to end against the demo mailbox (D-316): its in-process Convex
//! and worker run the production code paths, and AI providers are local mock servers.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use akatsuki_ai::AaspEvent;
use akatsuki_core::convex::AiProviderKind;
use akatsuki_core::convex::rows::{AiConsent, AiModels, AiProfile, AiRoute, AiRouteFeature, AiSettings};
use akatsuki_core::convex::{AiFeature, SendState};
use akatsuki_core::store::{ListQuery, Store};
use akatsuki_core::time::now_ms;
use akatsuki_search::SearchIndex;
use tokio::runtime::Handle;

use super::{Backend, DictationState, Event, SearchSource};
use crate::config::{Endpoints, Paths};
use crate::secrets::MemoryStore;
use crate::vm::compose::Draft;

fn backend() -> (Arc<Backend>, UnboundedReceiver<Event>) {
    let paths = Paths {
        data_dir: std::env::temp_dir().join(format!("akatsuki-features-{}", uuid::Uuid::new_v4())),
        config_file: std::env::temp_dir().join("unused.json"),
    };
    Backend::new(
        Handle::current(),
        Store::open_in_memory().unwrap(),
        SearchIndex::open_in_memory().unwrap(),
        Arc::new(MemoryStore::default()),
        Endpoints::default(),
        paths,
        "dev-1".into(),
        true,
    )
}

async fn next(rx: &mut UnboundedReceiver<Event>, f: impl Fn(&Event) -> bool) -> Event {
    loop {
        let e = tokio::time::timeout(Duration::from_secs(5), rx.next()).await.expect("event in time").expect("open");
        if f(&e) {
            return e;
        }
    }
}

async fn until(what: &str, f: impl Fn() -> bool) {
    for _ in 0..250 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn started() -> (Arc<Backend>, UnboundedReceiver<Event>) {
    let (b, rx) = backend();
    b.start_demo();
    let b2 = b.clone();
    until("the mirror", move || !inbox(&b2).is_empty()).await;
    (b, rx)
}

fn inbox(b: &Backend) -> Vec<String> {
    b.store
        .lock()
        .unwrap()
        .list_ids(&ListQuery { label_key: Some("sys:inbox".into()), ..ListQuery::default() })
        .unwrap()
}

fn settings(kind: AiProviderKind, base: String, auto_triage: bool) -> AiSettings {
    AiSettings {
        profiles: vec![AiProfile {
            profile_id: "p1".into(),
            label: "Local".into(),
            kind,
            base_url: Some(base),
            models: AiModels {
                default: Some("m".into()),
                fast: Some("m".into()),
                transcription: Some("whisper-1".into()),
                ..AiModels::default()
            },
            key_on_devices: vec![],
        }],
        routes: vec![AiRoute { feature: AiRouteFeature::DEFAULT, profile_id: "p1".into(), model: None, enabled: true }],
        consent: AiConsent { granted_at: Some(1), version: Some(1), auto_triage, sync_voice_profile: false },
        ..AiSettings::default()
    }
}

/// An OpenAI-compatible chat stream that answers `content`.
fn chat(content: &str) -> ResponseTemplate {
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices": [{"delta": {"content": content}, "finish_reason": null}]}),
        json!({"choices": [{"delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 3, "completion_tokens": 2}})
    );
    ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(body)
}

#[tokio::test(flavor = "multi_thread")]
async fn prefs_and_snippets_load_and_prefs_sync_only_changes() {
    let (b, mut rx) = started().await;
    let Event::Me(me) = next(&mut rx, |e| matches!(e, Event::Me(_))).await else { unreachable!() };
    assert_eq!(me.profile.prefs.undo_send_seconds, 10);
    let Event::Snippets(s) = next(&mut rx, |e| matches!(e, Event::Snippets(_))).await else { unreachable!() };
    assert!(s.iter().any(|x| x.shortcut.as_deref() == Some("thx")));

    let mut local = crate::vm::prefs::Synced::of(&crate::config::LocalSettings::default());
    local.undo_send_seconds = 30;
    let pending = crate::vm::prefs::Pending { undo_send_seconds: true, ..Default::default() };
    b.sync_prefs(local, crate::vm::prefs::patch(&local, &pending));
    next(&mut rx, |e| matches!(e, Event::PrefsSynced(s) if s.undo_send_seconds == 30)).await;
    let r = b.remote().unwrap();
    assert_eq!(r.api.me().await.unwrap().unwrap().profile.prefs.undo_send_seconds, 30, "stored on the account");

    b.save_snippet(None, "Sign-off".into(), Some(";bye".into()), None, "Best,\n{{name}}".into());
    let Event::Snippets(s) = next(&mut rx, |e| matches!(e, Event::Snippets(l) if l.len() == 4)).await else {
        unreachable!()
    };
    let bye = s.iter().find(|x| x.name == "Sign-off").unwrap();
    assert_eq!(bye.shortcut.as_deref(), Some("bye"), "the semicolon is not part of the shortcut");
    assert_eq!(bye.variables, vec!["name"]);
    assert_eq!(bye.body_html, "<p>Best,<br>{{name}}</p>");
    b.remove_snippet(bye.id.as_str());
    next(&mut rx, |e| matches!(e, Event::Snippets(l) if l.len() == 3)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn labels_are_created_applied_renamed_and_deleted() {
    let (b, _rx) = started().await;
    b.create_label(crate::demo::GMAIL.into(), "Taxes".into(), vec!["t-board".into()]);
    let label = |b: &Backend, name: &str| {
        b.store.lock().unwrap().labels(Some(crate::demo::GMAIL)).unwrap().into_iter().find(|l| l.name == name)
    };
    let b2 = b.clone();
    until("the new label in the mirror", move || label(&b2, "Taxes").is_some()).await;
    let l = label(&b, "Taxes").unwrap();
    let key = l.label_key.clone();
    let b2 = b.clone();
    let k2 = key.clone();
    until("the label on the thread", move || {
        b2.store.lock().unwrap().summary("t-board").unwrap().unwrap().label_keys.contains(&k2)
    })
    .await;
    // The add reached the server (the demo, unlike Convex, does not order it after later label ops).
    let b2 = b.clone();
    until("the add confirmed", move || b2.store.lock().unwrap().overlay().unwrap().is_empty()).await;

    b.rename_label(l.id.as_str(), "Tax 2026".into());
    let b2 = b.clone();
    until("the new name", move || label(&b2, "Tax 2026").is_some()).await;

    b.remove_label(l.id.as_str(), "Tax 2026");
    let b2 = b.clone();
    until("the label gone", move || label(&b2, "Tax 2026").is_none()).await;
    let b2 = b.clone();
    until("the key gone from the thread", move || {
        !b2.store.lock().unwrap().summary("t-board").unwrap().unwrap().label_keys.contains(&key)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_ops_roll_back_and_retry() {
    let (b, mut rx) = started().await;
    let plans = crate::vm::mail::plan(
        crate::vm::mail::ActionKind::Archive,
        &["t-staging".into()],
        &crate::vm::mail::Folder::Inbox,
        &b.store.lock().unwrap(),
    )
    .unwrap();
    b.apply(plans, Some("Archived"));
    assert!(!inbox(&b).contains(&"t-staging".to_owned()), "optimistic");
    let Event::Failures(list) = next(&mut rx, |e| matches!(e, Event::Failures(l) if !l.is_empty())).await else {
        unreachable!()
    };
    assert_eq!(list[0].subject.as_deref(), Some("Staging is green again"));
    assert!(crate::backend::failure_text(&list[0]).starts_with("Couldn't update 'Staging is green again'"));
    let b2 = b.clone();
    until("the rollback", move || inbox(&b2).contains(&"t-staging".to_owned())).await;

    b.retry_failure(list[0].id.as_str());
    assert!(b.failures().is_empty(), "the toast goes at once");
    let b2 = b.clone();
    until("the retried archive", move || !inbox(&b2).contains(&"t-staging".to_owned())).await;
    let r = b.remote().unwrap();
    assert!(r.api.op_failures().await.unwrap().next().await.unwrap().unwrap().is_empty(), "acknowledged");
}

#[tokio::test(flavor = "multi_thread")]
async fn later_sends_are_listed_rescheduled_and_cancelled() {
    let (b, mut rx) = started().await;
    let mut d = Draft::new("d-later".into(), crate::demo::GMAIL, crate::demo::ME);
    d.to = "hana@kitsune-mail.example".into();
    d.subject = "Tomorrow".into();
    d.body = "See you".into();
    assert!(b.send(d.clone(), 10, Some(now_ms() + 1000)).is_err(), "too soon to be a later send");
    let at = now_ms() + 3_600_000;
    let p = b.send(d.clone(), 10, Some(at)).unwrap();
    assert!(p.later && p.send_at == at);
    tokio::time::sleep(Duration::from_millis(100)).await;
    b.list_scheduled();
    let Event::Scheduled(list) = next(&mut rx, |e| matches!(e, Event::Scheduled(l) if !l.is_empty())).await else {
        unreachable!()
    };
    assert_eq!(list[0].state, SendState::Scheduled);
    assert_eq!(list[0].preview.subject, "Tomorrow");
    b.reschedule(p.client_send_id.clone(), at + 60_000);
    next(&mut rx, |e| matches!(e, Event::Scheduled(l) if l.first().is_some_and(|s| s.send_at == at + 60_000))).await;
    b.cancel_scheduled(p.client_send_id.clone());
    next(&mut rx, |e| matches!(e, Event::Scheduled(l) if l.is_empty())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn drafts_stay_on_the_device_until_sent_or_discarded() {
    let (b, mut rx) = started().await;
    let mut d = Draft::new("d-local".into(), crate::demo::GMAIL, crate::demo::ME);
    b.save_local_draft(&d);
    assert!(b.local_drafts().is_empty(), "blank drafts are not kept");
    d.subject = "Notes".into();
    d.to = "hana@kitsune-mail.example".into();
    b.save_local_draft(&d);
    let kept = b.local_drafts();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].subject, "Notes");
    assert!(kept[0].updated_at > 0);
    b.send(kept[0].clone(), 0, None).unwrap();
    next(&mut rx, |e| matches!(e, Event::SendDone { error: None, reopen: None, .. })).await;
    assert!(b.local_drafts().is_empty(), "sent drafts leave the compose store");
    d.client_draft_id = "d-gone".into();
    d.base_version = 2;
    b.save_local_draft(&d);
    b.discard_draft(&d);
    assert!(b.local_drafts().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn server_and_provider_search_add_hits() {
    let (b, mut rx) = started().await;
    b.search_server("deck", None);
    let Event::SearchResults { hits, source, .. } =
        next(&mut rx, |e| matches!(e, Event::SearchResults { source: SearchSource::Server, .. })).await
    else {
        unreachable!()
    };
    assert_eq!(source, SearchSource::Server);
    assert!(hits.contains(&"t-board".to_owned()));
    b.search_server("filename:plan deck", None);
    let Event::SearchResults { unsupported, .. } = next(&mut rx, |e| matches!(e, Event::SearchResults { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(unsupported, vec!["filename:"], "the UI offers the full-mailbox search");

    // "teapot" is only in a body that was never opened here: the provider finds it.
    b.search_provider("teapot", None);
    let Event::SearchResults { hits, error, .. } =
        next(&mut rx, |e| matches!(e, Event::SearchResults { source: SearchSource::Provider, .. })).await
    else {
        unreachable!()
    };
    assert_eq!(error, None);
    assert!(hits.contains(&"t-dawn".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn byo_triage_claims_sends_headers_only_and_stores_results() {
    let server = MockServer::start().await;
    let answer = json!({"items": [
        {"key": "1", "category": "receipts", "needsReply": false},
        {"key": "2", "category": "important", "needsReply": true}
    ]});
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat(&answer.to_string()))
        .mount(&server)
        .await;
    let (b, _rx) = started().await;
    let r = b.remote().unwrap();
    *b.ai_settings.write().unwrap() =
        Some(settings(AiProviderKind::OpenaiCompatible, format!("{}/v1", server.uri()), false));
    assert!(!b.triage_once(&r).await, "nothing runs while the switch is off");
    *b.ai_settings.write().unwrap() =
        Some(settings(AiProviderKind::OpenaiCompatible, format!("{}/v1", server.uri()), true));
    assert!(b.triage_once(&r).await);
    let sent = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body).into_owned();
    assert!(sent.contains("snippet:"), "headers and snippets");
    assert!(!sent.contains("Content-Type: text/html"), "never bodies");
    let b2 = b.clone();
    until("triage in the mirror", move || {
        let ids = inbox(&b2);
        let store = b2.store.lock().unwrap();
        store.summaries(&ids).unwrap().iter().filter(|s| s.triage.is_some()).count() == 2
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ai_write_streams_text_and_action_items_are_cached() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat("Thursday works."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let actions =
        json!({"actions": [{"kind": "todo", "title": "Add the hiring slide", "due": null, "quote": "add the slide"}]});
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(chat(&actions.to_string()))
        .expect(1)
        .mount(&server)
        .await;
    let (b, mut rx) = started().await;
    assert!(b.run_compose("say yes", "", None, "me", "UTC").is_err(), "no provider, nothing runs");
    *b.ai_settings.write().unwrap() =
        Some(settings(AiProviderKind::OpenaiCompatible, format!("{}/v1", server.uri()), false));
    let req = b.run_compose("say thursday works", "", Some("t-board"), "Me", "UTC").unwrap();
    let Event::Ai { event: AaspEvent::Done(d), .. } =
        next(&mut rx, |e| matches!(e, Event::Ai { request, event: AaspEvent::Done(_) } if *request == req)).await
    else {
        unreachable!()
    };
    assert_eq!(d.text.as_deref(), Some("Thursday works."));
    let first = String::from_utf8_lossy(&server.received_requests().await.unwrap()[0].body).into_owned();
    assert!(first.contains("say thursday works") && first.contains("Mira Okafor"), "instruction and thread");

    for _ in 0..2 {
        let req = b.run_ai(AiFeature::ExtractActions, "t-board", "Me", "UTC").unwrap();
        let Event::Ai { event: AaspEvent::Done(d), .. } =
            next(&mut rx, |e| matches!(e, Event::Ai { request, event: AaspEvent::Done(_) } if *request == req)).await
        else {
            unreachable!()
        };
        assert_eq!(crate::vm::ai::action_items(&d.object.unwrap())[0].title, "Add the hiring slide");
    }
    // `expect(1)` on drop: the second run came from the device cache.
}

#[tokio::test(flavor = "multi_thread")]
async fn dictation_checks_consent_and_transcribes_on_the_local_route() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"text": "Thursday works for me.", "duration": 1.5})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let (b, mut rx) = started().await;
    assert_eq!(b.start_dictation().unwrap_err(), "Set up an AI provider in Settings first.");
    let mut s = settings(AiProviderKind::OpenaiCompatible, format!("{}/v1", server.uri()), false);
    s.consent.granted_at = None;
    *b.ai_settings.write().unwrap() = Some(s.clone());
    assert!(b.dictation_ready().is_err(), "no recording without consent");
    s.consent.granted_at = Some(1);
    *b.ai_settings.write().unwrap() = Some(s);
    assert_eq!(b.dictation_ready().unwrap(), "Stays on this device");
    let mut wav = crate::dictation::wav_header(16_000);
    wav.extend(std::iter::repeat_n(0u8, 32_000));
    b.transcribe_wav(wav).await;
    let Event::Dictation(state) = next(&mut rx, |e| matches!(e, Event::Dictation(_))).await else { unreachable!() };
    assert_eq!(state, DictationState::Text("Thursday works for me.".into()));
}
