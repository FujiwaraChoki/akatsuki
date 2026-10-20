//! Streaming tests against a local mock HTTP server. No real keys, no network.

use akatsuki_ai::settings::{
    AiConsent, AiFeature, AiModels, AiProfile, AiProviderKind, AiRoute, AiRouteFeature, AiSettings,
};
use akatsuki_ai::{
    AaspEvent, AiClient, Escalation, PromptPack, Provider, ROUTING, resolve, run_feature, run_feature_with,
};
use akatsuki_core::wire::ErrorCode;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn sse(events: &[(&str, Value)]) -> String {
    events
        .iter()
        .map(|(e, d)| if e.is_empty() { format!("data: {d}\n\n") } else { format!("event: {e}\ndata: {d}\n\n") })
        .collect()
}

fn openai_chunks(parts: &[&str]) -> String {
    let mut out: String = parts
        .iter()
        .map(|p| format!("data: {}\n\n", json!({"choices": [{"delta": {"content": p}, "finish_reason": null}]})))
        .collect();
    out.push_str(&format!("data: {}\n\n", json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})));
    out.push_str(&format!(
        "data: {}\n\n",
        json!({"choices": [], "usage": {"prompt_tokens": 42, "completion_tokens": 7}})
    ));
    out.push_str("data: [DONE]\n\n");
    out
}

fn settings(kind: AiProviderKind, base_url: &str, model: Option<&str>) -> AiSettings {
    AiSettings {
        profiles: vec![AiProfile {
            profile_id: "p1".into(),
            label: "Test".into(),
            kind,
            base_url: Some(base_url.into()),
            models: AiModels { default: model.map(str::to_owned), ..Default::default() },
            key_on_devices: vec!["dev1".into()],
        }],
        routes: vec![AiRoute { feature: AiRouteFeature::DEFAULT, profile_id: "p1".into(), model: None, enabled: true }],
        consent: AiConsent { granted_at: Some(1), version: Some(1), auto_triage: false, sync_voice_profile: false },
        ..Default::default()
    }
}

fn vars(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().unwrap().clone()
}

async fn collect(s: futures::stream::BoxStream<'static, AaspEvent>) -> Vec<AaspEvent> {
    s.collect().await
}

/// The real prompt pack, embedded in the crate at build time.
fn pack() -> PromptPack {
    PromptPack::embedded().unwrap()
}

/// Valid `rewrite` vars.
fn rewrite(text: &str) -> serde_json::Map<String, Value> {
    vars(json!({"text": text, "style": "fix", "voice": null}))
}

/// Valid `summarize` vars.
fn summarize(transcript: &str) -> serde_json::Map<String, Value> {
    vars(json!({"transcript": transcript, "userName": "Sam", "today": "2026-10-20"}))
}

#[tokio::test]
async fn openai_compatible_text_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test-not-real"))
        .and(body_partial_json(json!({"model": "llama-local", "stream": true, "max_tokens": 2000})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(openai_chunks(&["Hello", ", ", "world"])),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pack = pack();
    let s = settings(AiProviderKind::OpenaiCompatible, &format!("{}/v1", server.uri()), Some("llama-local"));
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("hi"), |_| {
        Some("sk-test-not-real".into())
    }))
    .await;

    assert!(
        matches!(&events[0], AaspEvent::Meta(m) if m.model == "llama-local" && m.feature == "rewrite" && m.prompt_version == 1)
    );
    let deltas: String = events
        .iter()
        .filter_map(|e| match e {
            AaspEvent::Delta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, "Hello, world");
    let AaspEvent::Done(done) = events.last().unwrap() else { panic!("{events:?}") };
    assert_eq!(done.text.as_deref(), Some("Hello, world"));
    assert_eq!((done.usage.input_tokens, done.usage.output_tokens), (42, 7));

    let req: &Request = &server.received_requests().await.unwrap()[0];
    let body: Value = req.body_json().unwrap();
    assert!(body.get("temperature").is_none(), "no sampling parameters");
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(body["messages"][1]["content"].as_str().unwrap().contains("<text>\nhi\n</text>"));
    assert!(body.get("response_format").is_none());
}

#[tokio::test]
async fn openai_compatible_structured_output_with_partials() {
    let server = MockServer::start().await;
    let parts = [
        "{\"tl",
        "dr\": \"Lunch ",
        "moved\", \"bul",
        "lets\": [\"Noon\", ",
        "\"Room 4\"], \"asks\": [], \"deadlines\": []}",
    ];
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&parts)))
        .mount(&server)
        .await;
    let pack = pack();
    // Ollama via its OpenAI-compatible /v1 endpoint: no key needed.
    let s = settings(AiProviderKind::Ollama, &format!("{}/v1", server.uri()), Some("qwen3"));
    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Summarize, summarize("From: a"), |_| None)).await;

    let partials: Vec<&Value> = events
        .iter()
        .filter_map(|e| match e {
            AaspEvent::Partial { object } => Some(object),
            _ => None,
        })
        .collect();
    assert!(partials.len() >= 3, "{events:?}");
    assert!(partials.contains(&&json!({"tldr": "Lunch "})));
    let AaspEvent::Done(done) = events.last().unwrap() else { panic!() };
    assert_eq!(
        done.object,
        Some(json!({"tldr": "Lunch moved", "bullets": ["Noon", "Room 4"], "asks": [], "deadlines": []}))
    );
    assert_eq!(done.text, None);

    let body: Value = server.received_requests().await.unwrap()[0].body_json().unwrap();
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert_eq!(
        body["response_format"]["json_schema"]["schema"]["required"],
        json!(["tldr", "bullets", "asks", "deadlines"])
    );
    assert!(
        body["messages"][1]["content"].as_str().unwrap().contains("<email_transcript>\nFrom: a\n</email_transcript>")
    );
    assert!(server.received_requests().await.unwrap()[0].headers.get("authorization").is_none());
}

#[tokio::test]
async fn anthropic_native_structured_output() {
    let server = MockServer::start().await;
    let body = sse(&[
        (
            "message_start",
            json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "usage": {"input_tokens": 120, "output_tokens": 1}}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "private"}}),
        ),
        (
            "content_block_start",
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
        ),
        ("ping", json!({"type": "ping"})),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "{\"tldr\":\"Ship it\","}}),
        ),
        (
            "content_block_delta",
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "\"bullets\":[],\"asks\":[],\"deadlines\":[]}"}}),
        ),
        ("content_block_stop", json!({"type": "content_block_stop", "index": 1})),
        (
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 15}}),
        ),
        ("message_stop", json!({"type": "message_stop"})),
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-anthropic-key-not-real"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .expect(1)
        .mount(&server)
        .await;
    let pack = pack();
    // No model in the profile: resolution falls back to routing.json (claude-sonnet-5-5).
    let s = settings(AiProviderKind::Anthropic, &server.uri().replace("127.0.0.1", "localhost"), None);
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Summarize, summarize("x"), |_| {
        Some("test-anthropic-key-not-real".into())
    }))
    .await;

    let AaspEvent::Meta(meta) = &events[0] else { panic!() };
    assert_eq!(meta.model, "claude-sonnet-5-5");
    let AaspEvent::Done(done) = events.last().unwrap() else { panic!("{events:?}") };
    assert_eq!(done.object, Some(json!({"tldr": "Ship it", "bullets": [], "asks": [], "deadlines": []})));
    assert_eq!((done.usage.input_tokens, done.usage.output_tokens), (120, 15));
    assert!(!format!("{events:?}").contains("private"), "thinking never leaks");

    let req: Value = server.received_requests().await.unwrap()[0].body_json().unwrap();
    assert_eq!(req["model"], "claude-sonnet-5-5");
    assert_eq!(req["output_config"]["format"]["type"], "json_schema");
    assert_eq!(req["output_config"]["effort"], "low");
    assert!(req["system"].as_str().unwrap().starts_with("You summarize one email thread"));
    for forbidden in ["temperature", "top_p", "top_k", "tool_choice", "tools", "thinking"] {
        assert!(req.get(forbidden).is_none(), "{forbidden} must not be sent");
    }
}

#[tokio::test]
async fn invalid_output_gets_one_repair_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&["{\"tldr\": 5}"])))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&[
            "{\"tldr\": \"ok\", \"bullets\": [\"a\"], \"asks\": [], \"deadlines\": []}",
        ])))
        .mount(&server)
        .await;
    let pack = pack();
    let s = settings(AiProviderKind::OpenaiCompatible, &format!("{}/v1", server.uri()), Some("m"));
    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Summarize, summarize("x"), |_| None)).await;
    let AaspEvent::Done(done) = events.last().unwrap() else { panic!("{events:?}") };
    assert_eq!(done.object, Some(json!({"tldr": "ok", "bullets": ["a"], "asks": [], "deadlines": []})));
    assert_eq!(done.usage.input_tokens, 84, "usage of both attempts");
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 2);
    let second: Value = reqs[1].body_json().unwrap();
    assert_eq!(second["messages"][2]["role"], "assistant");
    assert_eq!(second["messages"][2]["content"], "{\"tldr\": 5}");
    assert!(second["messages"][3]["content"].as_str().unwrap().contains("did not match"));

    // Two bad answers: the stream ends with an error.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&["not json"])))
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::OpenaiCompatible, &format!("{}/v1", server.uri()), Some("m"));
    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Summarize, summarize("x"), |_| None)).await;
    assert!(matches!(events.last(), Some(AaspEvent::Error(e)) if e.code == ErrorCode::ProviderInvalid));
}

#[tokio::test]
async fn http_errors_map_to_registry_codes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string("{\"error\":{\"message\":\"Incorrect API key sk-abc\"}}"),
        )
        .mount(&server)
        .await;
    let pack = pack();
    let s = settings(AiProviderKind::OpenaiCompatible, &format!("{}/v1", server.uri()), Some("m"));
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("x"), |_| None)).await;
    let Some(AaspEvent::Error(e)) = events.last() else { panic!() };
    assert_eq!(e.code, ErrorCode::Unauthenticated);
    assert!(!e.message.contains("sk-abc"), "provider text is never surfaced");

    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(503)).mount(&server).await;
    let s = settings(AiProviderKind::OpenaiCompatible, &format!("{}/v1", server.uri()), Some("m"));
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("x"), |_| None)).await;
    assert!(
        matches!(events.last(), Some(AaspEvent::Error(e)) if e.code == ErrorCode::ProviderUnavailable && e.retryable)
    );

    // Unreachable host.
    let s = settings(AiProviderKind::OpenaiCompatible, "http://127.0.0.1:9/v1", Some("m"));
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("x"), |_| None)).await;
    assert!(matches!(events.last(), Some(AaspEvent::Error(e)) if e.code == ErrorCode::ProviderUnavailable));
}

#[tokio::test]
async fn preflight_failures_never_call_the_network() {
    let pack = pack();
    let mut s = settings(AiProviderKind::Anthropic, "https://api.anthropic.com", None);
    // Missing key: the registry's client-side `key_missing` (D-396), as in packages/ai.
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("x"), |_| None)).await;
    assert!(matches!(events.as_slice(), [AaspEvent::Error(e)] if e.code == ErrorCode::KeyMissing));
    // Invalid vars.
    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, vars(json!({"wrong": "x"})), |_| {
            Some("k".into())
        }))
        .await;
    assert!(matches!(events.as_slice(), [AaspEvent::Error(e)] if e.code == ErrorCode::InvalidVars));
    // No consent.
    s.consent.granted_at = None;
    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("x"), |_| Some("k".into()))).await;
    assert!(matches!(events.as_slice(), [AaspEvent::Error(e)] if e.code == ErrorCode::ConsentRequired));
}

#[tokio::test]
async fn managed_path_passes_aasp_through() {
    let server = MockServer::start().await;
    let aasp = "event: meta\ndata: {\"feature\":\"summarize\",\"model\":\"anthropic/claude-sonnet-5.5\",\"promptVersion\":1}\n\n\
                event: partial\ndata: {\"object\":{\"tldr\":\"Hi\"}}\n\n\
                event: done\ndata: {\"text\":null,\"object\":{\"tldr\":\"Hi\",\"bullets\":[]},\"usage\":{\"inputTokens\":1,\"outputTokens\":2},\"finishReason\":\"stop\"}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/ai/summarize"))
        .and(header("authorization", "Bearer user-jwt"))
        .and(body_partial_json(
            json!({"promptVersion": 1, "vars": {"transcript": "x", "userName": "Sam", "today": "2026-10-20"}}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(aasp))
        .expect(1)
        .mount(&server)
        .await;
    let pack = pack();
    // The synced profile names another origin (a poisoned setting, D-415): it must never see the
    // JWT or the transcript. Only the device's configured apps/api origin does.
    let evil = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&evil).await;
    let managed = || AiClient::new().with_managed_api(Some(server.uri().replace("127.0.0.1", "localhost")));
    let s = settings(AiProviderKind::Akatsuki, &evil.uri().replace("127.0.0.1", "localhost"), None);
    let events =
        collect(run_feature(&managed(), &pack, &s, AiFeature::Summarize, summarize("x"), |_| Some("user-jwt".into())))
            .await;
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(evil.received_requests().await.unwrap().is_empty(), "the profile's baseUrl is ignored");
    // Without configuration the managed provider goes to the default origin, never the profile's.
    let p = akatsuki_ai::Provider::for_profile(&s.profiles[0], Some("user-jwt".into())).unwrap();
    assert!(matches!(&p, akatsuki_ai::Provider::Akatsuki { base_url, .. } if base_url == "https://api.akatsuki.vc"));
    // A configured origin that is not https (or local http) is ignored too.
    assert_eq!(
        AiClient::new().with_managed_api(Some("http://evil.example".into())).managed_api(),
        "https://api.akatsuki.vc"
    );
    assert!(matches!(&events[2], AaspEvent::Done(d) if d.object == Some(json!({"tldr": "Hi", "bullets": []}))));

    // Envelope errors keep their code.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .set_body_json(json!({"error": {"code": "ai_budget_exhausted", "message": "x", "retryable": false}})),
        )
        .mount(&server)
        .await;
    let client = AiClient::new().with_managed_api(Some(server.uri().replace("127.0.0.1", "localhost")));
    let events =
        collect(run_feature(&client, &pack, &s, AiFeature::Summarize, summarize("x"), |_| Some("user-jwt".into())))
            .await;
    assert!(matches!(events.as_slice(), [AaspEvent::Error(e)] if e.code == ErrorCode::AiBudgetExhausted));
}

#[tokio::test]
async fn test_connection_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": []})))
        .expect(1)
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::Ollama, &format!("{}/v1", server.uri()), Some("m"));
    let resolved = resolve(AiFeature::Summarize, &s, &ROUTING).unwrap();
    let provider = Provider::for_profile(resolved.profile, None).unwrap();
    AiClient::new().test_connection(resolved.profile, &provider, &resolved.model).await.unwrap();
}

#[tokio::test]
async fn openrouter_byo_requests_carry_the_privacy_routing() {
    let server = MockServer::start().await;
    let parts = ["{\"tldr\": \"ok\", \"bullets\": [], \"asks\": [], \"deadlines\": []}"];
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-or-test-not-real"))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&parts)))
        .mount(&server)
        .await;
    let pack = pack();
    let s = settings(AiProviderKind::Openrouter, &format!("{}/api/v1", server.uri()), None);
    let fx: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/ai/fixtures/openrouter/request.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let key = |_: &str| Some("sk-or-test-not-real".to_owned());

    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Summarize, summarize("From: a"), key)).await;
    assert!(matches!(events.last(), Some(AaspEvent::Done(_))), "{events:?}");
    let events = collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Rewrite, rewrite("hi"), key)).await;
    assert!(matches!(events.last(), Some(AaspEvent::Done(_))), "{events:?}");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let summarize: Value = requests[0].body_json().unwrap();
    assert_eq!(summarize["model"], "anthropic/claude-sonnet-5.5");
    assert_eq!(summarize["provider"], fx["provider"]);
    assert_eq!(
        summarize["provider"],
        json!({"zdr": true, "data_collection": "deny", "allow_fallbacks": true, "require_parameters": true})
    );
    assert_eq!(summarize["reasoning"], json!({"effort": "low", "exclude": true}));
    let rewrite: Value = requests[1].body_json().unwrap();
    assert_eq!(rewrite["provider"], fx["provider"]);
    // Every feature in the pack has an effort hint; the no-effort body is covered by the unit test
    // against packages/ai/fixtures/openrouter/request.json.
    assert_eq!(rewrite["reasoning"], json!({"effort": "low", "exclude": true}));
    assert_eq!(requests[0].headers.get("x-title").unwrap(), "Akatsuki");
}

#[tokio::test]
async fn escalation_moves_the_repair_retry_to_the_heavy_candidate() {
    let server = MockServer::start().await;
    let valid = "{\"tldr\": \"ok\", \"bullets\": [], \"asks\": [], \"deadlines\": []}";
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(body_partial_json(json!({"model": "anthropic/claude-sonnet-5.5"})))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&["{\"tldr\": 5}"])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/chat/completions"))
        .and(body_partial_json(json!({"model": "anthropic/claude-opus-5.5"})))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&[valid])))
        .expect(1)
        .mount(&server)
        .await;
    let pack = pack();
    let s = settings(AiProviderKind::Openrouter, &format!("{}/api/v1", server.uri()), None);
    let key = |_: &str| Some("sk-or-test-not-real".to_owned());
    let events = collect(run_feature_with(
        &AiClient::new(),
        &pack,
        &s,
        AiFeature::Summarize,
        summarize("x"),
        key,
        Escalation::OnInvalidOutput,
    ))
    .await;
    let metas: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            AaspEvent::Meta(m) => Some(m.model.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(metas, ["anthropic/claude-sonnet-5.5", "anthropic/claude-opus-5.5"]);
    assert!(matches!(events.last(), Some(AaspEvent::Done(d)) if d.object.as_ref().unwrap()["tldr"] == "ok"));
    let second: Value = server.received_requests().await.unwrap()[1].body_json().unwrap();
    assert_eq!(second["provider"]["zdr"], true, "the heavier model keeps the privacy routing");
    assert!(second["messages"][3]["content"].as_str().unwrap().contains("did not match"));

    // Ollama has no escalation candidates: the retry stays on the same model.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string(openai_chunks(&["nope"])))
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::Ollama, &format!("{}/v1", server.uri()), Some("qwen3"));
    let events = collect(run_feature_with(
        &AiClient::new(),
        &pack,
        &s,
        AiFeature::Summarize,
        summarize("x"),
        |_| None,
        Escalation::OnInvalidOutput,
    ))
    .await;
    assert_eq!(events.iter().filter(|e| matches!(e, AaspEvent::Meta(_))).count(), 1);
    let models: Vec<Value> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.body_json::<Value>().unwrap()["model"].clone())
        .collect();
    assert_eq!(models, [json!("qwen3"), json!("qwen3")]);
}

#[tokio::test]
async fn dictation_is_not_a_prompt_feature() {
    let pack = pack();
    let s = settings(AiProviderKind::OpenaiCompatible, "http://127.0.0.1:9/v1", Some("m"));
    let events =
        collect(run_feature(&AiClient::new(), &pack, &s, AiFeature::Dictation, vars(json!({})), |_| None)).await;
    assert!(matches!(events.as_slice(), [AaspEvent::Error(_)]));
}
