//! The two remote halves the desktop talks to, as traits so the demo mailbox (and tests) can stand
//! in for them: Convex ([`akatsuki_convex_api::ClientApi`]) and the sync worker ([`MailServer`]).

use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use akatsuki_core::wire::worker::{DraftPut, DraftSaved, SendAccepted, SendRequest, UploadResponse};

use crate::auth::Jwt;
use crate::worker::{WorkerClient, WorkerError};

/// The worker routes the desktop uses (§7.2).
#[async_trait]
pub trait MailServer: Send + Sync {
    /// The raw RFC 822 bytes of a message.
    async fn raw_message(&self, message_id: &str) -> Result<Vec<u8>, WorkerError>;
    /// Autosaves a draft.
    async fn put_draft(&self, client_draft_id: &str, put: &DraftPut) -> Result<DraftSaved, WorkerError>;
    /// Sends (after the undo window).
    async fn send(&self, req: &SendRequest) -> Result<SendAccepted, WorkerError>;
    /// The worker's sanitized HTML of a message with remote images behind its proxy, and the
    /// proxy's URL prefix (`{sync}/v1/img/`), for "Show images" (D-399). `html` is `None` for a
    /// text-only message.
    async fn proxied_html(&self, message_id: &str) -> Result<ProxiedHtml, WorkerError>;
    /// Uploads an attachment (`POST /v1/uploads`, §10.4).
    async fn upload(&self, filename: &str, content_type: &str, bytes: Vec<u8>) -> Result<UploadResponse, WorkerError>;
}

/// See [`MailServer::proxied_html`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxiedHtml {
    /// `MessageBody.html`.
    pub html: Option<String>,
    /// `{sync}/v1/img/`.
    pub proxy_prefix: String,
}

/// The current user JWT, shared by every caller and refreshed by the session task.
pub type JwtCell = Arc<RwLock<Option<Jwt>>>;

/// The production worker with the session's JWT.
#[derive(Clone, Debug)]
pub struct LiveMailServer {
    client: WorkerClient,
    jwt: JwtCell,
}

impl LiveMailServer {
    /// Wraps a worker client.
    pub fn new(client: WorkerClient, jwt: JwtCell) -> Self {
        Self { client, jwt }
    }

    fn token(&self) -> Result<String, WorkerError> {
        self.jwt.read().unwrap_or_else(|e| e.into_inner()).as_ref().map(|j| j.token.clone()).ok_or(WorkerError::Api {
            code: Some(akatsuki_core::ErrorCode::Unauthenticated),
            message: "Sign in again.".into(),
            status: 401,
            version: None,
        })
    }
}

#[async_trait]
impl MailServer for LiveMailServer {
    async fn raw_message(&self, message_id: &str) -> Result<Vec<u8>, WorkerError> {
        self.client.raw_message(message_id, &self.token()?).await
    }

    async fn put_draft(&self, client_draft_id: &str, put: &DraftPut) -> Result<DraftSaved, WorkerError> {
        self.client.put_draft(client_draft_id, put, &self.token()?).await
    }

    async fn send(&self, req: &SendRequest) -> Result<SendAccepted, WorkerError> {
        self.client.send(req, &self.token()?).await
    }

    async fn proxied_html(&self, message_id: &str) -> Result<ProxiedHtml, WorkerError> {
        let body = self.client.sanitized_message(message_id, &self.token()?).await?;
        Ok(ProxiedHtml { html: body.html, proxy_prefix: format!("{}/v1/img/", self.client.base()) })
    }

    async fn upload(&self, filename: &str, content_type: &str, bytes: Vec<u8>) -> Result<UploadResponse, WorkerError> {
        self.client.upload(filename, content_type, bytes, &self.token()?).await
    }
}
