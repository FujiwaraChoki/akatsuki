//! Dictation against a local mock HTTP server: apps/api (managed), OpenAI (BYO key) and a local
//! OpenAI-compatible Whisper server. No real keys, no network.

use akatsuki_ai::settings::{
    AiConsent, AiFeature, AiModels, AiProfile, AiProviderKind, AiRoute, AiRouteFeature, AiSettings, Egress,
    MANAGED_DICTATION_EGRESS_TEXT,
};
use akatsuki_ai::transcribe::{TranscribeOptions, TranscriptionResult, dictate};
use akatsuki_ai::{AiClient, AiError, ROUTING, resolve};
use akatsuki_core::wire::ErrorCode;
use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn settings(kind: AiProviderKind, base_url: Option<String>, model: Option<&str>) -> AiSettings {
    AiSettings {
        profiles: vec![AiProfile {
            profile_id: "p1".into(),
            label: "Dictation".into(),
            kind,
            base_url,
            models: AiModels { transcription: model.map(str::to_owned), ..Default::default() },
            key_on_devices: vec!["dev1".into()],
        }],
        routes: vec![AiRoute {
            feature: AiRouteFeature::Feature(AiFeature::Dictation),
            profile_id: "p1".into(),
            model: None,
            enabled: true,
        }],
        consent: AiConsent { granted_at: Some(1), version: Some(1), auto_triage: false, sync_voice_profile: false },
        ..Default::default()
    }
}

/// The `name="<field>"` part of a multipart body, as text.
fn part(req: &Request, field: &str) -> Option<String> {
    let body = String::from_utf8_lossy(&req.body).into_owned();
    let marker = format!("name=\"{field}\"");
    let start = body.find(&marker)?;
    let rest = &body[start..];
    let value_start = rest.find("\r\n\r\n")? + 4;
    let value_end = rest[value_start..].find("\r\n--")?;
    Some(rest[value_start..value_start + value_end].to_owned())
}

fn audio() -> Vec<u8> {
    // Not real audio: the providers are mocks. Bytes that are not UTF-8 check the body is binary-safe.
    let mut a = b"OggS".to_vec();
    a.extend([0xff, 0xfe, 0x00, 0x80]);
    a
}

#[tokio::test]
async fn managed_dictation_goes_through_apps_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/ai/transcribe"))
        .and(header("authorization", "Bearer user-jwt"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"text": "Hallo Mia", "durationSec": 4.2, "language": "de"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let base = server.uri().replace("127.0.0.1", "localhost");
    // The managed profile carries no origin of its own; the device configuration names apps/api.
    let s = settings(AiProviderKind::Akatsuki, None, None);
    let client = AiClient::new().with_managed_api(Some(base));
    let resolved = resolve(AiFeature::Dictation, &s, &ROUTING).unwrap();
    assert_eq!(resolved.egress(), Egress::AkatsukiOpenai);
    assert_eq!(resolved.egress_text(), MANAGED_DICTATION_EGRESS_TEXT);

    let opts = TranscribeOptions { language: Some("de-CH".into()), prompt: Some("Mia, Akatsuki ".repeat(60)) };
    let got = dictate(&client, &s, &audio(), "audio/webm;codecs=opus", &opts, |id| {
        assert_eq!(id, "p1");
        Some("user-jwt".into())
    })
    .await
    .unwrap();
    assert_eq!(got, TranscriptionResult { text: "Hallo Mia".into(), duration_sec: 4.2, language: Some("de".into()) });

    let req = &server.received_requests().await.unwrap()[0];
    let ct = req.headers.get("content-type").unwrap().to_str().unwrap().to_owned();
    assert!(ct.starts_with("multipart/form-data; boundary="), "{ct}");
    let body = String::from_utf8_lossy(&req.body);
    assert!(body.contains("name=\"audio\"; filename=\"audio.webm\"\r\nContent-Type: audio/webm\r\n"));
    assert!(req.body.windows(audio().len()).any(|w| w == audio().as_slice()), "audio bytes are sent unchanged");
    assert_eq!(part(req, "language").as_deref(), Some("de-CH"), "apps/api takes the BCP-47 tag");
    assert_eq!(part(req, "prompt").unwrap().chars().count(), 500, "the hint is cut to 500 characters");
    assert!(part(req, "model").is_none(), "the managed provider picks the model");
}

#[tokio::test]
async fn managed_errors_keep_the_envelope_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "3600")
                .set_body_json(json!({"error": {"code": "rate_limited", "message": "Dictation is limited to 120 minutes a day.", "retryable": true}})),
        )
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::Akatsuki, Some("https://evil.example".into()), None);
    let client = AiClient::new().with_managed_api(Some(server.uri().replace("127.0.0.1", "localhost")));
    let err = dictate(&client, &s, &audio(), "audio/ogg", &TranscribeOptions::default(), |_| Some("user-jwt".into()))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, AiError::Provider { code: ErrorCode::RateLimited, retry_after_ms: Some(3_600_000), .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn openai_byo_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .and(header("authorization", "Bearer sk-test-not-real"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"text": "Hallo", "usage": {"type": "tokens"}})))
        .expect(1)
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::Openai, Some(format!("{}/v1", server.uri())), None);
    let resolved = resolve(AiFeature::Dictation, &s, &ROUTING).unwrap();
    assert_eq!(resolved.model, "gpt-4o-transcribe", "routing.json transcription tier");
    assert_eq!(resolved.egress(), Egress::Provider);
    let opts = TranscribeOptions { language: Some("de-CH".into()), prompt: None };
    let got =
        dictate(&AiClient::new(), &s, &audio(), "audio/mp4", &opts, |_| Some("sk-test-not-real".into())).await.unwrap();
    assert_eq!(got, TranscriptionResult { text: "Hallo".into(), duration_sec: 0.0, language: None });

    let req = &server.received_requests().await.unwrap()[0];
    assert_eq!(part(req, "model").as_deref(), Some("gpt-4o-transcribe"));
    assert_eq!(part(req, "response_format").as_deref(), Some("json"), "gpt-4o models answer json only");
    assert_eq!(part(req, "language").as_deref(), Some("de"), "OpenAI takes ISO 639-1");
    assert!(part(req, "prompt").is_none());
    assert!(String::from_utf8_lossy(&req.body).contains("filename=\"audio.m4a\""));

    // Without a key nothing is sent.
    let err = dictate(&AiClient::new(), &s, &audio(), "audio/mp4", &opts, |_| None).await.unwrap_err();
    assert_eq!(err, AiError::KeyMissing);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn local_whisper_server_without_a_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/audio/transcriptions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"text": "hi there", "duration": 3.5, "language": "english"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::OpenaiCompatible, Some(format!("{}/v1", server.uri())), Some("whisper-large-v3"));
    let resolved = resolve(AiFeature::Dictation, &s, &ROUTING).unwrap();
    assert_eq!(resolved.egress(), Egress::Device, "a loopback Whisper server keeps audio on the device");
    let got =
        dictate(&AiClient::new(), &s, &audio(), "audio/wav", &TranscribeOptions::default(), |_| None).await.unwrap();
    assert_eq!(got, TranscriptionResult { text: "hi there".into(), duration_sec: 3.5, language: Some("en".into()) });
    let req = &server.received_requests().await.unwrap()[0];
    assert!(req.headers.get("authorization").is_none());
    assert_eq!(part(req, "model").as_deref(), Some("whisper-large-v3"));
    assert_eq!(part(req, "response_format").as_deref(), Some("verbose_json"));
}

#[tokio::test]
async fn refusals_never_call_the_network() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(500)).expect(0).mount(&server).await;
    let local = || Some(format!("{}/v1", server.uri()));
    let client = AiClient::new();
    let none = TranscribeOptions::default();

    // A provider without a transcription API.
    let s = settings(AiProviderKind::Anthropic, None, Some("whatever"));
    let err = dictate(&client, &s, &audio(), "audio/wav", &none, |_| Some("k".into())).await.unwrap_err();
    assert_eq!(err, AiError::Unsupported);
    assert_eq!(err.to_stream_error().code, ErrorCode::Unsupported);
    // No consent.
    let mut s = settings(AiProviderKind::OpenaiCompatible, local(), Some("w"));
    s.consent.granted_at = None;
    assert_eq!(dictate(&client, &s, &audio(), "audio/wav", &none, |_| None).await, Err(AiError::ConsentRequired));
    // A model is required for openai_compatible.
    let s = settings(AiProviderKind::OpenaiCompatible, local(), None);
    assert_eq!(dictate(&client, &s, &audio(), "audio/wav", &none, |_| None).await, Err(AiError::ModelRequired));
    // Bad input.
    let s = settings(AiProviderKind::OpenaiCompatible, local(), Some("w"));
    for (bytes, media, opts) in [
        (audio(), "text/plain", none.clone()),
        (Vec::new(), "audio/wav", none.clone()),
        (audio(), "audio/wav", TranscribeOptions { language: Some("english please".into()), prompt: None }),
    ] {
        let err = dictate(&client, &s, &bytes, media, &opts, |_| None).await.unwrap_err();
        assert!(matches!(err, AiError::InvalidAudio(_)), "{err:?}");
    }
    let big = vec![0u8; akatsuki_ai::transcribe::AUDIO_MAX_BYTES + 1];
    assert_eq!(dictate(&client, &s, &big, "audio/wav", &none, |_| None).await, Err(AiError::PayloadTooLarge));
}

#[tokio::test]
async fn provider_errors_never_surface_provider_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401).set_body_string("{\"error\":{\"message\":\"Incorrect API key sk-abc\"}}"),
        )
        .mount(&server)
        .await;
    let s = settings(AiProviderKind::Openai, Some(format!("{}/v1", server.uri())), None);
    let err = dictate(&AiClient::new(), &s, &audio(), "audio/wav", &TranscribeOptions::default(), |_| {
        Some("sk-wrong".into())
    })
    .await
    .unwrap_err();
    assert_eq!(err.to_stream_error().code, ErrorCode::Unauthenticated);
    assert!(!format!("{err:?} {err}").contains("sk-abc"));
}
