//! At-most-once sends with automatic verification (ARCHITECTURE §5.6), as a pure decision table.
//!
//! The worker drives a claimed `send` op through these questions:
//!
//! 1. Before **every** attempt: `find_sent(rfcMessageId)`. Found → the op is done and the send is
//!    `sent` ([`before_send`]).
//! 2. A `verifyOnly` op never sends. Not found → verify again at about +30 s, +2 min and +10 min,
//!    then give up as `unknown` ([`after_verify`]).
//! 3. Otherwise `worker.markSendStarted`, then the provider call. Its error decides between a
//!    certain non-acceptance (requeue or fail) and an ambiguous outcome ([`after_send_error`]).
//!    An ambiguous outcome is never reported: the op stays `inflight`, its lease expires, and the
//!    reaper turns it into `verifyOnly` because `sendStartedAt` is set (§5.3, DECISIONS D-234).

use super::SEND_VERIFY_DELAYS_MS;
use crate::wire::ErrorCode;

/// What the worker does next with a send op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendStep {
    /// Report success with the provider message id.
    Sent {
        /// `sentProviderMessageId`.
        provider_message_id: String,
    },
    /// Call `markSendStarted`, then the provider.
    Send,
    /// Report a retryable error so the op comes back after `after_ms` (attempt counted).
    VerifyLater {
        /// Delay before the next verification.
        after_ms: i64,
    },
    /// Report a non-retryable error; Convex turns a `verifyOnly` op into `unknown`.
    GiveUp,
    /// Requeue: the provider certainly did not take the message.
    Requeue {
        /// Error code to report.
        code: ErrorCode,
        /// Earliest retry.
        retry_after_ms: i64,
        /// Whether it counts against `maxAttempts` (false for auth and rate limits).
        count_attempt: bool,
    },
    /// Fail permanently with this code.
    Fail {
        /// Error code.
        code: ErrorCode,
    },
    /// The outcome is ambiguous: do not report; let the lease expire so the reaper sets
    /// `verifyOnly`.
    LeaveInflight,
}

/// Step 1: after `find_sent` for an op that is not `verifyOnly`.
pub fn before_send(found: Option<String>) -> SendStep {
    match found {
        Some(id) => SendStep::Sent { provider_message_id: id },
        None => SendStep::Send,
    }
}

/// Step 2: after `find_sent` for a `verifyOnly` op, timed by `send_elapsed_ms` (time since the
/// send began, from Convex, so a worker restart keeps the schedule): checks at about +30 s, +2 min
/// and +10 min after the send began, and the answer `unknown` only once a check at or after
/// +10 min found nothing. Each miss before that is `VerifyLater` until the next point.
pub fn verify_due(found: Option<String>, send_elapsed_ms: Option<i64>) -> SendStep {
    if let Some(id) = found {
        return SendStep::Sent { provider_message_id: id };
    }
    let Some(elapsed) = send_elapsed_ms else { return SendStep::GiveUp };
    let elapsed = elapsed.max(0);
    match SEND_VERIFY_DELAYS_MS.iter().find(|t| elapsed < **t) {
        Some(t) => SendStep::VerifyLater { after_ms: (t - elapsed).max(1_000) },
        None => SendStep::GiveUp,
    }
}

/// The schedule by count, for callers without a start time (`verifies_done` counts earlier
/// verifications of this op, including this one). The worker uses [`verify_due`].
pub fn after_verify(found: Option<String>, verifies_done: u32) -> SendStep {
    match found {
        Some(id) => SendStep::Sent { provider_message_id: id },
        None => match SEND_VERIFY_DELAYS_MS.get(verifies_done as usize) {
            Some(ms) => SendStep::VerifyLater { after_ms: *ms },
            None => SendStep::GiveUp,
        },
    }
}

/// How a provider send error is classified (mapped from `ProviderError` by the worker).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendErrorClass {
    /// Timeout, reset, 5xx after the upload began: the provider may have accepted it.
    Ambiguous,
    /// 429 / rate limited before acceptance.
    RateLimited {
        /// Retry-After in ms, if any.
        retry_after_ms: Option<i64>,
    },
    /// 401 after refresh: not accepted.
    Auth,
    /// The draft no longer exists.
    DraftGone,
    /// 4xx before upload (invalid, conflict).
    Rejected,
    /// Mailbox full or daily send limit.
    Quota,
    /// A connection failure before anything was sent (certainly not accepted).
    NotSent,
}

/// Step 3: after the provider call failed. `attempt` is the attempt that failed, `max_attempts`
/// the op's limit (5 for sends), `backoff_ms` the §5.5 backoff for this attempt.
pub fn after_send_error(class: SendErrorClass, attempt: u32, max_attempts: u32, backoff_ms: i64) -> SendStep {
    match class {
        SendErrorClass::Ambiguous => SendStep::LeaveInflight,
        SendErrorClass::RateLimited { retry_after_ms } => SendStep::Requeue {
            code: ErrorCode::ProviderRateLimited,
            retry_after_ms: retry_after_ms.unwrap_or(0).max(backoff_ms),
            count_attempt: false,
        },
        SendErrorClass::Auth => {
            SendStep::Requeue { code: ErrorCode::ReauthRequired, retry_after_ms: backoff_ms, count_attempt: false }
        }
        SendErrorClass::DraftGone => SendStep::Fail { code: ErrorCode::DraftDeleted },
        SendErrorClass::Rejected => SendStep::Fail { code: ErrorCode::ProviderInvalid },
        SendErrorClass::Quota => SendStep::Fail { code: ErrorCode::ProviderQuota },
        SendErrorClass::NotSent if attempt >= max_attempts => SendStep::Fail { code: ErrorCode::ProviderTransient },
        SendErrorClass::NotSent => {
            SendStep::Requeue { code: ErrorCode::ProviderTransient, retry_after_ms: backoff_ms, count_attempt: true }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn found_before_sending_never_sends() {
        assert_eq!(before_send(Some("m1".into())), SendStep::Sent { provider_message_id: "m1".into() });
        assert_eq!(before_send(None), SendStep::Send);
    }

    #[test]
    fn verify_only_schedule_then_unknown() {
        assert_eq!(after_verify(None, 0), SendStep::VerifyLater { after_ms: 30_000 });
        assert_eq!(after_verify(None, 1), SendStep::VerifyLater { after_ms: 120_000 });
        assert_eq!(after_verify(None, 2), SendStep::VerifyLater { after_ms: 600_000 });
        assert_eq!(after_verify(None, 3), SendStep::GiveUp);
        assert_eq!(after_verify(Some("x".into()), 3), SendStep::Sent { provider_message_id: "x".into() });
    }

    #[test]
    fn verification_is_timed_from_the_send_start() {
        assert_eq!(verify_due(None, Some(5_000)), SendStep::VerifyLater { after_ms: 25_000 });
        assert_eq!(verify_due(None, Some(40_000)), SendStep::VerifyLater { after_ms: 80_000 });
        assert_eq!(verify_due(None, Some(130_000)), SendStep::VerifyLater { after_ms: 470_000 });
        assert_eq!(verify_due(None, Some(599_500)), SendStep::VerifyLater { after_ms: 1_000 });
        // Only a check at or after +10 min may give up.
        assert_eq!(verify_due(None, Some(600_000)), SendStep::GiveUp);
        assert_eq!(verify_due(Some("m".into()), Some(0)), SendStep::Sent { provider_message_id: "m".into() });
        assert_eq!(verify_due(None, None), SendStep::GiveUp);
    }

    #[test]
    fn ambiguous_errors_are_never_retried_as_sends() {
        assert_eq!(after_send_error(SendErrorClass::Ambiguous, 1, 5, 2_000), SendStep::LeaveInflight);
        assert_eq!(
            after_send_error(SendErrorClass::NotSent, 1, 5, 2_000),
            SendStep::Requeue { code: ErrorCode::ProviderTransient, retry_after_ms: 2_000, count_attempt: true }
        );
        assert_eq!(
            after_send_error(SendErrorClass::NotSent, 5, 5, 2_000),
            SendStep::Fail { code: ErrorCode::ProviderTransient }
        );
        assert_eq!(
            after_send_error(SendErrorClass::RateLimited { retry_after_ms: Some(30_000) }, 1, 5, 2_000),
            SendStep::Requeue { code: ErrorCode::ProviderRateLimited, retry_after_ms: 30_000, count_attempt: false }
        );
        assert_eq!(
            after_send_error(SendErrorClass::DraftGone, 1, 5, 0),
            SendStep::Fail { code: ErrorCode::DraftDeleted }
        );
        assert!(matches!(
            after_send_error(SendErrorClass::Auth, 1, 5, 0),
            SendStep::Requeue { count_attempt: false, .. }
        ));
    }
}
