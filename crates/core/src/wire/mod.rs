//! Wire types shared with TypeScript (`packages/wire`, ARCHITECTURE §1.7):
//!
//! - [`ErrorCode`] and [`ErrorBody`]: the error registry and HTTP error envelope (§15).
//! - [`worker`]: sync worker HTTP API types (§7).
//! - [`api`]: apps/api HTTP API types (§8).
//! - [`aasp`]: the Akatsuki AI Stream Protocol (§12.6).

pub mod aasp;
pub mod api;
#[cfg(test)]
mod shared_fixtures;
pub mod worker;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// The error-code registry (§15). Serializes as the `snake_case` code string.
///
/// `NotImplemented` is an addition recorded in `docs/DECISIONS.md` (D-091): the sync worker
/// answers `501 not_implemented` on routes whose provider logic is not built yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// 401: missing, invalid or expired JWT.
    Unauthenticated,
    /// 404: missing or not owned.
    NotFound,
    /// 402: no `hostedSync` / `managedAi`.
    NotEntitled,
    /// 400: validation failed.
    InvalidArgument,
    /// 400: action not allowed for the account's caps or label keys.
    InvalidAction,
    /// 422: AI vars fail `input.schema.json`.
    InvalidVars,
    /// 413.
    PayloadTooLarge,
    /// 429, retryable, carries `retryAfterMs`.
    RateLimited,
    /// 429: monthly AI budget used up; `resetsAt` in the message.
    AiBudgetExhausted,
    /// Client-side: no model resolved for the feature.
    AiModelRequired,
    /// Client-side: AI has not been consented to (`consent.grantedAt` unset, §12.2).
    ConsentRequired,
    /// Client-side: the feature's route is switched off.
    AiOff,
    /// Client-side: the BYO profile has no key on this device.
    KeyMissing,
    /// 409: the client must update its prompt pack.
    PromptVersionMismatch,
    /// 409: the mailbox is linked to another Akatsuki user.
    AlreadyConnected,
    /// 409.
    AlreadySubscribed,
    /// 409: carries `version`.
    DraftVersionConflict,
    /// 409: worker-internal.
    LeaseLost,
    /// 409: secret compare-and-set failed.
    VersionConflict,
    /// 422: IMAP LOGIN rejected.
    ImapAuthFailed,
    /// 422: SMTP AUTH rejected.
    SmtpAuthFailed,
    /// 422: TLS handshake failed.
    TlsFailed,
    /// 422: host unreachable.
    Unreachable,
    /// 409: account credentials rejected.
    ReauthRequired,
    /// Op failed after 7 days in reauth.
    ReauthTimeout,
    /// 502, retryable.
    ProviderTransient,
    /// 503, retryable.
    ProviderRateLimited,
    /// 422.
    ProviderInvalid,
    /// 422.
    ProviderConflict,
    /// 507: mailbox full, send limit.
    ProviderQuota,
    /// 503, retryable: an upstream service is unavailable (the AI provider, or a worker-side provider or
    /// OAuth client that is not configured or not running).
    ProviderUnavailable,
    /// 400: e.g. `delete_forever` on Gmail.
    Unsupported,
    /// Send outcome: the draft was deleted elsewhere.
    DraftDeleted,
    /// Send outcome: a send-later more than 24 h overdue.
    StaleSchedule,
    /// Send outcome: could not confirm.
    SendUnconfirmed,
    /// 500, retryable: a bug, logged with a request id.
    Internal,
    /// 501: the route exists in the contract but is not implemented yet (D-091).
    NotImplemented,
}

impl ErrorCode {
    /// Every code, in registry order.
    pub const ALL: [ErrorCode; 37] = [
        ErrorCode::Unauthenticated,
        ErrorCode::NotFound,
        ErrorCode::NotEntitled,
        ErrorCode::InvalidArgument,
        ErrorCode::InvalidAction,
        ErrorCode::InvalidVars,
        ErrorCode::PayloadTooLarge,
        ErrorCode::RateLimited,
        ErrorCode::AiBudgetExhausted,
        ErrorCode::AiModelRequired,
        ErrorCode::ConsentRequired,
        ErrorCode::AiOff,
        ErrorCode::KeyMissing,
        ErrorCode::PromptVersionMismatch,
        ErrorCode::AlreadyConnected,
        ErrorCode::AlreadySubscribed,
        ErrorCode::DraftVersionConflict,
        ErrorCode::LeaseLost,
        ErrorCode::VersionConflict,
        ErrorCode::ImapAuthFailed,
        ErrorCode::SmtpAuthFailed,
        ErrorCode::TlsFailed,
        ErrorCode::Unreachable,
        ErrorCode::ReauthRequired,
        ErrorCode::ReauthTimeout,
        ErrorCode::ProviderTransient,
        ErrorCode::ProviderRateLimited,
        ErrorCode::ProviderInvalid,
        ErrorCode::ProviderConflict,
        ErrorCode::ProviderQuota,
        ErrorCode::ProviderUnavailable,
        ErrorCode::Unsupported,
        ErrorCode::DraftDeleted,
        ErrorCode::StaleSchedule,
        ErrorCode::SendUnconfirmed,
        ErrorCode::Internal,
        ErrorCode::NotImplemented,
    ];

    /// The wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Unauthenticated => "unauthenticated",
            ErrorCode::NotFound => "not_found",
            ErrorCode::NotEntitled => "not_entitled",
            ErrorCode::InvalidArgument => "invalid_argument",
            ErrorCode::InvalidAction => "invalid_action",
            ErrorCode::InvalidVars => "invalid_vars",
            ErrorCode::PayloadTooLarge => "payload_too_large",
            ErrorCode::RateLimited => "rate_limited",
            ErrorCode::AiBudgetExhausted => "ai_budget_exhausted",
            ErrorCode::AiModelRequired => "ai_model_required",
            ErrorCode::ConsentRequired => "consent_required",
            ErrorCode::AiOff => "ai_off",
            ErrorCode::KeyMissing => "key_missing",
            ErrorCode::PromptVersionMismatch => "prompt_version_mismatch",
            ErrorCode::AlreadyConnected => "already_connected",
            ErrorCode::AlreadySubscribed => "already_subscribed",
            ErrorCode::DraftVersionConflict => "draft_version_conflict",
            ErrorCode::LeaseLost => "lease_lost",
            ErrorCode::VersionConflict => "version_conflict",
            ErrorCode::ImapAuthFailed => "imap_auth_failed",
            ErrorCode::SmtpAuthFailed => "smtp_auth_failed",
            ErrorCode::TlsFailed => "tls_failed",
            ErrorCode::Unreachable => "unreachable",
            ErrorCode::ReauthRequired => "reauth_required",
            ErrorCode::ReauthTimeout => "reauth_timeout",
            ErrorCode::ProviderTransient => "provider_transient",
            ErrorCode::ProviderRateLimited => "provider_rate_limited",
            ErrorCode::ProviderInvalid => "provider_invalid",
            ErrorCode::ProviderConflict => "provider_conflict",
            ErrorCode::ProviderQuota => "provider_quota",
            ErrorCode::ProviderUnavailable => "provider_unavailable",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::DraftDeleted => "draft_deleted",
            ErrorCode::StaleSchedule => "stale_schedule",
            ErrorCode::SendUnconfirmed => "send_unconfirmed",
            ErrorCode::Internal => "internal",
            ErrorCode::NotImplemented => "not_implemented",
        }
    }

    /// Parses a wire string.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }

    /// The HTTP status for this code (§15). Codes the table marks "—" never travel over HTTP on
    /// their own; they map to 400 or 500 so a misuse is still well-formed.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::Unauthenticated => 401,
            ErrorCode::NotFound => 404,
            ErrorCode::NotEntitled => 402,
            ErrorCode::InvalidArgument | ErrorCode::InvalidAction | ErrorCode::Unsupported => 400,
            ErrorCode::InvalidVars => 422,
            ErrorCode::PayloadTooLarge => 413,
            ErrorCode::RateLimited | ErrorCode::AiBudgetExhausted => 429,
            ErrorCode::PromptVersionMismatch
            | ErrorCode::AlreadyConnected
            | ErrorCode::AlreadySubscribed
            | ErrorCode::DraftVersionConflict
            | ErrorCode::LeaseLost
            | ErrorCode::VersionConflict
            | ErrorCode::ReauthRequired => 409,
            ErrorCode::ImapAuthFailed
            | ErrorCode::SmtpAuthFailed
            | ErrorCode::TlsFailed
            | ErrorCode::Unreachable
            | ErrorCode::ProviderInvalid
            | ErrorCode::ProviderConflict => 422,
            ErrorCode::ProviderTransient => 502,
            ErrorCode::ProviderRateLimited | ErrorCode::ProviderUnavailable => 503,
            ErrorCode::ProviderQuota => 507,
            ErrorCode::Internal => 500,
            ErrorCode::NotImplemented => 501,
            ErrorCode::AiModelRequired
            | ErrorCode::ConsentRequired
            | ErrorCode::AiOff
            | ErrorCode::KeyMissing
            | ErrorCode::ReauthTimeout
            | ErrorCode::DraftDeleted
            | ErrorCode::StaleSchedule
            | ErrorCode::SendUnconfirmed => 400,
        }
    }

    /// Whether a retry may succeed (§15 "Retryable" column).
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorCode::RateLimited
                | ErrorCode::ProviderTransient
                | ErrorCode::ProviderRateLimited
                | ErrorCode::ProviderUnavailable
                | ErrorCode::Internal
        )
    }

    /// A safe, generic English message. Callers may pass a more specific one, which must still
    /// never contain provider text or user data.
    pub fn default_message(self) -> &'static str {
        match self {
            ErrorCode::Unauthenticated => "Sign in again.",
            ErrorCode::NotFound => "Not found.",
            ErrorCode::NotEntitled => "This needs an active Akatsuki Pro plan.",
            ErrorCode::InvalidArgument => "The request was not valid.",
            ErrorCode::InvalidAction => "That action is not available for this account.",
            ErrorCode::InvalidVars => "The AI request was not valid.",
            ErrorCode::PayloadTooLarge => "The request is too large.",
            ErrorCode::RateLimited => "Too many requests. Try again shortly.",
            ErrorCode::AiBudgetExhausted => "The monthly AI budget is used up.",
            ErrorCode::AiModelRequired => "Choose a model.",
            ErrorCode::ConsentRequired => "Turn on AI in Settings first.",
            ErrorCode::AiOff => "This AI feature is turned off.",
            ErrorCode::KeyMissing => "Add your API key on this device.",
            ErrorCode::PromptVersionMismatch => "Update the app to use this feature.",
            ErrorCode::AlreadyConnected => "This mailbox is connected to another Akatsuki account.",
            ErrorCode::AlreadySubscribed => "You already have a subscription.",
            ErrorCode::DraftVersionConflict => "This draft was changed on another device.",
            ErrorCode::LeaseLost => "Another worker owns this account.",
            ErrorCode::VersionConflict => "The stored credentials changed.",
            ErrorCode::ImapAuthFailed => "The IMAP server rejected the username or password.",
            ErrorCode::SmtpAuthFailed => "The SMTP server rejected the username or password.",
            ErrorCode::TlsFailed => "A secure connection could not be established.",
            ErrorCode::Unreachable => "The server could not be reached.",
            ErrorCode::ReauthRequired => "Reconnect this account.",
            ErrorCode::ReauthTimeout => "The account was not reconnected in time.",
            ErrorCode::ProviderTransient => "The mail provider had a temporary problem.",
            ErrorCode::ProviderRateLimited => "The mail provider is limiting requests.",
            ErrorCode::ProviderInvalid => "The mail provider rejected the request.",
            ErrorCode::ProviderConflict => "The mail provider reported a conflict.",
            ErrorCode::ProviderQuota => "The mailbox is full or a sending limit was reached.",
            ErrorCode::ProviderUnavailable => "The service is unavailable.",
            ErrorCode::Unsupported => "This is not supported for this account.",
            ErrorCode::DraftDeleted => "The draft was deleted.",
            ErrorCode::StaleSchedule => "The scheduled send was too far overdue.",
            ErrorCode::SendUnconfirmed => "We could not confirm this was sent.",
            ErrorCode::Internal => "Something went wrong.",
            ErrorCode::NotImplemented => "This is not available yet.",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error object inside [`ErrorBody`] (and the AASP `error` event payload).
#[skip_serializing_none]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    /// Code.
    pub code: ErrorCode,
    /// Safe English message.
    pub message: String,
    /// Retryable.
    pub retryable: bool,
    /// Retry delay for 429/503.
    pub retry_after_ms: Option<u64>,
    /// Current version, for `draft_version_conflict`.
    pub version: Option<u64>,
}

impl ErrorPayload {
    /// A payload with the code's default message and retryability.
    pub fn new(code: ErrorCode) -> Self {
        Self {
            code,
            message: code.default_message().to_owned(),
            retryable: code.retryable(),
            retry_after_ms: None,
            version: None,
        }
    }

    /// Replaces the message (must be safe to show).
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Sets `retryAfterMs`.
    pub fn with_retry_after_ms(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }
}

/// The HTTP error envelope: `{ "error": { "code", "message", "retryable", "retryAfterMs"? } }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// The error.
    pub error: ErrorPayload,
}

impl From<ErrorCode> for ErrorBody {
    fn from(code: ErrorCode) -> Self {
        Self { error: ErrorPayload::new(code) }
    }
}

/// `GET /healthz` body: `{"ok":true}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    /// Always `true`.
    pub ok: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_code_round_trips() {
        for code in ErrorCode::ALL {
            let wire = serde_json::to_value(code).unwrap();
            assert_eq!(wire, json!(code.as_str()));
            assert_eq!(ErrorCode::from_name(code.as_str()), Some(code));
            assert!(code.as_str().bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
        }
    }

    #[test]
    fn statuses_match_registry() {
        assert_eq!(ErrorCode::Unauthenticated.http_status(), 401);
        assert_eq!(ErrorCode::NotEntitled.http_status(), 402);
        assert_eq!(ErrorCode::ProviderQuota.http_status(), 507);
        assert_eq!(ErrorCode::NotImplemented.http_status(), 501);
        assert!(ErrorCode::RateLimited.retryable());
        assert!(!ErrorCode::AiBudgetExhausted.retryable());
    }

    /// The shared §15 registry snapshot (`packages/wire/fixtures/error-codes.json`, §1.7): same codes, same
    /// order, same HTTP status (`null` → 400 in Rust) and retryability on both sides.
    #[test]
    fn registry_fixture_matches() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/wire/fixtures/error-codes.json");
        let fx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let codes = fx["codes"].as_array().unwrap();
        assert_eq!(codes.len(), ErrorCode::ALL.len(), "registry size");
        for (row, code) in codes.iter().zip(ErrorCode::ALL) {
            let name = row["code"].as_str().unwrap();
            assert_eq!(code.as_str(), name, "registry order");
            assert_eq!(ErrorCode::from_name(name), Some(code));
            let http = row["http"].as_u64().map_or(400, |h| h as u16);
            assert_eq!(code.http_status(), http, "{name} http");
            assert_eq!(code.retryable(), row["retryable"].as_bool().unwrap(), "{name} retryable");
        }
    }

    #[test]
    fn envelope_shape() {
        let body = ErrorBody { error: ErrorPayload::new(ErrorCode::RateLimited).with_retry_after_ms(1500) };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({"error": {"code": "rate_limited", "message": "Too many requests. Try again shortly.", "retryable": true, "retryAfterMs": 1500}})
        );
    }
}
