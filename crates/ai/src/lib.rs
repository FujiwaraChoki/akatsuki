//! # akatsuki-ai
//!
//! The Rust AI client for the desktop app (ARCHITECTURE §12). It mirrors `packages/ai` (the AI SDK
//! registry used by web, mobile and apps/api) so behaviour matches on every client:
//!
//! - [`settings`]: the provider settings model, the routing table embedded from
//!   `packages/ai/routing.json` (with its escalation candidates and prices), model resolution,
//!   escalation, and the egress label shown under every AI action.
//! - [`prompts`]: the shared prompt pack in `packages/prompts` (plain files, embedded in the
//!   binary) and its `{{var}}` renderer.
//! - [`transcript`]: the shared transcript and triage-item renderers.
//! - [`client`]: streaming calls to OpenAI-compatible endpoints (OpenAI, OpenRouter, Google,
//!   AI Gateway, Ollama `/v1`, LM Studio, llama.cpp, vLLM), Anthropic's Messages API, and the
//!   managed path through apps/api, all producing AASP events.
//! - [`transcribe`]: dictation through apps/api (managed), OpenAI or a local Whisper server.
//!
//! Keys never leave the device: the caller reads them from the OS keychain and passes them in.

pub mod client;
pub mod error;
pub mod partial_json;
pub mod prompts;
pub mod settings;
pub mod sse;
pub mod transcribe;
pub mod transcript;

pub use akatsuki_core::wire::aasp::AaspEvent;
pub use client::{AiClient, Provider};
pub use error::AiError;
pub use prompts::{PromptPack, RenderedPrompt};
pub use settings::{
    Egress, Escalation, ROUTING, Resolved, Routing, Tier, egress, egress_text, ensure_consent, resolve,
    resolve_with_tier, validate_settings,
};
pub use transcribe::{TranscribeOptions, TranscriptionResult, TranscriptionTarget, dictate};

use futures::stream::{self, BoxStream, StreamExt};
use settings::{AiFeature, AiSettings};

/// `runFeature(feature, vars, settings)` (§12.6): checks consent, resolves the route, then runs
/// the feature on the user's provider, or through apps/api for the managed provider. `key_for`
/// returns the device key for a profile id (or the user's JWT for the managed profile). The stream
/// always ends with one `done` or `error` event. Never escalates; see [`run_feature_with`].
pub fn run_feature(
    client: &AiClient,
    pack: &PromptPack,
    settings: &AiSettings,
    feature: AiFeature,
    vars: serde_json::Map<String, serde_json::Value>,
    key_for: impl FnOnce(&str) -> Option<String>,
) -> BoxStream<'static, AaspEvent> {
    run_feature_with(client, pack, settings, feature, vars, key_for, Escalation::Never)
}

/// [`run_feature`] with an escalation policy (D-120, `escalation` in `packages/ai` `runFeature`).
/// With [`Escalation::OnInvalidOutput`] the repair retry after a schema-invalid answer runs on
/// the first heavy candidate for the profile's kind (Opus 5.5 for Anthropic, OpenRouter and AI
/// Gateway). Managed profiles never escalate on the client. Dictation is not a prompt feature:
/// use [`transcribe::dictate`].
pub fn run_feature_with(
    client: &AiClient,
    pack: &PromptPack,
    settings: &AiSettings,
    feature: AiFeature,
    vars: serde_json::Map<String, serde_json::Value>,
    key_for: impl FnOnce(&str) -> Option<String>,
    escalation: Escalation,
) -> BoxStream<'static, AaspEvent> {
    let fail = |e: AiError| stream::iter([AaspEvent::Error(e.to_stream_error())]).boxed();
    if let Err(e) = ensure_consent(settings) {
        return fail(e);
    }
    if feature == AiFeature::Dictation {
        return fail(AiError::PromptPack("dictation has no prompt; use transcribe::dictate".into()));
    }
    let resolved = match resolve(feature, settings, &ROUTING) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let provider = match client.provider_for(resolved.profile, key_for(&resolved.profile.profile_id)) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    if let Provider::Akatsuki { base_url, jwt } = &provider {
        let version = match pack.manifest.features.get(feature.as_str()) {
            Some(v) => v.version,
            None => return fail(AiError::PromptPack(format!("no version for {}", feature.as_str()))),
        };
        return client.run_managed(base_url, jwt, feature.as_str(), version, vars);
    }
    match pack.render(feature, &vars) {
        Ok(prompt) => client.run_with(provider, &resolved, &prompt, escalation.retry_model(&ROUTING, &resolved)),
        Err(e) => fail(e),
    }
}
