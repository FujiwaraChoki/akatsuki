//! Errors from the AI client. Messages are safe to show and never contain provider response text.

use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::aasp::StreamError;

/// Errors from resolving, rendering or running a feature.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AiError {
    /// No enabled route for the feature or `default`.
    #[error("AI is off for this feature")]
    FeatureOff,
    /// No model resolved (`ai_model_required`, "Choose a model").
    #[error("Choose a model.")]
    ModelRequired,
    /// `consent.grantedAt` is not set.
    #[error("AI consent has not been given")]
    ConsentRequired,
    /// The device has no key for a provider that needs one.
    #[error("Key not set on this device.")]
    KeyMissing,
    /// The profile's base URL is not allowed (userinfo, query, unsupported scheme).
    #[error("invalid base URL")]
    InvalidBaseUrl,
    /// The prompt pack is missing or malformed.
    #[error("prompt pack: {0}")]
    PromptPack(String),
    /// The variables do not match `input.schema.json` (`invalid_vars`).
    #[error("invalid AI variables: {0}")]
    InvalidVars(String),
    /// The vars add up to more than the feature's `maxInputChars` (`payload_too_large`, as in
    /// `packages/ai` and apps/api).
    #[error("The input is too long for this feature.")]
    PayloadTooLarge,
    /// The provider answered with an error status.
    #[error("{code}: {message}")]
    Provider {
        /// Registry code.
        code: ErrorCode,
        /// Safe message.
        message: String,
        /// `Retry-After`, if sent.
        retry_after_ms: Option<u64>,
    },
    /// The response did not match the output schema after the repair retry.
    #[error("the AI response did not match the expected format")]
    InvalidOutput,
    /// The stream broke or could not be parsed.
    #[error("the AI stream was interrupted")]
    Stream,
    /// The provider has no API for this feature (dictation on a provider without transcription).
    #[error("This provider cannot transcribe audio. Choose OpenAI or a local Whisper server.")]
    Unsupported,
    /// The recording cannot be sent (empty, unsupported format, invalid language tag).
    #[error("{0}")]
    InvalidAudio(&'static str),
}

impl AiError {
    /// The registry code for AASP `error` events.
    pub fn code(&self) -> ErrorCode {
        match self {
            AiError::FeatureOff => ErrorCode::AiOff,
            AiError::ConsentRequired => ErrorCode::ConsentRequired,
            AiError::InvalidBaseUrl => ErrorCode::InvalidArgument,
            AiError::ModelRequired => ErrorCode::AiModelRequired,
            AiError::KeyMissing => ErrorCode::KeyMissing,
            AiError::PromptPack(_) => ErrorCode::Internal,
            AiError::InvalidVars(_) => ErrorCode::InvalidVars,
            AiError::PayloadTooLarge => ErrorCode::PayloadTooLarge,
            AiError::Provider { code, .. } => *code,
            AiError::InvalidOutput => ErrorCode::ProviderInvalid,
            AiError::Stream => ErrorCode::ProviderUnavailable,
            AiError::Unsupported => ErrorCode::Unsupported,
            AiError::InvalidAudio(_) => ErrorCode::InvalidArgument,
        }
    }

    /// The AASP `error` payload.
    pub fn to_stream_error(&self) -> StreamError {
        let code = self.code();
        let (message, retry_after_ms) = match self {
            AiError::Provider { message, retry_after_ms, .. } => (message.clone(), *retry_after_ms),
            other => (other.to_string(), None),
        };
        StreamError { code, message, retryable: code.retryable(), retry_after_ms }
    }

    /// Maps an HTTP status from a provider to an error.
    pub fn from_status(status: u16, retry_after_ms: Option<u64>) -> Self {
        let (code, message) = match status {
            401 | 403 => (ErrorCode::Unauthenticated, "The AI provider rejected the key.".to_owned()),
            402 => (ErrorCode::NotEntitled, "The AI provider account has no credit.".to_owned()),
            404 => (ErrorCode::InvalidArgument, "The AI provider does not know this model.".to_owned()),
            408 | 500..=599 => (ErrorCode::ProviderUnavailable, "The AI provider is unavailable.".to_owned()),
            409 => (ErrorCode::PromptVersionMismatch, "Update the app to use this feature.".to_owned()),
            413 => (ErrorCode::PayloadTooLarge, "The thread is too long for this model.".to_owned()),
            422 => (ErrorCode::InvalidVars, "The AI request was not valid.".to_owned()),
            429 => (ErrorCode::RateLimited, "The AI provider is limiting requests.".to_owned()),
            s => (ErrorCode::InvalidArgument, format!("The AI provider rejected the request (HTTP {s}).")),
        };
        AiError::Provider { code, message, retry_after_ms }
    }
}
