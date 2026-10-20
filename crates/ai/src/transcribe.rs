//! Dictation (§12.1 `dictation`, tier `transcription`), the Rust twin of
//! `packages/ai/src/transcribe.ts`. Three targets:
//!
//! - **Managed** (Akatsuki Pro): `POST {api}/v1/ai/transcribe` on apps/api with the user's JWT
//!   (multipart `audio`, `language?`, `prompt?` → `{text, durationSec, language}`, §8). apps/api
//!   forwards the audio to OpenAI under OpenAI's standard data policy (D-128), which the egress
//!   label says ([`crate::settings::MANAGED_DICTATION_EGRESS_TEXT`]).
//! - **OpenAI** with the user's own key: `POST {base}/audio/transcriptions`
//!   (`https://api.openai.com/v1` unless the profile sets a `baseUrl`).
//! - **A local Whisper server** with an OpenAI-compatible `/v1/audio/transcriptions` endpoint
//!   (whisper.cpp server, faster-whisper-server, LocalAI, …) as an `openai_compatible` profile; the
//!   key is optional, so audio can stay on the device or the local network.
//!
//! Requests are multipart bodies built here (no extra HTTP features). Provider response text is
//! never surfaced in errors.

use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use akatsuki_core::wire::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AiError;
use crate::client::AiClient;
use crate::settings::{AiFeature, AiProviderKind, AiSettings, ROUTING, Resolved, base_url, ensure_consent, resolve};

/// OpenAI transcription models (`OPENAI_TRANSCRIPTION_MODELS` in `packages/ai`).
pub const OPENAI_TRANSCRIPTION_MODELS: [&str; 3] = ["gpt-4o-transcribe", "gpt-4o-mini-transcribe", "whisper-1"];
/// Largest upload (§8: ≤ 25 MB).
pub const AUDIO_MAX_BYTES: usize = 25 * 1024 * 1024;
/// Longest recording (§8: ≤ 10 min).
pub const AUDIO_MAX_SECONDS: u32 = 600;
/// Longest vocabulary hint, in characters.
pub const PROMPT_HINT_MAX: usize = 500;
/// Accepted audio media types (`AUDIO_MEDIA_TYPES`).
pub const AUDIO_MEDIA_TYPES: [&str; 10] = [
    "audio/webm",
    "audio/ogg",
    "audio/mp4",
    "audio/m4a",
    "audio/x-m4a",
    "audio/wav",
    "audio/x-wav",
    "audio/wave",
    "audio/mpeg",
    "audio/mp3",
];

/// Whisper's language names → ISO 639-1 (the table `@ai-sdk/openai` applies to `verbose_json`).
const WHISPER_LANGUAGES: [(&str, &str); 57] = [
    ("afrikaans", "af"),
    ("arabic", "ar"),
    ("armenian", "hy"),
    ("azerbaijani", "az"),
    ("belarusian", "be"),
    ("bosnian", "bs"),
    ("bulgarian", "bg"),
    ("catalan", "ca"),
    ("chinese", "zh"),
    ("croatian", "hr"),
    ("czech", "cs"),
    ("danish", "da"),
    ("dutch", "nl"),
    ("english", "en"),
    ("estonian", "et"),
    ("finnish", "fi"),
    ("french", "fr"),
    ("galician", "gl"),
    ("german", "de"),
    ("greek", "el"),
    ("hebrew", "he"),
    ("hindi", "hi"),
    ("hungarian", "hu"),
    ("icelandic", "is"),
    ("indonesian", "id"),
    ("italian", "it"),
    ("japanese", "ja"),
    ("kannada", "kn"),
    ("kazakh", "kk"),
    ("korean", "ko"),
    ("latvian", "lv"),
    ("lithuanian", "lt"),
    ("macedonian", "mk"),
    ("malay", "ms"),
    ("marathi", "mr"),
    ("maori", "mi"),
    ("nepali", "ne"),
    ("norwegian", "no"),
    ("persian", "fa"),
    ("polish", "pl"),
    ("portuguese", "pt"),
    ("romanian", "ro"),
    ("russian", "ru"),
    ("serbian", "sr"),
    ("slovak", "sk"),
    ("slovenian", "sl"),
    ("spanish", "es"),
    ("swahili", "sw"),
    ("swedish", "sv"),
    ("tagalog", "tl"),
    ("tamil", "ta"),
    ("thai", "th"),
    ("turkish", "tr"),
    ("ukrainian", "uk"),
    ("urdu", "ur"),
    ("vietnamese", "vi"),
    ("welsh", "cy"),
];

/// Options for one transcription.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TranscribeOptions {
    /// BCP-47 hint, e.g. `"en"` or `"de-CH"`.
    pub language: Option<String>,
    /// Vocabulary hint (names, jargon); cut to [`PROMPT_HINT_MAX`] characters.
    pub prompt: Option<String>,
}

/// `{ text, durationSec, language }` (§8). `duration_sec` is 0 when the provider reports none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionResult {
    /// The transcript.
    pub text: String,
    /// Length of the recording in seconds.
    pub duration_sec: f64,
    /// Detected language (ISO 639-1), if known.
    pub language: Option<String>,
}

/// Where a recording is transcribed.
#[derive(Clone, PartialEq, Eq)]
pub enum TranscriptionTarget {
    /// apps/api `POST /v1/ai/transcribe` (the managed provider picks the model).
    Managed {
        /// apps/api base URL, e.g. `https://api.akatsuki.vc`.
        base_url: String,
        /// The user's Convex JWT.
        jwt: String,
    },
    /// An OpenAI-style `POST {base}/audio/transcriptions` endpoint.
    OpenAiCompatible {
        /// Base URL including the version segment, e.g. `https://api.openai.com/v1`.
        base_url: String,
        /// Bearer key; `None` for local servers.
        api_key: Option<String>,
        /// Model id, e.g. `gpt-4o-transcribe` or `whisper-large-v3`.
        model: String,
    },
}

impl std::fmt::Debug for TranscriptionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscriptionTarget::Managed { base_url, .. } => {
                f.debug_struct("Managed").field("base_url", base_url).finish_non_exhaustive()
            }
            TranscriptionTarget::OpenAiCompatible { base_url, model, .. } => f
                .debug_struct("OpenAiCompatible")
                .field("base_url", base_url)
                .field("model", model)
                .finish_non_exhaustive(),
        }
    }
}

impl TranscriptionTarget {
    /// The target for a resolved `dictation` route (`transcriptionModelFor`). `key` comes from the
    /// device keychain, or is the user's JWT for the managed profile. Kinds without a
    /// transcription API (Anthropic, Google, OpenRouter, AI Gateway, Ollama) are
    /// [`AiError::Unsupported`].
    pub fn for_resolved(resolved: &Resolved<'_>, key: Option<String>) -> Result<Self, AiError> {
        let profile = resolved.profile;
        let key = key.filter(|k| !k.trim().is_empty());
        let base = || base_url(profile).map(|u| u.as_str().trim_end_matches('/').to_owned());
        match profile.kind {
            // Never `profile.base_url` (synced data; it would receive the JWT, D-415).
            AiProviderKind::Akatsuki => Ok(TranscriptionTarget::Managed {
                base_url: crate::client::DEFAULT_MANAGED_API.to_owned(),
                jwt: key.ok_or(AiError::KeyMissing)?,
            }),
            AiProviderKind::Openai => Ok(TranscriptionTarget::OpenAiCompatible {
                base_url: base()?,
                api_key: Some(key.ok_or(AiError::KeyMissing)?),
                model: resolved.model.clone(),
            }),
            AiProviderKind::OpenaiCompatible => Ok(TranscriptionTarget::OpenAiCompatible {
                base_url: base()?,
                api_key: key,
                model: resolved.model.clone(),
            }),
            _ => Err(AiError::Unsupported),
        }
    }
}

/// The base media type, lowercased and without parameters (`audio/webm;codecs=opus` → `audio/webm`).
fn base_media_type(media_type: &str) -> String {
    media_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

/// The file extension the upload is named with (`mediaTypeToExtension` in the AI SDK).
pub fn audio_extension(media_type: &str) -> &'static str {
    match base_media_type(media_type).as_str() {
        "audio/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        _ => "bin",
    }
}

/// A BCP-47 tag as apps/api accepts it (`^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$`).
pub fn is_bcp47(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let first = parts.next().unwrap_or("");
    (2..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Checks a recording before anything is sent.
pub fn check_audio(audio: &[u8], media_type: &str, opts: &TranscribeOptions) -> Result<(), AiError> {
    if audio.is_empty() {
        return Err(AiError::InvalidAudio("The recording is empty."));
    }
    if audio.len() > AUDIO_MAX_BYTES {
        return Err(AiError::PayloadTooLarge);
    }
    if !AUDIO_MEDIA_TYPES.contains(&base_media_type(media_type).as_str()) {
        return Err(AiError::InvalidAudio("This audio format is not supported."));
    }
    if opts.language.as_deref().is_some_and(|l| !is_bcp47(l)) {
        return Err(AiError::InvalidAudio("The language is not a valid language tag."));
    }
    Ok(())
}

fn cut_hint(prompt: &str) -> String {
    prompt.chars().take(PROMPT_HINT_MAX).collect()
}

/// A Whisper language name or code → ISO 639-1, if known.
pub fn language_code(language: &str) -> Option<String> {
    let l = language.trim().to_ascii_lowercase();
    if let Some((_, code)) = WHISPER_LANGUAGES.iter().find(|(name, _)| *name == l) {
        return Some((*code).to_owned());
    }
    ((2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase())).then_some(l)
}

/// A `multipart/form-data` body.
struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        // Per-process random keys plus a counter: the boundary never repeats and cannot be guessed
        // from the audio, so a recording cannot contain it by accident.
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        let a = h.finish();
        h.write_u64(a);
        Self { boundary: format!("akatsuki-{a:016x}{:016x}", h.finish()), body: Vec::new() }
    }

    fn head(&mut self, disposition: &str, content_type: Option<&str>) {
        self.body.extend_from_slice(
            format!("--{}\r\nContent-Disposition: form-data; {disposition}\r\n", self.boundary).as_bytes(),
        );
        if let Some(ct) = content_type {
            self.body.extend_from_slice(format!("Content-Type: {ct}\r\n").as_bytes());
        }
        self.body.extend_from_slice(b"\r\n");
    }

    fn text(&mut self, name: &str, value: &str) {
        self.head(&format!("name=\"{name}\""), None);
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
    }

    fn file(&mut self, name: &str, filename: &str, content_type: &str, bytes: &[u8]) {
        self.head(&format!("name=\"{name}\"; filename=\"{filename}\""), Some(content_type));
        self.body.extend_from_slice(bytes);
        self.body.extend_from_slice(b"\r\n");
    }

    fn finish(mut self) -> (String, Vec<u8>) {
        self.body.extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (format!("multipart/form-data; boundary={}", self.boundary), self.body)
    }
}

fn transport_error() -> AiError {
    AiError::Provider {
        code: ErrorCode::ProviderUnavailable,
        message: "The transcription service could not be reached.".into(),
        retry_after_ms: None,
    }
}

fn retry_after(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|s| s * 1000)
}

/// OpenAI's transcription response (`json` or `verbose_json`).
#[derive(Deserialize)]
struct OpenAiTranscription {
    text: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    language: Option<String>,
}

impl AiClient {
    /// Transcribes one recording on `target`. `media_type` is the recording's type (for example
    /// `audio/webm;codecs=opus`). Fails before any request when the audio is empty, too large, of
    /// an unsupported type, or the language tag is invalid.
    pub async fn transcribe(
        &self,
        target: &TranscriptionTarget,
        audio: &[u8],
        media_type: &str,
        opts: &TranscribeOptions,
    ) -> Result<TranscriptionResult, AiError> {
        check_audio(audio, media_type, opts)?;
        let media = base_media_type(media_type);
        let filename = format!("audio.{}", audio_extension(&media));
        let mut form = Multipart::new();
        let req = match target {
            TranscriptionTarget::Managed { base_url, jwt } => {
                form.file("audio", &filename, &media, audio);
                if let Some(l) = &opts.language {
                    form.text("language", l);
                }
                if let Some(p) = opts.prompt.as_deref().filter(|p| !p.is_empty()) {
                    form.text("prompt", &cut_hint(p));
                }
                let (content_type, body) = form.finish();
                self.http()
                    .post(format!("{}/v1/ai/transcribe", base_url.trim_end_matches('/')))
                    .bearer_auth(jwt)
                    .header("content-type", content_type)
                    .body(body)
            }
            TranscriptionTarget::OpenAiCompatible { base_url, api_key, model } => {
                form.text("model", model);
                form.file("file", &filename, &media, audio);
                // gpt-4o transcription models only answer `json`; Whisper models also give the
                // duration and language with `verbose_json` (as `@ai-sdk/openai` asks).
                let gpt4o = model == "gpt-4o-transcribe" || model == "gpt-4o-mini-transcribe";
                form.text("response_format", if gpt4o { "json" } else { "verbose_json" });
                if let Some(l) = &opts.language {
                    // OpenAI takes ISO 639-1, so `de-CH` is sent as `de`.
                    form.text("language", &l.split('-').next().unwrap_or(l).to_ascii_lowercase());
                }
                if let Some(p) = opts.prompt.as_deref().filter(|p| !p.is_empty()) {
                    form.text("prompt", &cut_hint(p));
                }
                let (content_type, body) = form.finish();
                let mut r = self
                    .http()
                    .post(format!("{}/audio/transcriptions", base_url.trim_end_matches('/')))
                    .header("content-type", content_type)
                    .body(body);
                if let Some(k) = api_key {
                    r = r.bearer_auth(k);
                }
                r
            }
        };
        let resp = req.header("accept", "application/json").send().await.map_err(|_| transport_error())?;
        let status = resp.status().as_u16();
        if !resp.status().is_success() {
            let ra = retry_after(&resp);
            let mut err = AiError::from_status(status, ra);
            if matches!(target, TranscriptionTarget::Managed { .. }) {
                // apps/api errors use the shared envelope; prefer its code (e.g. the daily cap).
                let body: Value = resp.json().await.unwrap_or(Value::Null);
                let code = body.pointer("/error/code").and_then(Value::as_str).and_then(ErrorCode::from_name);
                if let (Some(code), AiError::Provider { code: c, message, .. }) = (code, &mut err) {
                    *c = code;
                    *message = code.default_message().to_owned();
                }
            }
            return Err(err);
        }
        match target {
            TranscriptionTarget::Managed { .. } => {
                let r: TranscriptionResult = resp.json().await.map_err(|_| AiError::Stream)?;
                Ok(TranscriptionResult { language: r.language.as_deref().and_then(language_code), ..r })
            }
            TranscriptionTarget::OpenAiCompatible { .. } => {
                let r: OpenAiTranscription = resp.json().await.map_err(|_| AiError::Stream)?;
                Ok(TranscriptionResult {
                    text: r.text,
                    duration_sec: r.duration.filter(|d| d.is_finite() && *d > 0.0).unwrap_or(0.0),
                    language: r.language.as_deref().and_then(language_code),
                })
            }
        }
    }
}

/// Dictation on the user's `dictation` route (`dictate` in `packages/ai`, plus the managed path):
/// checks consent, resolves the route, then transcribes through apps/api, OpenAI or a local
/// Whisper server. `key_for` returns the device key for a profile id, or the user's JWT for the
/// managed profile.
pub async fn dictate(
    client: &AiClient,
    settings: &AiSettings,
    audio: &[u8],
    media_type: &str,
    opts: &TranscribeOptions,
    key_for: impl FnOnce(&str) -> Option<String>,
) -> Result<TranscriptionResult, AiError> {
    ensure_consent(settings)?;
    let resolved = resolve(AiFeature::Dictation, settings, &ROUTING)?;
    check_audio(audio, media_type, opts)?;
    let target = match TranscriptionTarget::for_resolved(&resolved, key_for(&resolved.profile.profile_id))? {
        TranscriptionTarget::Managed { jwt, .. } => {
            TranscriptionTarget::Managed { base_url: client.managed_api().to_owned(), jwt }
        }
        t => t,
    };
    client.transcribe(&target, audio, media_type, opts).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_types_and_tags() {
        assert_eq!(audio_extension("audio/webm;codecs=opus"), "webm");
        assert_eq!(audio_extension("audio/x-m4a"), "m4a");
        assert_eq!(audio_extension("audio/mpeg"), "mp3");
        assert!(is_bcp47("en") && is_bcp47("de-CH") && is_bcp47("zh-Hant-TW"));
        assert!(!is_bcp47("e") && !is_bcp47("english!") && !is_bcp47("en-") && !is_bcp47("en_GB"));
        assert_eq!(language_code("english").as_deref(), Some("en"));
        assert_eq!(language_code("de").as_deref(), Some("de"));
        assert_eq!(language_code("Klingon"), None);
    }

    #[test]
    fn audio_checks() {
        let o = TranscribeOptions::default();
        assert!(matches!(check_audio(&[], "audio/wav", &o), Err(AiError::InvalidAudio(_))));
        assert!(matches!(check_audio(&[1], "text/plain", &o), Err(AiError::InvalidAudio(_))));
        assert_eq!(check_audio(&vec![0; AUDIO_MAX_BYTES + 1], "audio/wav", &o), Err(AiError::PayloadTooLarge));
        let bad = TranscribeOptions { language: Some("en GB".into()), prompt: None };
        assert!(matches!(check_audio(&[1], "audio/wav", &bad), Err(AiError::InvalidAudio(_))));
        assert_eq!(check_audio(&[1], "audio/webm;codecs=opus", &o), Ok(()));
        assert_eq!(cut_hint(&"é".repeat(600)).chars().count(), PROMPT_HINT_MAX);
    }

    #[test]
    fn multipart_boundaries_differ() {
        let a = Multipart::new().finish().0;
        let b = Multipart::new().finish().0;
        assert_ne!(a, b);
        assert!(a.starts_with("multipart/form-data; boundary=akatsuki-"));
    }
}
