//! Provider settings, the routing table and model resolution (ARCHITECTURE §12.2–§12.3).
//!
//! The settings model is `aiSettings` in Convex, mirrored by `packages/ai/schema/ai-settings.schema.json`
//! and the serde types re-exported here. Resolution matches `packages/ai/src/resolve.ts` and is
//! checked against `packages/ai/fixtures/resolve/*.json`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

pub use akatsuki_core::convex::rows::{AiConsent, AiModels, AiProfile, AiRoute, AiRouteFeature, AiSettings};
pub use akatsuki_core::convex::schema::{AiFeature, AiProviderKind};

use crate::AiError;

/// Model tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Cheap, high volume (triage, NL search).
    Fast,
    /// Most features.
    Default,
    /// Only where an eval shows a gain.
    Heavy,
    /// Speech to text.
    Transcription,
}

/// List prices in micro-dollars (`ModelPrice` in `packages/ai/src/routing.ts`, D-120).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPrice {
    /// Input tokens, per million.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_per_m_tok_micros: Option<u64>,
    /// Output tokens, per million.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_per_m_tok_micros: Option<u64>,
    /// Audio, per minute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_audio_minute_micros: Option<u64>,
}

/// `packages/ai/routing.json`, embedded at build time: the only routing table (§12.3). There is
/// no Rust copy of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routing {
    /// Table version.
    pub version: u32,
    /// Feature → tier.
    pub features: BTreeMap<AiFeature, Tier>,
    /// Tier → provider kind (`routing.json` column) → model id.
    pub tiers: BTreeMap<Tier, BTreeMap<String, String>>,
    /// Escalation candidates per tier and provider kind, cheapest first (D-120).
    #[serde(default)]
    pub escalation: BTreeMap<Tier, BTreeMap<String, Vec<String>>>,
    /// List prices keyed by the managed model id.
    #[serde(default)]
    pub pricing: BTreeMap<String, ModelPrice>,
}

/// The routing table's JSON as embedded from `packages/ai/routing.json`.
pub const ROUTING_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/routing.json"));

/// The embedded routing table (see `build.rs`).
pub static ROUTING: LazyLock<Routing> = LazyLock::new(|| {
    serde_json::from_str(ROUTING_JSON).unwrap_or_else(|e| {
        // The build embeds a file that is checked in and tested; failing here is a packaging bug.
        panic!("embedded routing.json is invalid: {e}")
    })
});

impl Routing {
    /// The tier of a feature.
    pub fn tier(&self, feature: AiFeature) -> Option<Tier> {
        self.features.get(&feature).copied()
    }

    /// The default model of a tier for a provider kind.
    pub fn model(&self, tier: Tier, kind: AiProviderKind) -> Option<&str> {
        self.tiers.get(&tier).and_then(|t| t.get(kind.as_str())).map(String::as_str)
    }

    /// Heavier models for a tier and provider kind, cheapest first (`escalationCandidates`).
    pub fn escalation_candidates(&self, tier: Tier, kind: AiProviderKind) -> &[String] {
        self.escalation.get(&tier).and_then(|t| t.get(kind.as_str())).map(Vec::as_slice).unwrap_or_default()
    }
}

/// When a BYO request may move to a heavier model (the `EscalationHook` of `packages/ai`, D-120).
/// The managed path never escalates on the client: apps/api picks its model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Escalation {
    /// Never (`NO_ESCALATION`, the default).
    #[default]
    Never,
    /// Retry one schema-invalid structured answer on the first heavy candidate for the provider
    /// kind (`escalateOnInvalidOutput`). Kinds without candidates (OpenAI, Google, Ollama,
    /// OpenAI-compatible) and the transcription tier do not escalate.
    OnInvalidOutput,
}

impl Escalation {
    /// The model the repair retry uses after an invalid answer, if this policy escalates
    /// `resolved`. Fable 5.1 is only the second candidate, so it is never chosen here.
    pub fn retry_model(self, routing: &Routing, resolved: &Resolved<'_>) -> Option<String> {
        match self {
            Escalation::Never => None,
            Escalation::OnInvalidOutput if resolved.tier == Tier::Transcription || resolved.is_managed() => None,
            Escalation::OnInvalidOutput => {
                routing.escalation_candidates(Tier::Heavy, resolved.profile.kind).first().cloned()
            }
        }
    }
}

/// The outcome of resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// Feature.
    pub feature: AiFeature,
    /// Tier.
    pub tier: Tier,
    /// The profile to call.
    pub profile: &'a AiProfile,
    /// The model id. For `akatsuki` the server picks; this is the routing table's value.
    pub model: String,
}

impl Resolved<'_> {
    /// Managed (`akatsuki`) profiles send only `{feature, promptVersion, vars}` to apps/api.
    pub fn is_managed(&self) -> bool {
        self.profile.kind == AiProviderKind::Akatsuki
    }

    /// The egress label of the resolved profile for this feature.
    pub fn egress(&self) -> Egress {
        egress(self.profile, AiRouteFeature::Feature(self.feature))
    }

    /// The UI copy of [`Self::egress`].
    pub fn egress_text(&self) -> String {
        egress_text(self.profile, AiRouteFeature::Feature(self.feature))
    }
}

fn tier_model(models: &AiModels, tier: Tier) -> Option<&str> {
    match tier {
        Tier::Fast => models.fast.as_deref(),
        Tier::Default => models.default.as_deref(),
        Tier::Heavy => models.heavy.as_deref(),
        Tier::Transcription => models.transcription.as_deref(),
    }
}

/// Resolves a feature (§12.3), the twin of `resolveFeature` in `packages/ai/src/resolve.ts`:
///
/// 1. the enabled route for the feature, else the enabled `default` route; none → off;
/// 2. its profile; missing → off;
/// 3. `route.model ?? profile.models[tier] ?? routing.tiers[tier][profile.kind]`, else
///    `ai_model_required`.
pub fn resolve<'a>(feature: AiFeature, settings: &'a AiSettings, routing: &Routing) -> Result<Resolved<'a>, AiError> {
    resolve_with_tier(feature, settings, routing, None)
}

/// [`resolve`] with an explicit tier (escalation overrides the table tier).
pub fn resolve_with_tier<'a>(
    feature: AiFeature,
    settings: &'a AiSettings,
    routing: &Routing,
    tier: Option<Tier>,
) -> Result<Resolved<'a>, AiError> {
    let route = settings
        .routes
        .iter()
        .find(|r| r.enabled && r.feature == AiRouteFeature::Feature(feature))
        .or_else(|| settings.routes.iter().find(|r| r.enabled && r.feature == AiRouteFeature::DEFAULT))
        .ok_or(AiError::FeatureOff)?;
    let profile = settings.profiles.iter().find(|p| p.profile_id == route.profile_id).ok_or(AiError::FeatureOff)?;
    let tier = tier.or_else(|| routing.tier(feature)).ok_or(AiError::FeatureOff)?;
    let model = route
        .model
        .as_deref()
        .or_else(|| tier_model(&profile.models, tier))
        .or_else(|| routing.model(tier, profile.kind))
        .ok_or(AiError::ModelRequired)?;
    Ok(Resolved { feature, tier, profile, model: model.to_owned() })
}

/// Fails unless consent was granted (§12.2: no AI feature runs before `consent.grantedAt`).
pub fn ensure_consent(settings: &AiSettings) -> Result<(), AiError> {
    if settings.consent.granted_at.is_some() { Ok(()) } else { Err(AiError::ConsentRequired) }
}

/// What leaves the device when a profile runs (the egress label under every AI action, §12.2;
/// `Egress` in `packages/ai/src/settings.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Egress {
    /// A loopback endpoint: "Stays on this device".
    Device,
    /// An RFC 1918 or `.local` endpoint: "Stays on your local network (<host>)".
    Network,
    /// A BYO cloud provider: "Leaves this device: <what the feature sends>, sent to <host>".
    Provider,
    /// Managed text features: "Sent to Akatsuki for processing, not stored".
    Akatsuki,
    /// Managed dictation, forwarded to OpenAI without zero data retention (D-128):
    /// [`MANAGED_DICTATION_EGRESS_TEXT`].
    AkatsukiOpenai,
}

/// The managed dictation label (D-128): it names OpenAI and its abuse-monitoring retention.
pub const MANAGED_DICTATION_EGRESS_TEXT: &str =
    "Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring";

/// Loopback only: `localhost`, a 127.0.0.0/8 literal or `::1` (`isLoopbackHost`).
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    h == "localhost" || h == "::1" || h.parse::<std::net::Ipv4Addr>().is_ok_and(|v4| v4.is_loopback())
}

/// An RFC 1918 IPv4 literal or an mDNS `.local` name (`isPrivateNetworkHost`).
pub fn is_private_network_host(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    h.ends_with(".local") || h.parse::<std::net::Ipv4Addr>().is_ok_and(|v4| v4.is_private())
}

/// True for loopback, RFC 1918 and `.local` hosts, exactly like `isLocalHost` in
/// `packages/ai/src/settings.ts`.
pub fn is_local_host(host: &str) -> bool {
    is_loopback_host(host) || is_private_network_host(host)
}

/// Hosts named in the egress label when a profile has no `baseUrl`.
fn default_host(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::Openrouter => "openrouter.ai",
        AiProviderKind::Anthropic => "api.anthropic.com",
        AiProviderKind::Openai => "api.openai.com",
        AiProviderKind::Google => "generativelanguage.googleapis.com",
        AiProviderKind::Gateway => "ai-gateway.vercel.sh",
        other => other.as_str(),
    }
}

fn parsed_host(base: Option<&str>) -> Option<(String, String)> {
    let url = url::Url::parse(base?).ok()?;
    let host = url.host_str()?.to_owned();
    let with_port = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.clone(),
    };
    Some((host, with_port))
}

/// The egress label of a profile when it runs `feature` (`egressFor`). Managed dictation is
/// [`Egress::AkatsukiOpenai`]; local endpoints are [`Egress::Device`] on loopback and
/// [`Egress::Network`] on an RFC 1918 or `.local` host.
pub fn egress(profile: &AiProfile, feature: AiRouteFeature) -> Egress {
    let base = profile.base_url.as_deref();
    match profile.kind {
        AiProviderKind::Akatsuki if feature == AiRouteFeature::Feature(AiFeature::Dictation) => Egress::AkatsukiOpenai,
        AiProviderKind::Akatsuki => Egress::Akatsuki,
        AiProviderKind::Ollama | AiProviderKind::OpenaiCompatible => match parsed_host(base) {
            // Ollama without a baseUrl talks to its default, http://127.0.0.1:11434.
            None if base.is_none() && profile.kind == AiProviderKind::Ollama => Egress::Device,
            Some((host, _)) if is_loopback_host(&host) => Egress::Device,
            Some((host, _)) if is_private_network_host(&host) => Egress::Network,
            _ => Egress::Provider,
        },
        _ => Egress::Provider,
    }
}

/// What a feature sends, for the "Leaves this device" label (`EGRESS_WHAT`).
fn egress_what(feature: AiRouteFeature) -> &'static str {
    match feature {
        AiRouteFeature::Default(_) => "the mail you run AI on",
        AiRouteFeature::Feature(f) => match f {
            AiFeature::Summarize | AiFeature::InstantReply | AiFeature::ExtractActions => "the thread you run this on",
            AiFeature::Compose => "your instruction, your draft and the thread you reply to",
            AiFeature::Rewrite => "the text you rewrite",
            AiFeature::Triage => "the headers and snippets of new mail",
            AiFeature::NlSearch => "your search and your label and split names",
            AiFeature::VoiceProfile => "samples of your sent mail",
            AiFeature::Dictation => "your recording",
        },
    }
}

/// The UI copy of the egress label (`egressText`), byte-identical to TypeScript
/// (`packages/ai/fixtures/egress/cases.json`).
pub fn egress_text(profile: &AiProfile, feature: AiRouteFeature) -> String {
    let host = parsed_host(profile.base_url.as_deref()).map(|(_, with_port)| with_port);
    match egress(profile, feature) {
        Egress::Device => "Stays on this device".to_owned(),
        Egress::Akatsuki => "Sent to Akatsuki for processing, not stored".to_owned(),
        Egress::AkatsukiOpenai => MANAGED_DICTATION_EGRESS_TEXT.to_owned(),
        Egress::Network => format!("Stays on your local network ({})", host.unwrap_or_default()),
        Egress::Provider => {
            let host = host.unwrap_or_else(|| default_host(profile.kind).to_owned());
            format!("Leaves this device: {}, sent to {host}", egress_what(feature))
        }
    }
}

/// `baseUrlProblem`: https anywhere, http only for local hosts; no userinfo, query or fragment.
pub fn base_url_problem(value: &str) -> Option<&'static str> {
    let Ok(url) = url::Url::parse(value) else { return Some("must be an absolute URL") };
    if !url.username().is_empty() || url.password().is_some() {
        return Some("must not contain a user name or password");
    }
    if url.query().is_some() || value.contains('?') {
        return Some("must not contain a query string");
    }
    if url.fragment().is_some() {
        return Some("must not contain a fragment");
    }
    match url.scheme() {
        "https" => None,
        "http" if url.host_str().is_some_and(is_local_host) => None,
        _ => Some("must use https (http is allowed only for localhost and private network addresses)"),
    }
}

/// Default API base URLs per provider kind (no trailing slash).
pub fn default_base_url(kind: AiProviderKind) -> Option<&'static str> {
    Some(match kind {
        AiProviderKind::Akatsuki => "https://api.akatsuki.vc",
        AiProviderKind::Openrouter => "https://openrouter.ai/api/v1",
        AiProviderKind::Anthropic => "https://api.anthropic.com",
        AiProviderKind::Openai => "https://api.openai.com/v1",
        AiProviderKind::Google => "https://generativelanguage.googleapis.com/v1beta/openai",
        AiProviderKind::Gateway => "https://ai-gateway.vercel.sh/v1",
        AiProviderKind::Ollama => "http://127.0.0.1:11434",
        AiProviderKind::OpenaiCompatible => return None,
    })
}

/// The endpoint base the Rust client calls for a profile, validated with [`base_url_problem`].
/// Ollama profiles store the server origin (as `packages/ai` does); the client uses its
/// OpenAI-compatible `/v1` API (DECISIONS D-093).
pub fn base_url(profile: &AiProfile) -> Result<url::Url, AiError> {
    let raw = profile.base_url.as_deref().or_else(|| default_base_url(profile.kind)).ok_or(AiError::InvalidBaseUrl)?;
    let raw = raw.trim().trim_end_matches('/');
    if base_url_problem(raw).is_some() {
        return Err(AiError::InvalidBaseUrl);
    }
    let raw = if profile.kind == AiProviderKind::Ollama {
        format!("{}/v1", raw.trim_end_matches("/api").trim_end_matches("/v1"))
    } else {
        raw.to_owned()
    };
    url::Url::parse(&raw).map_err(|_| AiError::InvalidBaseUrl)
}

mod strict {
    //! Strict mirrors of the settings model, used only for validation (unknown keys are errors,
    //! like the zod `strictObject`s in `packages/ai/src/settings.ts`).
    use serde::Deserialize;

    use super::{AiFeature, AiProviderKind};

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    pub(super) struct Settings {
        pub profiles: Vec<Profile>,
        pub routes: Vec<Route>,
        pub consent: Consent,
        pub voice_profile: Option<String>,
        #[allow(dead_code)]
        pub updated_at: Option<f64>,
        #[serde(rename = "_id")]
        #[allow(dead_code)]
        pub id: Option<String>,
        #[serde(rename = "_creationTime")]
        #[allow(dead_code)]
        pub creation_time: Option<f64>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    pub(super) struct Profile {
        pub profile_id: String,
        pub label: String,
        pub kind: AiProviderKind,
        pub base_url: Option<String>,
        pub models: Models,
        pub key_on_devices: Vec<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct Models {
        pub fast: Option<String>,
        pub default: Option<String>,
        pub heavy: Option<String>,
        pub transcription: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    pub(super) enum Feature {
        Feature(#[allow(dead_code)] AiFeature),
        Default(#[allow(dead_code)] DefaultLiteral),
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub(super) enum DefaultLiteral {
        Default,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    pub(super) struct Route {
        #[allow(dead_code)]
        pub feature: Feature,
        pub profile_id: String,
        pub model: Option<String>,
        #[allow(dead_code)]
        pub enabled: bool,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    pub(super) struct Consent {
        #[allow(dead_code)]
        pub granted_at: Option<f64>,
        #[allow(dead_code)]
        pub version: Option<f64>,
        #[allow(dead_code)]
        pub auto_triage: bool,
        pub sync_voice_profile: bool,
    }
}

/// Validates settings JSON with the same rules as `zAiSettings` in `packages/ai` (strict keys,
/// limits, base-URL rules, known route profiles, voice-profile consent) and parses it.
pub fn validate_settings(value: &serde_json::Value) -> Result<AiSettings, String> {
    let s: strict::Settings = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    if s.profiles.len() > 10 {
        return Err("at most 10 profiles".into());
    }
    if s.routes.len() > 20 {
        return Err("at most 20 routes".into());
    }
    let model_ok = |m: &Option<String>| m.as_ref().is_none_or(|m| !m.is_empty() && m.chars().count() <= 200);
    let mut ids = std::collections::BTreeSet::new();
    for p in &s.profiles {
        let id_ok = (1..=64).contains(&p.profile_id.len())
            && p.profile_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !id_ok {
            return Err("profileId: letters, digits and hyphens only".into());
        }
        if !ids.insert(p.profile_id.clone()) {
            return Err("duplicate profileId".into());
        }
        if p.label.is_empty() || p.label.chars().count() > 80 {
            return Err("label must be 1 to 80 characters".into());
        }
        if let Some(b) = &p.base_url {
            if b.chars().count() > 500 {
                return Err("baseUrl is too long".into());
            }
            if let Some(problem) = base_url_problem(b) {
                return Err(format!("baseUrl {problem}"));
            }
        }
        if p.kind == AiProviderKind::OpenaiCompatible && p.base_url.is_none() {
            return Err("openai_compatible profiles need a baseUrl".into());
        }
        if p.kind == AiProviderKind::Akatsuki && p.base_url.is_some() {
            return Err("the managed provider has no baseUrl".into());
        }
        let m = &p.models;
        if ![&m.fast, &m.default, &m.heavy, &m.transcription].into_iter().all(model_ok) {
            return Err("model ids are 1 to 200 characters".into());
        }
        if p.key_on_devices.len() > 50 || p.key_on_devices.iter().any(|d| d.is_empty() || d.len() > 64) {
            return Err("keyOnDevices is invalid".into());
        }
    }
    for r in &s.routes {
        if !ids.contains(&r.profile_id) {
            return Err("unknown profileId in a route".into());
        }
        if !model_ok(&r.model) {
            return Err("model ids are 1 to 200 characters".into());
        }
    }
    if let Some(v) = &s.voice_profile {
        if v.chars().count() > 2048 {
            return Err("voiceProfile is at most 2048 characters".into());
        }
        if !s.consent.sync_voice_profile {
            return Err("voiceProfile is only synced with consent.syncVoiceProfile".into());
        }
    }
    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, kind: AiProviderKind) -> AiProfile {
        AiProfile {
            profile_id: id.into(),
            label: id.into(),
            kind,
            base_url: None,
            models: AiModels::default(),
            key_on_devices: vec![],
        }
    }

    fn route(feature: AiRouteFeature, profile: &str, model: Option<&str>, enabled: bool) -> AiRoute {
        AiRoute { feature, profile_id: profile.into(), model: model.map(str::to_owned), enabled }
    }

    #[test]
    fn embedded_routing_table() {
        assert_eq!(ROUTING.tier(AiFeature::Triage), Some(Tier::Fast));
        assert_eq!(ROUTING.tier(AiFeature::Summarize), Some(Tier::Default));
        assert_eq!(ROUTING.model(Tier::Default, AiProviderKind::Anthropic), Some("claude-sonnet-5-5"));
        assert_eq!(ROUTING.model(Tier::Fast, AiProviderKind::Openrouter), Some("anthropic/claude-haiku-5.5"));
        assert_eq!(ROUTING.model(Tier::Default, AiProviderKind::Ollama), None);
        assert_eq!(
            ROUTING.escalation_candidates(Tier::Heavy, AiProviderKind::Anthropic),
            ["claude-opus-5-5", "claude-fable-5-1"]
        );
        assert!(ROUTING.escalation_candidates(Tier::Heavy, AiProviderKind::Ollama).is_empty());
        assert_eq!(ROUTING.pricing["gpt-4o-transcribe"].per_audio_minute_micros, Some(6000));
        for tier in [Tier::Fast, Tier::Default, Tier::Heavy, Tier::Transcription] {
            let model = ROUTING.model(tier, AiProviderKind::Akatsuki).unwrap();
            assert!(ROUTING.pricing.contains_key(model), "{model} has a list price");
        }
    }

    #[test]
    fn routing_is_the_shared_file_byte_for_byte() {
        let shared = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../packages/ai/routing.json"));
        assert_eq!(ROUTING_JSON, shared.unwrap());
        // No second copy of the table exists in the crate (§12.3: model ids live in one file).
        assert!(!std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/routing.default.json")).exists());
    }

    #[test]
    fn escalation_policy() {
        let mut anthropic = profile("a", AiProviderKind::Anthropic);
        anthropic.models.default = Some("claude-sonnet-5-5".into());
        let s = AiSettings {
            profiles: vec![anthropic, profile("o", AiProviderKind::Ollama), profile("m", AiProviderKind::Akatsuki)],
            routes: vec![
                route(AiRouteFeature::DEFAULT, "a", None, true),
                route(AiRouteFeature::Feature(AiFeature::Triage), "o", Some("qwen3"), true),
                route(AiRouteFeature::Feature(AiFeature::NlSearch), "m", None, true),
            ],
            ..Default::default()
        };
        let summarize = resolve(AiFeature::Summarize, &s, &ROUTING).unwrap();
        assert_eq!(Escalation::Never.retry_model(&ROUTING, &summarize), None);
        assert_eq!(Escalation::OnInvalidOutput.retry_model(&ROUTING, &summarize).as_deref(), Some("claude-opus-5-5"));
        let triage = resolve(AiFeature::Triage, &s, &ROUTING).unwrap();
        assert_eq!(Escalation::OnInvalidOutput.retry_model(&ROUTING, &triage), None, "no candidates for Ollama");
        let managed = resolve(AiFeature::NlSearch, &s, &ROUTING).unwrap();
        assert_eq!(Escalation::OnInvalidOutput.retry_model(&ROUTING, &managed), None, "apps/api picks the model");
    }

    #[test]
    fn resolution_order() {
        let mut ollama = profile("local", AiProviderKind::Ollama);
        ollama.models.fast = Some("llama3.3".into());
        let s = AiSettings {
            profiles: vec![profile("or", AiProviderKind::Openrouter), ollama],
            routes: vec![
                route(AiRouteFeature::DEFAULT, "or", None, true),
                route(AiRouteFeature::Feature(AiFeature::Triage), "local", None, true),
                route(AiRouteFeature::Feature(AiFeature::Compose), "or", Some("openai/gpt-5"), true),
                route(AiRouteFeature::Feature(AiFeature::Rewrite), "local", None, false),
            ],
            ..Default::default()
        };
        let r = resolve(AiFeature::Summarize, &s, &ROUTING).unwrap();
        assert_eq!(
            (r.profile.profile_id.as_str(), r.model.as_str(), r.tier),
            ("or", "anthropic/claude-sonnet-5.5", Tier::Default)
        );
        let r = resolve(AiFeature::Triage, &s, &ROUTING).unwrap();
        assert_eq!(r.model, "llama3.3");
        assert_eq!(resolve(AiFeature::Compose, &s, &ROUTING).unwrap().model, "openai/gpt-5");
        // A disabled feature route falls back to default.
        assert_eq!(resolve(AiFeature::Rewrite, &s, &ROUTING).unwrap().profile.profile_id, "or");
    }

    #[test]
    fn resolution_errors() {
        let s = AiSettings::default();
        assert_eq!(resolve(AiFeature::Summarize, &s, &ROUTING), Err(AiError::FeatureOff));
        let s = AiSettings {
            profiles: vec![profile("o", AiProviderKind::Google)],
            routes: vec![route(AiRouteFeature::DEFAULT, "o", None, true)],
            ..Default::default()
        };
        assert_eq!(resolve(AiFeature::Summarize, &s, &ROUTING), Err(AiError::ModelRequired));
        let s = AiSettings { routes: vec![route(AiRouteFeature::DEFAULT, "nope", None, true)], ..Default::default() };
        assert_eq!(resolve(AiFeature::Summarize, &s, &ROUTING), Err(AiError::FeatureOff));
        assert_eq!(ensure_consent(&s), Err(AiError::ConsentRequired));
    }

    #[test]
    fn egress_labels() {
        let dictation = AiRouteFeature::Feature(AiFeature::Dictation);
        let summarize = AiRouteFeature::Feature(AiFeature::Summarize);
        assert_eq!(egress(&profile("a", AiProviderKind::Akatsuki), summarize), Egress::Akatsuki);
        assert_eq!(egress(&profile("a", AiProviderKind::Akatsuki), dictation), Egress::AkatsukiOpenai);
        assert_eq!(egress(&profile("o", AiProviderKind::Ollama), summarize), Egress::Device);
        let mut lm = profile("lm", AiProviderKind::OpenaiCompatible);
        lm.base_url = Some("http://192.168.1.20:1234/v1".into());
        assert_eq!(egress(&lm, dictation), Egress::Network);
        assert_eq!(egress_text(&lm, dictation), "Stays on your local network (192.168.1.20:1234)");
        lm.base_url = Some("https://llm.example.com:8443/v1".into());
        assert_eq!(egress(&lm, summarize), Egress::Provider);
        assert_eq!(
            egress_text(&lm, summarize),
            "Leaves this device: the thread you run this on, sent to llm.example.com:8443"
        );
        lm.base_url = Some("not a url".into());
        assert_eq!(egress(&lm, summarize), Egress::Provider);
        assert!(is_local_host("printer.local") && is_local_host("[::1]") && is_local_host("172.20.0.5"));
        assert!(!is_local_host("172.32.0.1") && !is_local_host("example.com"));
        assert!(is_loopback_host("127.0.0.1") && !is_loopback_host("10.0.0.1"));
        assert!(is_private_network_host("10.0.0.1") && !is_private_network_host("localhost"));
    }

    #[test]
    fn shared_egress_cases() {
        // Same cases as `egressFor` / `egressText` in packages/ai (§12.2).
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/ai/fixtures/egress/cases.json");
        let fx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = fx["cases"].as_array().unwrap();
        assert!(cases.len() >= 20);
        for case in cases {
            let kind: AiProviderKind = serde_json::from_value(case["profile"]["kind"].clone()).unwrap();
            let mut p = profile("p", kind);
            p.base_url = case["profile"]["baseUrl"].as_str().map(str::to_owned);
            let feature: AiRouteFeature = serde_json::from_value(case["feature"].clone()).unwrap();
            assert_eq!(serde_json::to_value(egress(&p, feature)).unwrap(), case["egress"], "{case}");
            assert_eq!(egress_text(&p, feature), case["text"].as_str().unwrap(), "{case}");
        }
    }

    #[test]
    fn base_url_rules() {
        let mut p = profile("x", AiProviderKind::OpenaiCompatible);
        assert_eq!(base_url(&p), Err(AiError::InvalidBaseUrl));
        for bad in [
            "http://example.com/v1",
            "https://user:pw@example.com/v1",
            "https://example.com/v1?key=x",
            "https://example.com/#x",
            "ftp://x/",
        ] {
            p.base_url = Some(bad.into());
            assert_eq!(base_url(&p), Err(AiError::InvalidBaseUrl), "{bad}");
        }
        for good in
            ["http://localhost:1234/v1", "http://127.0.0.1:8080/v1", "http://[::1]:11434/v1", "https://example.com/v1"]
        {
            p.base_url = Some(good.into());
            assert!(base_url(&p).is_ok(), "{good}");
        }
        let mut o = profile("o", AiProviderKind::Ollama);
        assert_eq!(base_url(&o).unwrap().as_str(), "http://127.0.0.1:11434/v1");
        o.base_url = Some("http://localhost:11434/api".into());
        assert_eq!(base_url(&o).unwrap().as_str(), "http://localhost:11434/v1");
    }

    fn fixtures(dir: &str) -> Vec<(String, serde_json::Value)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/ai/fixtures").join(dir);
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let mut out: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .map(|e| {
                (
                    e.path().display().to_string(),
                    serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap(),
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        assert!(!out.is_empty(), "no shared fixtures found");
        out
    }

    #[test]
    fn shared_resolve_fixtures() {
        for (name, case) in fixtures("resolve") {
            let settings: AiSettings =
                serde_json::from_value(case["settings"].clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let feature = AiFeature::from_name(case["feature"].as_str().unwrap()).unwrap();
            let tier: Option<Tier> = case.get("tier").map(|t| serde_json::from_value(t.clone()).unwrap());
            let expect = &case["expect"];
            let got = resolve_with_tier(feature, &settings, &ROUTING, tier);
            match expect["status"].as_str().unwrap() {
                "off" => assert_eq!(got, Err(AiError::FeatureOff), "{name}"),
                "error" => {
                    assert_eq!(expect["code"], "ai_model_required", "{name}");
                    assert_eq!(got, Err(AiError::ModelRequired), "{name}");
                }
                "ok" => {
                    let r = got.unwrap_or_else(|e| panic!("{name}: {e:?}"));
                    assert_eq!(serde_json::to_value(r.tier).unwrap(), expect["tier"], "{name}");
                    assert_eq!(r.profile.profile_id, expect["profileId"].as_str().unwrap(), "{name}");
                    assert_eq!(r.model, expect["model"].as_str().unwrap(), "{name}");
                    assert_eq!(r.is_managed(), expect["managed"].as_bool().unwrap(), "{name}");
                    assert_eq!(serde_json::to_value(r.egress()).unwrap(), expect["egress"], "{name}");
                }
                other => panic!("{name}: unknown status {other}"),
            }
        }
    }

    #[test]
    fn shared_base_url_cases() {
        // Same cases as `baseUrlProblem` (packages/ai) and `checkBaseUrl` (convex/ai.ts), D-129.
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/ai/fixtures/base-url/cases.json");
        let fx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = fx["cases"].as_array().unwrap();
        assert!(cases.len() >= 30);
        for case in cases {
            let url = case["url"].as_str().unwrap();
            assert_eq!(base_url_problem(url).is_none(), case["ok"].as_bool().unwrap(), "{url}");
            let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            assert_eq!(is_local_host(&host), case["local"].as_bool().unwrap(), "{url} ({host})");
        }
    }

    #[test]
    fn shared_settings_fixtures() {
        for (name, case) in fixtures("settings") {
            let valid = case["valid"].as_bool().unwrap();
            let got = validate_settings(&case["settings"]);
            assert_eq!(got.is_ok(), valid, "{name}: {got:?}");
        }
    }
}
