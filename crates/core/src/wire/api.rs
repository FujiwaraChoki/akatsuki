//! apps/api HTTP API types (`packages/wire/src/api.ts`, ARCHITECTURE §8).

use serde::{Deserialize, Serialize};

use crate::convex::TriageResult;
use crate::convex::rows::TriageInput;
use crate::time::UnixMs;

/// `POST /v1/ai/{feature}` request. Raw prompts are never accepted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiFeatureRequest {
    /// Must equal `manifest.features[feature].version`.
    pub prompt_version: u32,
    /// Template variables, validated against `input.schema.json`.
    pub vars: serde_json::Map<String, serde_json::Value>,
}

/// `POST /v1/ai/transcribe` response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeResponse {
    /// Transcript.
    pub text: String,
    /// Audio duration.
    pub duration_sec: f64,
    /// Detected language, or `null`.
    pub language: Option<String>,
}

/// `GET /v1/ai/quota` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaResponse {
    /// `"YYYY-MM"` (UTC).
    pub period: String,
    /// Used micro-dollars.
    pub used_micros: u64,
    /// Limit micro-dollars.
    pub limit_micros: u64,
    /// When the budget resets.
    pub resets_at: UnixMs,
}

/// `POST /v1/billing/checkout` and `/portal` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlResponse {
    /// Stripe URL.
    pub url: String,
}

/// `POST /v1/billing/cancel` request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRequest {
    /// Only `true` during account deletion.
    pub immediately: bool,
}

/// `POST /internal/v1/triage` request (worker → apps/api).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalTriageRequest {
    /// The user the budget applies to.
    pub user_id: String,
    /// Headers and snippets (≤ 25).
    pub items: Vec<TriageInput>,
}

/// `POST /internal/v1/triage` response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalTriageResponse {
    /// Results.
    pub results: Vec<TriageResult>,
}
