//! Provider push hooks (ARCHITECTURE §9.6): Gmail Pub/Sub and Microsoft Graph change and
//! lifecycle notifications. Hooks answer quickly and only nudge actors; the actor runs the delta.

use akatsuki_core::convex::worker::AccountsForPushArgs;
use akatsuki_core::sync::ActorCmd;
use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::worker::{GmailPushData, GraphNotificationBatch, GraphValidationQuery, PubSubPush};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;

use super::services;
use crate::crypto::ct_eq;
use crate::engine::actor::client_state_hash;
use crate::engine::{ActorMsg, Lifecycle};
use crate::http::{ApiError, AppState};
use crate::metrics::Metrics;

/// Largest notification body accepted.
pub const HOOK_BODY_LIMIT: usize = 1024 * 1024;

/// `POST /hooks/gmail`: verify the Google OIDC token, map the address to accounts and hint
/// their actors; `204` at once.
pub async fn gmail(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    let Some(verifier) = &svc.push_verifier else { return Err(ApiError::new(ErrorCode::NotFound)) };
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(akatsuki_core::auth::bearer_token)
        .unwrap_or_default();
    if !verifier.verify(token).await {
        Metrics::inc(&svc.metrics.push_rejected);
        return Err(ApiError::new(ErrorCode::Unauthenticated));
    }
    // A malformed payload is acknowledged (Pub/Sub would retry it forever) but ignored.
    let data = serde_json::from_slice::<PubSubPush>(&body)
        .ok()
        .and_then(|p| base64::engine::general_purpose::STANDARD.decode(p.message.data.as_bytes()).ok())
        .and_then(|d| serde_json::from_slice::<GmailPushData>(&d).ok());
    if let Some(d) = data {
        let rows = svc
            .api
            .accounts_for_push(&AccountsForPushArgs::GmailEmail { email: d.email_address.trim().to_lowercase() })
            .await
            .unwrap_or_default();
        for r in rows {
            if svc.registry.send(&r.account_id, ActorMsg::Cmd(ActorCmd::PushHint { history_id: Some(d.history_id) })) {
                Metrics::inc(&svc.metrics.push_hints);
            }
        }
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn echo(token: String) -> Result<Response, ApiError> {
    if token.len() > 4096 {
        return Err(ApiError::new(ErrorCode::InvalidArgument));
    }
    let mut resp = (StatusCode::OK, token).into_response();
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    Ok(resp)
}

/// Most notifications accepted in one Graph batch. Graph sends a handful per POST for one mailbox; a larger
/// batch is refused before any work (D-405).
pub const GRAPH_BATCH_MAX: usize = 50;

/// Graph subscription ids are GUIDs (`8-4-4-4-12` hex).
fn is_guid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Change and lifecycle notifications. Nothing here calls Convex: each notification is checked against the
/// subscriptions the running actors hold ([`crate::engine::GraphSubscriptions`]), and only a matching
/// `clientState` nudges its actor. An unknown id could not be delivered anyway (no actor here), so a forged
/// batch costs at most [`GRAPH_BATCH_MAX`] map lookups and hashes (D-405).
async fn notifications(state: &AppState, body: &[u8], lifecycle: bool) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    let batch: GraphNotificationBatch =
        serde_json::from_slice(body).map_err(|_| ApiError::new(ErrorCode::InvalidArgument))?;
    if batch.value.len() > GRAPH_BATCH_MAX {
        Metrics::inc(&svc.metrics.push_rejected);
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "Too many notifications in one batch."));
    }
    let mut seen = std::collections::HashSet::new();
    for n in batch.value {
        let Some(cs) = n.client_state.as_deref().filter(|c| !c.is_empty() && c.len() <= 256) else {
            Metrics::inc(&svc.metrics.push_rejected);
            continue;
        };
        if !is_guid(&n.subscription_id) {
            Metrics::inc(&svc.metrics.push_rejected);
            continue;
        }
        let Some((account_id, stored)) = svc.registry.graph.get(&n.subscription_id) else {
            Metrics::inc(&svc.metrics.push_rejected);
            continue;
        };
        // The clientState secret must match the hash stored when the subscription was made.
        if !ct_eq(client_state_hash(cs).as_bytes(), stored.as_bytes()) {
            Metrics::inc(&svc.metrics.push_rejected);
            continue;
        }
        let msg = match (lifecycle, n.lifecycle_event.as_deref()) {
            (true, Some("reauthorizationRequired")) => ActorMsg::Lifecycle(Lifecycle::Reauthorize),
            (true, Some("subscriptionRemoved")) => ActorMsg::Lifecycle(Lifecycle::Removed),
            (true, Some("missed")) => ActorMsg::Lifecycle(Lifecycle::Missed),
            (true, _) => continue,
            // One delta hint per subscription and batch is enough.
            (false, _) if !seen.insert(n.subscription_id.clone()) => continue,
            (false, _) => ActorMsg::Cmd(ActorCmd::PushHint { history_id: None }),
        };
        if svc.registry.send(&account_id, msg) {
            Metrics::inc(&svc.metrics.push_hints);
        }
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

/// `POST /hooks/graph`: echoes `validationToken` within 10 s, or `202` for a notification batch.
pub async fn graph(
    State(state): State<AppState>,
    Query(q): Query<GraphValidationQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    if let Some(token) = q.validation_token {
        return echo(token);
    }
    notifications(&state, &body, false).await
}

/// `POST /hooks/graph/lifecycle`.
pub async fn graph_lifecycle(
    State(state): State<AppState>,
    Query(q): Query<GraphValidationQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    if let Some(token) = q.validation_token {
        return echo(token);
    }
    notifications(&state, &body, true).await
}
