//! Provider-side search (`GET /v1/search`, ARCHITECTURE §11.2) and manual refresh
//! (`POST /v1/accounts/{accountId}/sync`).

use std::time::{Duration, Instant};

use akatsuki_core::ids::AccountId;
use akatsuki_core::sync::ActorCmd;
use akatsuki_core::time::now_ms;
use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::worker::{SearchQuery, SearchResponse};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{provider_api_error, services};
use crate::engine::ActorMsg;
use crate::http::{ApiError, AppState, CurrentUser};

/// Manual refresh debounce.
pub const MANUAL_SYNC_DEBOUNCE: Duration = Duration::from_secs(10);
/// How long a search waits for its hits to be upserted.
pub const SEARCH_FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The opaque search cursor: a provider page token bound to the account and the query (§1.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCursor {
    /// Account.
    pub a: String,
    /// `hex(sha256(q))[..16]`.
    pub h: String,
    /// Provider page token.
    pub p: String,
}

fn query_hash(q: &str) -> String {
    hex::encode(&Sha256::digest(q.as_bytes())[..8])
}

/// Encodes a cursor.
pub fn encode_cursor(account_id: &AccountId, q: &str, page: &str) -> String {
    let c = SearchCursor { a: account_id.as_str().to_owned(), h: query_hash(q), p: page.to_owned() };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&c).unwrap_or_default())
}

/// Decodes a cursor and checks it belongs to this account and query.
pub fn decode_cursor(cursor: &str, account_id: &AccountId, q: &str) -> Option<String> {
    let raw = URL_SAFE_NO_PAD.decode(cursor.as_bytes()).ok()?;
    let c: SearchCursor = serde_json::from_slice(&raw).ok()?;
    (c.a == account_id.as_str() && c.h == query_hash(q)).then_some(c.p)
}

/// `GET /v1/search?accountId&q&cursor`.
pub async fn search(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(q): Query<SearchQuery>,
) -> Result<Response, ApiError> {
    if q.q.chars().count() > 500 {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "The query is too long."));
    }
    let svc = services(&state.services)?;
    let context = svc.entitled(&user.user_id).await?;
    let handle = svc.handle(&user.user_id, &q.account_id)?;
    if !handle.provider.caps().server_search {
        return Err(ApiError::new(ErrorCode::Unsupported));
    }
    let page = match &q.cursor {
        Some(c) => Some(
            decode_cursor(c, &q.account_id, &q.q)
                .ok_or_else(|| ApiError::with_message(ErrorCode::InvalidArgument, "The cursor does not match."))?,
        ),
        None => None,
    };
    // Dates resolve in the user's zone, as in Convex `search.threads` (§11.1, D-404).
    let parsed = akatsuki_search::query::parse(&q.q, now_ms(), &context.timezone);
    let (ids, next) = handle.provider.search(&parsed.ast, page).await.map_err(|e| provider_api_error(&e))?;
    let thread_ids = if ids.is_empty() {
        vec![]
    } else {
        // Hits missing from Convex are fetched and upserted by the actor before we answer.
        let (msg, rx) = ActorMsg::search(ids);
        if !handle.send(msg) {
            return Err(ApiError::new(ErrorCode::ProviderUnavailable).retry_after(2_000));
        }
        tokio::time::timeout(SEARCH_FETCH_TIMEOUT, rx)
            .await
            .map_err(|_| ApiError::new(ErrorCode::ProviderTransient))?
            .map_err(|_| ApiError::new(ErrorCode::ProviderTransient))?
    };
    Ok(Json(SearchResponse { thread_ids, next_cursor: next.map(|p| encode_cursor(&q.account_id, &q.q, &p)) })
        .into_response())
}

/// `POST /v1/accounts/{accountId}/sync`: `202`, at most one delta per 10 s.
pub async fn account_sync(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(account_id): Path<String>,
) -> Result<Response, ApiError> {
    let svc = services(&state.services)?;
    let account_id = AccountId::new(account_id);
    svc.entitled_with_account(&user.user_id, &account_id).await?;
    if let Ok(h) = svc.handle(&user.user_id, &account_id) {
        let mut last = h.last_manual_sync.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_none_or(|t| t.elapsed() >= MANUAL_SYNC_DEBOUNCE) {
            *last = Some(Instant::now());
            h.send(ActorMsg::Cmd(ActorCmd::PushHint { history_id: None }));
        }
    }
    Ok(StatusCode::ACCEPTED.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_are_bound_to_account_and_query() {
        let a: AccountId = "acc00001".into();
        let c = encode_cursor(&a, "from:ada", "tok123");
        assert_eq!(decode_cursor(&c, &a, "from:ada").as_deref(), Some("tok123"));
        assert_eq!(decode_cursor(&c, &"acc00002".into(), "from:ada"), None);
        assert_eq!(decode_cursor(&c, &a, "from:bob"), None);
        assert_eq!(decode_cursor("!!", &a, "x"), None);
    }
}
