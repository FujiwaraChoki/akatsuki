//! Client functions the worker calls **as the user** (with the caller's own JWT) over the Convex
//! HTTP API: `drafts.get`, `drafts.discard` and `identities.list` (DECISIONS D-235). They give the
//! draft routes the fixed `rfcMessageId`, the provider draft id, the CAS version and the
//! identity's signature without new worker service functions, and Convex checks ownership.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::time::Duration;

use akatsuki_core::convex::DraftState;
use akatsuki_core::ids::{AccountId, DraftId, MessageId, ThreadId};
use akatsuki_core::model::Addr;
use akatsuki_core::wire::ErrorCode;

/// `DraftMeta` (`drafts` minus `userId`), the fields the worker reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftMeta {
    /// Row id.
    #[serde(rename = "_id")]
    pub id: DraftId,
    /// Account.
    pub account_id: AccountId,
    /// Client key.
    pub client_draft_id: String,
    /// Thread.
    #[serde(default)]
    pub thread_id: Option<ThreadId>,
    /// Message replied to.
    #[serde(default)]
    pub reply_to_message_id: Option<MessageId>,
    /// Provider draft id.
    #[serde(default)]
    pub provider_draft_id: Option<String>,
    /// Provider message id.
    #[serde(default)]
    pub provider_message_id: Option<String>,
    /// Fixed `Message-ID`.
    pub rfc_message_id: String,
    /// CAS version.
    pub version: u64,
    /// Subject.
    #[serde(default)]
    pub subject: String,
    /// To.
    #[serde(default)]
    pub to: Vec<Addr>,
    /// State.
    pub state: DraftState,
}

/// `IdentityView`, the fields the worker reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityView {
    /// Account.
    pub account_id: AccountId,
    /// Address.
    pub email: String,
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Reply-To.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Signature (sanitized on write; sanitized again at compose).
    #[serde(default)]
    pub signature_html: Option<String>,
    /// Default identity.
    pub is_default: bool,
}

/// A failed user-scoped call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UserCallError {
    /// Convex answered with an application error.
    #[error("convex error {0:?}")]
    App(Option<ErrorCode>),
    /// Network or decoding failure.
    #[error("convex unavailable")]
    Unavailable,
}

impl UserCallError {
    /// The registry code to answer with.
    pub fn code(&self) -> ErrorCode {
        match self {
            UserCallError::App(Some(c)) => *c,
            UserCallError::App(None) => ErrorCode::Internal,
            UserCallError::Unavailable => ErrorCode::ProviderUnavailable,
        }
    }
}

/// User-scoped Convex calls.
#[async_trait]
pub trait UserConvex: Send + Sync {
    /// `drafts.get`.
    async fn draft_get(&self, jwt: &str, client_draft_id: &str) -> Result<Option<DraftMeta>, UserCallError>;
    /// `drafts.discard` (enqueues `delete_draft`).
    async fn draft_discard(&self, jwt: &str, client_op_id: &str, client_draft_id: &str) -> Result<(), UserCallError>;
    /// `identities.list` for one account.
    async fn identities(&self, jwt: &str, account_id: &AccountId) -> Result<Vec<IdentityView>, UserCallError>;
}

/// The Convex HTTP API (`POST {CONVEX_URL}/api/query|mutation`).
#[derive(Clone, Debug)]
pub struct HttpUserConvex {
    http: reqwest::Client,
    base: String,
}

impl HttpUserConvex {
    /// A client for `CONVEX_URL`.
    pub fn new(http: reqwest::Client, convex_url: &str) -> Self {
        Self { http, base: convex_url.trim_end_matches('/').to_owned() }
    }

    async fn call(&self, kind: &str, jwt: &str, path: &str, args: Json) -> Result<Json, UserCallError> {
        let resp = self
            .http
            .post(format!("{}/api/{kind}", self.base))
            .bearer_auth(jwt)
            .json(&json!({"path": path, "args": args, "format": "json"}))
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|_| UserCallError::Unavailable)?;
        let body: Json = resp.json().await.map_err(|_| UserCallError::Unavailable)?;
        match body.get("status").and_then(Json::as_str) {
            Some("success") => {
                // The HTTP API writes every `v.number()` as a float (`1.0`); integer fields need integers.
                let mut value = body.get("value").cloned().unwrap_or(Json::Null);
                akatsuki_convex_api::value::normalize_json_numbers(&mut value);
                Ok(value)
            }
            Some("error") => Err(UserCallError::App(
                body.get("errorData").and_then(|d| d.get("code")).and_then(Json::as_str).and_then(ErrorCode::from_name),
            )),
            _ => Err(UserCallError::Unavailable),
        }
    }
}

#[async_trait]
impl UserConvex for HttpUserConvex {
    async fn draft_get(&self, jwt: &str, client_draft_id: &str) -> Result<Option<DraftMeta>, UserCallError> {
        let v = self.call("query", jwt, "drafts:get", json!({"clientDraftId": client_draft_id})).await?;
        serde_json::from_value(v).map_err(|_| UserCallError::Unavailable)
    }

    async fn draft_discard(&self, jwt: &str, client_op_id: &str, client_draft_id: &str) -> Result<(), UserCallError> {
        self.call(
            "mutation",
            jwt,
            "drafts:discard",
            json!({"clientOpId": client_op_id, "clientDraftId": client_draft_id}),
        )
        .await
        .map(|_| ())
    }

    async fn identities(&self, jwt: &str, account_id: &AccountId) -> Result<Vec<IdentityView>, UserCallError> {
        let v = self.call("query", jwt, "identities:list", json!({"accountId": account_id})).await?;
        serde_json::from_value(v).map_err(|_| UserCallError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn calls_convex_http_api_with_the_user_jwt() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/query"))
            .and(header("authorization", "Bearer user-jwt"))
            .and(body_partial_json(json!({"path": "drafts:get", "args": {"clientDraftId": "cd1"}})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "success", "value": {
                "_id": "drf00001", "_creationTime": 1.0, "accountId": "acc00001", "clientDraftId": "cd1",
                "providerDraftId": "r-1", "rfcMessageId": "x@y.z", "version": 3.0, "subject": "S", "to": [],
                "cc": [], "bcc": [], "state": "editing", "updatedAt": 1
            }})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/mutation"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "error", "errorMessage": "x", "errorData": {"code": "not_found", "message": "Not found"}
            })))
            .mount(&server)
            .await;
        let c = HttpUserConvex::new(reqwest::Client::new(), &server.uri());
        let d = c.draft_get("user-jwt", "cd1").await.unwrap().unwrap();
        assert_eq!(d.version, 3);
        assert_eq!(d.provider_draft_id.as_deref(), Some("r-1"));
        assert_eq!(c.draft_discard("user-jwt", "op", "cd1").await, Err(UserCallError::App(Some(ErrorCode::NotFound))));
    }
}
