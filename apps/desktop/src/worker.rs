//! The sync worker's HTTP API as the desktop uses it (ARCHITECTURE §7): raw bodies for local
//! sanitizing (§14.3), draft autosave (§10.1) and send (§10.2). Every call carries the user JWT;
//! errors are the §15 envelope. Nothing here logs a URL with a query, a body or a token.

use akatsuki_core::ErrorCode;
use akatsuki_core::wire::ErrorBody;
use akatsuki_core::wire::worker::{DraftPut, DraftSaved, SendAccepted, SendRequest, UploadResponse};

/// Worker errors, safe to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerError {
    /// Network failure.
    #[error("Could not reach the sync server.")]
    Network,
    /// A §15 error.
    #[error("{message}")]
    Api {
        /// Code, when it is in the registry.
        code: Option<ErrorCode>,
        /// Safe message.
        message: String,
        /// HTTP status.
        status: u16,
        /// `draft_version_conflict` carries the current version.
        version: Option<u64>,
    },
    /// Unexpected response.
    #[error("The sync server sent an unexpected answer.")]
    Protocol,
}

impl WorkerError {
    /// The registry code.
    #[cfg(test)]
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            WorkerError::Api { code, .. } => *code,
            _ => None,
        }
    }
}

/// A client for one sync worker.
#[derive(Clone, Debug)]
pub struct WorkerClient {
    http: reqwest::Client,
    base: String,
}

async fn api_error(res: reqwest::Response) -> WorkerError {
    let status = res.status().as_u16();
    match res.json::<ErrorBody>().await {
        Ok(b) => {
            WorkerError::Api { code: Some(b.error.code), message: b.error.message, status, version: b.error.version }
        }
        Err(_) => WorkerError::Api {
            code: None,
            message: format!("The sync server answered {status}."),
            status,
            version: None,
        },
    }
}

impl WorkerClient {
    /// A client for `base` (e.g. `https://sync.akatsuki.vc`).
    pub fn new(http: reqwest::Client, base: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').to_owned() }
    }

    /// `GET /v1/messages/{id}?format=raw`: the RFC 822 bytes (desktop only).
    pub async fn raw_message(&self, message_id: &str, jwt: &str) -> Result<Vec<u8>, WorkerError> {
        let res = self
            .http
            .get(format!("{}/v1/messages/{}?format=raw", self.base, urlencode(message_id)))
            .bearer_auth(jwt)
            .send()
            .await
            .map_err(|_| WorkerError::Network)?;
        if !res.status().is_success() {
            return Err(api_error(res).await);
        }
        Ok(res.bytes().await.map_err(|_| WorkerError::Network)?.to_vec())
    }

    /// The worker base URL (`https://sync.akatsuki.vc`).
    pub fn base(&self) -> &str {
        &self.base
    }

    /// `GET /v1/messages/{id}?format=sanitized`: the worker's sanitized body, whose remote images
    /// point at its image proxy (§14.4). Used only for "Show images" (D-399).
    pub async fn sanitized_message(
        &self,
        message_id: &str,
        jwt: &str,
    ) -> Result<akatsuki_core::wire::worker::MessageBody, WorkerError> {
        let res = self
            .http
            .get(format!("{}/v1/messages/{}?format=sanitized", self.base, urlencode(message_id)))
            .bearer_auth(jwt)
            .send()
            .await
            .map_err(|_| WorkerError::Network)?;
        if !res.status().is_success() {
            return Err(api_error(res).await);
        }
        res.json().await.map_err(|_| WorkerError::Protocol)
    }

    /// `PUT /v1/drafts/{clientDraftId}`.
    pub async fn put_draft(&self, client_draft_id: &str, put: &DraftPut, jwt: &str) -> Result<DraftSaved, WorkerError> {
        let res = self
            .http
            .put(format!("{}/v1/drafts/{}", self.base, urlencode(client_draft_id)))
            .bearer_auth(jwt)
            .json(put)
            .send()
            .await
            .map_err(|_| WorkerError::Network)?;
        if !res.status().is_success() {
            return Err(api_error(res).await);
        }
        res.json().await.map_err(|_| WorkerError::Protocol)
    }

    /// `POST /v1/uploads` (§10.4): the raw bytes, `Content-Type` and the percent-encoded
    /// `X-Filename`; the worker keeps the file encrypted for 24 h.
    pub async fn upload(
        &self,
        filename: &str,
        content_type: &str,
        bytes: Vec<u8>,
        jwt: &str,
    ) -> Result<UploadResponse, WorkerError> {
        let res = self
            .http
            .post(format!("{}/v1/uploads", self.base))
            .bearer_auth(jwt)
            .header("content-type", content_type)
            .header("x-filename", urlencode(filename))
            .body(bytes)
            .send()
            .await
            .map_err(|_| WorkerError::Network)?;
        if !res.status().is_success() {
            return Err(api_error(res).await);
        }
        res.json().await.map_err(|_| WorkerError::Protocol)
    }

    /// `POST /v1/send`.
    pub async fn send(&self, req: &SendRequest, jwt: &str) -> Result<SendAccepted, WorkerError> {
        let res = self
            .http
            .post(format!("{}/v1/send", self.base))
            .bearer_auth(jwt)
            .json(req)
            .send()
            .await
            .map_err(|_| WorkerError::Network)?;
        if !res.status().is_success() {
            return Err(api_error(res).await);
        }
        res.json().await.map_err(|_| WorkerError::Protocol)
    }
}

/// Percent-encodes a path segment (Convex ids are URL-safe already; this is defence in depth).
fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_core::wire::worker::DraftContent;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn draft() -> DraftContent {
        DraftContent {
            account_id: "a1".into(),
            identity_email: "me@example.com".into(),
            to: vec![akatsuki_core::Addr::new(None, "ada@example.com")],
            cc: vec![],
            bcc: vec![],
            subject: "Hi".into(),
            html: None,
            text: "Hello".into(),
            in_reply_to_message_id: None,
            forward_of_message_id: None,
            attachments: vec![],
            read_receipt: None,
        }
    }

    #[tokio::test]
    async fn raw_bodies_send_the_jwt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/messages/m%2F1"))
            .and(query_param("format", "raw"))
            .and(header("authorization", "Bearer jwt"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"From: a@b.c\r\n\r\nhi".to_vec()))
            .mount(&server)
            .await;
        let c = WorkerClient::new(reqwest::Client::new(), &server.uri());
        assert_eq!(c.raw_message("m/1", "jwt").await.unwrap(), b"From: a@b.c\r\n\r\nhi");
    }

    #[tokio::test]
    async fn drafts_and_sends_decode_and_errors_carry_codes() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/v1/drafts/d1"))
            .respond_with(ResponseTemplate::new(409).set_body_json(json!({"error": {
                "code": "draft_version_conflict", "message": "This draft changed on another device.",
                "retryable": false, "version": 4}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/send"))
            .respond_with(
                ResponseTemplate::new(202).set_body_json(json!({"sendId": "s1", "sendAt": 10, "state": "scheduled"})),
            )
            .mount(&server)
            .await;
        let c = WorkerClient::new(reqwest::Client::new(), &format!("{}/", server.uri()));
        let err = c
            .put_draft("d1", &DraftPut { base_version: 1, draft: draft(), device_id: "dev".into() }, "jwt")
            .await
            .unwrap_err();
        assert_eq!(err.code(), Some(ErrorCode::DraftVersionConflict));
        assert!(matches!(err, WorkerError::Api { version: Some(4), status: 409, .. }));
        let sent = c
            .send(
                &SendRequest {
                    client_send_id: "cs".into(),
                    client_draft_id: "d1".into(),
                    base_version: 0,
                    draft: Some(draft()),
                    send_at: None,
                    undo_seconds: 10,
                    device_id: "dev".into(),
                },
                "jwt",
            )
            .await
            .unwrap();
        assert_eq!(sent.send_at, 10);
    }

    #[tokio::test]
    async fn uploads_send_the_name_type_and_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/uploads"))
            .and(header("authorization", "Bearer jwt"))
            .and(header("content-type", "application/pdf"))
            .and(header("x-filename", "Q3%20r%C3%A9sum%C3%A9.pdf"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"uploadId": "u1", "size": 3, "sha256": "ab"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let c = WorkerClient::new(reqwest::Client::new(), &server.uri());
        let r = c.upload("Q3 résumé.pdf", "application/pdf", b"pdf".to_vec(), "jwt").await.unwrap();
        assert_eq!(r.upload_id, "u1");
        let got = &server.received_requests().await.unwrap()[0];
        assert_eq!(got.body, b"pdf");
    }

    #[tokio::test]
    async fn non_envelope_errors_are_generic() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(502).set_body_string("<html>"))
            .mount(&server)
            .await;
        let c = WorkerClient::new(reqwest::Client::new(), &server.uri());
        let err = c.raw_message("m1", "jwt").await.unwrap_err();
        assert_eq!(err.code(), None);
        assert_eq!(err.to_string(), "The sync server answered 502.");
    }
}
