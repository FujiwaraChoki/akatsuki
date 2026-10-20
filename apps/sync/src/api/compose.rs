//! Compose (ARCHITECTURE §10): uploads into the encrypted spool, provider drafts built with
//! `crates/mime::compose` (the only MIME builder), and the send pipeline (`worker.scheduleSend`;
//! Convex is the clock, the provider Drafts folder holds the text while it waits).

use akatsuki_core::convex::DraftState;
use akatsuki_core::convex::SendKind;
use akatsuki_core::convex::rows::SendPreview;
use akatsuki_core::convex::worker::{RecordDraftArgs, RecordDraftResult, ResolvedMessage, ScheduleSendArgs};
use akatsuki_core::ids::{AccountId, DraftId, MessageId, ThreadId};
use akatsuki_core::model::Addr;
use akatsuki_core::time::{DAY_MS, now_ms};
use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::worker::{
    AdoptRequest, AdoptResponse, DraftAttachment, DraftContent, DraftPut, DraftSaved, SendAccepted, SendRequest,
    UploadResponse,
};
use akatsuki_mime::{ComposeAttachment, ComposeInput, MimeError};
use akatsuki_providers::{OutgoingMime, RemoteDraftRef};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;

use super::{Services, convex_api_error, provider_api_error, services};
use crate::capability::{receipt_token, receipt_url};
use crate::http::{ApiError, ApiJson, AppState, CurrentUser, UserToken};
use crate::metrics::Metrics;
use crate::spool::{MAX_UPLOAD_BYTES, SpoolError};
use crate::user_convex::DraftMeta;

/// Client ids (`clientDraftId`, `clientSendId`) are UUIDs: 8 to 64 of `[A-Za-z0-9-]`.
pub fn valid_client_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Decodes and cleans `X-Filename` (URL-encoded): no path separators or control characters.
pub fn clean_filename(raw: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(raw).decode_utf8().ok()?;
    let name: String =
        decoded.chars().map(|c| if matches!(c, '/' | '\\') { '_' } else { c }).filter(|c| !c.is_control()).collect();
    let name = name.trim().trim_start_matches('.').to_owned();
    (!name.is_empty() && name.chars().count() <= 255).then_some(name)
}

fn valid_content_type(ct: &str) -> Option<String> {
    let base = ct.split(';').next()?.trim().to_ascii_lowercase();
    let (t, s) = base.split_once('/')?;
    let ok = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b));
    (ok(t) && ok(s)).then_some(base)
}

/// `POST /v1/uploads`: raw body, `Content-Type`, `X-Filename`; ≤ 25 MB; 24 h in the spool.
pub async fn upload(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    let filename = headers.get("x-filename").and_then(|v| v.to_str().ok()).and_then(clean_filename);
    let ctype = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).and_then(valid_content_type);
    let (Some(filename), Some(ctype)) = (filename, ctype) else {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "Send Content-Type and X-Filename."));
    };
    if body.is_empty() {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "The upload is empty."));
    }
    if body.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::new(ErrorCode::PayloadTooLarge));
    }
    let svc = services(&state.services)?;
    let spool = svc.spool.as_ref().ok_or_else(|| {
        ApiError::with_message(ErrorCode::ProviderUnavailable, "Uploads are not configured on this server.")
    })?;
    svc.entitled(&user.user_id).await?;
    // 200 MB/h per user, a cap on live uploads per user, and a cap on the whole spool (§13.4, D-397).
    svc.limits.upload(&user.user_id, body.len() as u64, spool.total_bytes().await)?;
    let (upload_id, meta) = spool.put(&user.user_id, &filename, &ctype, &body).await.map_err(|e| match e {
        SpoolError::TooLarge => ApiError::new(ErrorCode::PayloadTooLarge),
        _ => ApiError::new(ErrorCode::Internal),
    })?;
    Metrics::inc(&svc.metrics.uploads);
    Ok((StatusCode::CREATED, Json(UploadResponse { upload_id, size: meta.size, sha256: meta.sha256 })).into_response())
}

fn conflict(version: u64) -> Response {
    let code = ErrorCode::DraftVersionConflict;
    let body = json!({"error": {"code": code.as_str(), "message": code.default_message(), "retryable": false, "version": version}});
    (StatusCode::CONFLICT, Json(body)).into_response()
}

/// What a draft save produced.
#[derive(Debug)]
pub struct SavedDraft {
    /// Convex row.
    pub draft_id: DraftId,
    /// New version.
    pub version: u64,
    /// Provider draft.
    pub provider_draft_id: String,
    /// Fixed `Message-ID`.
    pub rfc_message_id: String,
    /// Account.
    pub account_id: AccountId,
}

/// Why a save did not happen.
#[derive(Debug)]
pub enum SaveError {
    /// The envelope to answer with.
    Api(ApiError),
    /// `409 draft_version_conflict` with the current version.
    Conflict(u64),
}

impl From<ApiError> for SaveError {
    fn from(e: ApiError) -> Self {
        SaveError::Api(e)
    }
}

impl IntoResponse for SaveError {
    fn into_response(self) -> Response {
        match self {
            SaveError::Api(e) => e.into_response(),
            SaveError::Conflict(v) => conflict(v),
        }
    }
}

struct ReplyContext {
    in_reply_to: Option<String>,
    references: Vec<String>,
    provider_thread_id: Option<String>,
    thread_id: Option<ThreadId>,
    reply_to_message_id: Option<MessageId>,
}

async fn reply_context(svc: &Services, user_id: &str, draft: &DraftContent) -> Result<ReplyContext, ApiError> {
    let mut ctx = ReplyContext {
        in_reply_to: None,
        references: vec![],
        provider_thread_id: None,
        thread_id: None,
        reply_to_message_id: None,
    };
    if let Some(id) = &draft.forward_of_message_id {
        let m = svc.resolve_message(user_id, id).await?;
        if m.account_id != draft.account_id {
            return Err(ApiError::new(ErrorCode::NotFound));
        }
    }
    let Some(id) = &draft.in_reply_to_message_id else { return Ok(ctx) };
    let m = svc.resolve_message(user_id, id).await?;
    if m.account_id != draft.account_id {
        return Err(ApiError::new(ErrorCode::NotFound));
    }
    let handle = svc.handle(user_id, &m.account_id)?;
    let raw = svc.fetch_raw(user_id, &handle, &m).await?;
    if let Ok(parsed) = akatsuki_mime::parse_message(&raw) {
        let mut refs = parsed.references.clone();
        if let Some(own) = &parsed.rfc_message_id {
            if !refs.contains(own) {
                refs.push(own.clone());
            }
            ctx.in_reply_to = Some(own.clone());
        }
        let start = refs.len().saturating_sub(20);
        ctx.references = refs[start..].to_vec();
    }
    ctx.provider_thread_id = Some(m.provider_thread_id.clone());
    ctx.thread_id = Some(m.thread_id.clone());
    ctx.reply_to_message_id = Some(m.message_id.clone());
    Ok(ctx)
}

async fn attachments(
    svc: &Services,
    user_id: &str,
    draft: &DraftContent,
) -> Result<(Vec<ComposeAttachment>, Vec<String>), ApiError> {
    let mut out = Vec::new();
    let mut uploads = Vec::new();
    let mut total = 0usize;
    for a in &draft.attachments {
        let att = match a {
            DraftAttachment::Upload { upload_id } => {
                let spool = svc.spool.as_ref().ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
                let up = spool.get(user_id, upload_id).await.map_err(|_| {
                    ApiError::with_message(ErrorCode::NotFound, "An attachment expired. Attach it again.")
                })?;
                uploads.push(upload_id.clone());
                ComposeAttachment {
                    filename: up.meta.filename,
                    mime_type: up.meta.content_type,
                    content: up.content,
                    inline_content_id: None,
                }
            }
            DraftAttachment::FromMessage { from_message_id, part_id } => {
                let m = svc.resolve_message(user_id, from_message_id).await?;
                let handle = svc.handle(user_id, &m.account_id)?;
                let raw = svc.fetch_raw(user_id, &handle, &m).await?;
                let part = akatsuki_mime::part_content(&raw, part_id)
                    .ok()
                    .flatten()
                    .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
                ComposeAttachment {
                    filename: part.filename.unwrap_or_else(|| "attachment".into()),
                    mime_type: part.mime_type,
                    content: part.bytes,
                    inline_content_id: None,
                }
            }
        };
        total += att.content.len();
        if total > akatsuki_mime::compose::MAX_ATTACHMENTS_BYTES {
            return Err(ApiError::new(ErrorCode::PayloadTooLarge));
        }
        out.push(att);
    }
    Ok((out, uploads))
}

/// Saves a draft at the provider and records it in Convex (§10.1). Shared by `PUT /v1/drafts`
/// and `POST /v1/send`.
pub async fn save_draft(
    state: &AppState,
    svc: &Services,
    user_id: &str,
    jwt: &str,
    client_draft_id: &str,
    base_version: u64,
    draft: &DraftContent,
    device_id: &str,
) -> Result<SavedDraft, SaveError> {
    let ctx = svc.entitled_with_account(user_id, &draft.account_id).await?;
    let handle = svc.handle(user_id, &draft.account_id)?;
    let existing: Option<DraftMeta> =
        svc.user_convex.draft_get(jwt, client_draft_id).await.map_err(|e| ApiError::new(e.code()))?;
    if let Some(d) = &existing {
        if d.account_id != draft.account_id {
            return Err(
                ApiError::with_message(ErrorCode::InvalidArgument, "A draft cannot move to another account.").into()
            );
        }
        if d.version != base_version || matches!(d.state, DraftState::Sending | DraftState::Sent) {
            return Err(SaveError::Conflict(d.version));
        }
    } else if base_version != 0 {
        return Err(SaveError::Conflict(0));
    }
    // The From address must be one of the account's identities (§7.3).
    let identities = svc.user_convex.identities(jwt, &draft.account_id).await.map_err(|e| ApiError::new(e.code()))?;
    let identity =
        identities.iter().find(|i| i.email.eq_ignore_ascii_case(draft.identity_email.trim())).ok_or_else(|| {
            ApiError::with_message(ErrorCode::InvalidArgument, "Send from one of this account's addresses.")
        })?;
    let domain = identity.email.rsplit_once('@').map(|(_, d)| d.to_owned()).unwrap_or_else(|| "akatsuki.vc".into());
    let rfc_message_id = existing
        .as_ref()
        .map(|d| d.rfc_message_id.trim_matches(|c| c == '<' || c == '>').to_owned())
        .unwrap_or_else(|| akatsuki_mime::mint_message_id(&domain));
    let reply = reply_context(svc, user_id, draft).await?;
    let (atts, uploads) = attachments(svc, user_id, draft).await?;
    let read_receipt = match (
        draft.read_receipt == Some(true) && ctx.prefs.read_receipts.enabled,
        &state.config.read_receipt_key,
        &state.config.public_base_url,
    ) {
        (true, Some(key), Some(base)) => {
            let token = receipt_token(&key.0, user_id, &rfc_message_id);
            Some((receipt_url(base, &token), token))
        }
        _ => None,
    };
    let (read_receipt_url, read_receipt_token) = read_receipt.unzip();
    let signature_html = identity.signature_html.clone().filter(|s| !s.trim().is_empty());
    let signature_text = signature_html.as_deref().map(akatsuki_mime::html_to_text);
    let input = ComposeInput {
        from: Addr::new(identity.name.as_deref(), &identity.email),
        to: draft.to.clone(),
        cc: draft.cc.clone(),
        bcc: draft.bcc.clone(),
        reply_to: identity.reply_to.iter().map(|r| Addr::new(None, r)).collect(),
        subject: draft.subject.clone(),
        text: draft.text.clone(),
        html: draft.html.clone(),
        message_id: rfc_message_id.clone(),
        in_reply_to: reply.in_reply_to.clone(),
        references: reply.references.clone(),
        date_ms: now_ms(),
        signature_html,
        signature_text,
        read_receipt_url,
        attachments: atts,
        // Gmail and Graph drafts keep Bcc in the stored draft; the IMAP adapter strips it for SMTP.
        include_bcc_header: true,
    };
    let bytes = akatsuki_mime::compose(&input).map_err(|e| match e {
        MimeError::TooLarge => ApiError::new(ErrorCode::PayloadTooLarge),
        _ => ApiError::with_message(ErrorCode::InvalidArgument, "The draft has an invalid address or header."),
    })?;
    let existing_ref = existing.as_ref().and_then(|d| {
        d.provider_draft_id
            .clone()
            .map(|id| RemoteDraftRef { provider_draft_id: id, provider_message_id: d.provider_message_id.clone() })
    });
    let mime = OutgoingMime {
        rfc_message_id: rfc_message_id.clone(),
        bytes: bytes.into(),
        thread: reply.provider_thread_id.clone(),
    };
    let saved = handle.provider.save_draft(&mime, existing_ref.as_ref()).await.map_err(|e| provider_api_error(&e))?;
    let rec = svc
        .api
        .record_draft(&RecordDraftArgs {
            user_id: user_id.to_owned(),
            account_id: draft.account_id.clone(),
            client_draft_id: client_draft_id.to_owned(),
            base_version,
            provider_draft_id: saved.provider_draft_id.clone(),
            provider_message_id: saved.provider_message_id.clone().unwrap_or_else(|| saved.provider_draft_id.clone()),
            rfc_message_id: rfc_message_id.clone(),
            thread_id: reply.thread_id.clone(),
            reply_to_message_id: reply.reply_to_message_id.clone(),
            subject: draft.subject.clone(),
            to: draft.to.clone(),
            cc: draft.cc.clone(),
            bcc: draft.bcc.clone(),
            device_id: Some(device_id.to_owned()),
            // Convex creates the `readReceipts` row the pixel reports to (D-412).
            read_receipt_token,
        })
        .await
        .map_err(|e| convex_api_error(&e))?;
    // Uploads stay in the spool for their 24 h (D-416): every later save of this draft (each autosave, a send,
    // the draft reopened by undo-send) composes the whole MIME again and attaches them again.
    let _ = uploads;
    match rec {
        RecordDraftResult::Saved { draft_id, version } => Ok(SavedDraft {
            draft_id,
            version,
            provider_draft_id: saved.provider_draft_id,
            rfc_message_id,
            account_id: draft.account_id.clone(),
        }),
        RecordDraftResult::Conflict { version, .. } => Err(SaveError::Conflict(version)),
    }
}

fn check_client_draft_id(id: &str) -> Result<(), ApiError> {
    if valid_client_id(id) {
        Ok(())
    } else {
        Err(ApiError::with_message(ErrorCode::InvalidArgument, "Invalid clientDraftId."))
    }
}

/// `PUT /v1/drafts/{clientDraftId}`.
pub async fn put(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(UserToken(jwt)): Extension<UserToken>,
    Path(client_draft_id): Path<String>,
    ApiJson(body): ApiJson<DraftPut>,
) -> Result<Response, SaveError> {
    check_client_draft_id(&client_draft_id)?;
    let svc = services(&state.services)?;
    let saved =
        save_draft(&state, svc, &user.user_id, &jwt, &client_draft_id, body.base_version, &body.draft, &body.device_id)
            .await?;
    Ok(Json(DraftSaved {
        draft_id: saved.draft_id,
        version: saved.version,
        provider_draft_id: saved.provider_draft_id,
        rfc_message_id: saved.rfc_message_id,
    })
    .into_response())
}

/// `POST /v1/drafts/{clientDraftId}/adopt`: opens a provider-native draft for editing.
pub async fn adopt(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(UserToken(jwt)): Extension<UserToken>,
    Path(client_draft_id): Path<String>,
    ApiJson(body): ApiJson<AdoptRequest>,
) -> Result<Response, ApiError> {
    check_client_draft_id(&client_draft_id)?;
    let svc = services(&state.services)?;
    svc.entitled(&user.user_id).await?;
    let m: ResolvedMessage = svc.resolve_message(&user.user_id, &body.message_id).await?;
    if !m.is_draft {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "This message is not a draft."));
    }
    let handle = svc.handle(&user.user_id, &m.account_id)?;
    let raw = svc.fetch_raw(&user.user_id, &handle, &m).await?;
    let parsed = akatsuki_mime::parse_message(&raw).map_err(|_| ApiError::new(ErrorCode::ProviderInvalid))?;
    let account_email = handle.snapshot().email;
    let rfc_message_id = parsed.rfc_message_id.clone().unwrap_or_else(|| {
        akatsuki_mime::mint_message_id(account_email.rsplit_once('@').map_or("akatsuki.vc", |(_, d)| d))
    });
    let draft = DraftContent {
        account_id: m.account_id.clone(),
        identity_email: parsed.from.as_ref().map_or(account_email, |f| f.email.clone()),
        to: parsed.to.clone(),
        cc: parsed.cc.clone(),
        bcc: parsed.bcc.clone(),
        subject: parsed.subject.clone(),
        html: parsed.html.as_deref().map(akatsuki_mime::sanitize_outgoing_html),
        text: parsed
            .text
            .clone()
            .or_else(|| parsed.html.as_deref().map(akatsuki_mime::html_to_text))
            .unwrap_or_default(),
        in_reply_to_message_id: None,
        forward_of_message_id: None,
        attachments: parsed
            .attachments
            .iter()
            .filter(|a| !a.inline)
            .map(|a| DraftAttachment::FromMessage { from_message_id: m.message_id.clone(), part_id: a.part_id.clone() })
            .collect(),
        read_receipt: None,
    };
    let existing = svc.user_convex.draft_get(&jwt, &client_draft_id).await.map_err(|e| ApiError::new(e.code()))?;
    let base_version = existing.as_ref().map_or(0, |d| d.version);
    // Graph and IMAP drafts are addressed by their message; Gmail adapters map it to the draft.
    let rec = svc
        .api
        .record_draft(&RecordDraftArgs {
            user_id: user.user_id.clone(),
            account_id: m.account_id.clone(),
            client_draft_id: client_draft_id.clone(),
            base_version,
            provider_draft_id: existing
                .as_ref()
                .and_then(|d| d.provider_draft_id.clone())
                .unwrap_or_else(|| m.provider_message_id.clone()),
            provider_message_id: m.provider_message_id.clone(),
            rfc_message_id,
            thread_id: Some(m.thread_id.clone()),
            reply_to_message_id: None,
            subject: draft.subject.clone(),
            to: draft.to.clone(),
            cc: draft.cc.clone(),
            bcc: draft.bcc.clone(),
            device_id: None,
            read_receipt_token: None,
        })
        .await
        .map_err(|e| convex_api_error(&e))?;
    let version = match rec {
        RecordDraftResult::Saved { version, .. } | RecordDraftResult::Conflict { version, .. } => version,
    };
    Ok(Json(AdoptResponse { draft, version }).into_response())
}

/// `DELETE /v1/drafts/{clientDraftId}`: enqueues `delete_draft` through Convex (`drafts.discard`).
pub async fn delete(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(UserToken(jwt)): Extension<UserToken>,
    Path(client_draft_id): Path<String>,
) -> Result<Response, ApiError> {
    check_client_draft_id(&client_draft_id)?;
    let svc = services(&state.services)?;
    svc.entitled(&user.user_id).await?;
    let op_id = uuid::Uuid::now_v7().to_string();
    svc.user_convex.draft_discard(&jwt, &op_id, &client_draft_id).await.map_err(|e| ApiError::new(e.code()))?;
    Ok(StatusCode::ACCEPTED.into_response())
}

/// `POST /v1/send` (§10.2): save the final draft synchronously, then `worker.scheduleSend`.
pub async fn send(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(UserToken(jwt)): Extension<UserToken>,
    ApiJson(body): ApiJson<SendRequest>,
) -> Result<Response, SaveError> {
    if ![0, 5, 10, 20, 30].contains(&body.undo_seconds) {
        return Err(
            ApiError::with_message(ErrorCode::InvalidArgument, "undoSeconds must be 0, 5, 10, 20 or 30.").into()
        );
    }
    if !valid_client_id(&body.client_send_id) {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "Invalid clientSendId.").into());
    }
    check_client_draft_id(&body.client_draft_id)?;
    let now = now_ms();
    if let Some(at) = body.send_at
        && (at < now - 60_000 || at > now + 366 * DAY_MS)
    {
        return Err(ApiError::with_message(ErrorCode::InvalidArgument, "sendAt must be within the next year.").into());
    }
    let svc = services(&state.services)?;
    let (draft_id, account_id, subject, to) = match &body.draft {
        Some(d) => {
            let saved = save_draft(
                &state,
                svc,
                &user.user_id,
                &jwt,
                &body.client_draft_id,
                body.base_version,
                d,
                &body.device_id,
            )
            .await?;
            (saved.draft_id, saved.account_id, d.subject.clone(), d.to.clone())
        }
        None => {
            svc.entitled(&user.user_id).await?;
            let d = svc
                .user_convex
                .draft_get(&jwt, &body.client_draft_id)
                .await
                .map_err(|e| ApiError::new(e.code()))?
                .ok_or_else(|| ApiError::new(ErrorCode::NotFound))?;
            if d.version != body.base_version {
                return Err(SaveError::Conflict(d.version));
            }
            if d.provider_draft_id.is_none() {
                return Err(ApiError::with_message(ErrorCode::InvalidArgument, "Save the draft before sending.").into());
            }
            (d.id, d.account_id, d.subject, d.to)
        }
    };
    let (kind, send_at) = match body.send_at {
        Some(at) => (SendKind::Later, at),
        None => (SendKind::Undo, now + i64::from(body.undo_seconds) * 1_000),
    };
    let r = svc
        .api
        .schedule_send(&ScheduleSendArgs {
            user_id: user.user_id.clone(),
            account_id,
            client_send_id: body.client_send_id.clone(),
            draft_id,
            kind,
            send_at,
            preview: SendPreview { subject, to: to.into_iter().take(10).collect() },
        })
        .await
        .map_err(|e| convex_api_error(&e))?;
    Ok((StatusCode::ACCEPTED, Json(SendAccepted { send_id: r.send_id, send_at, state: r.state })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_and_ids() {
        assert_eq!(clean_filename("R%C3%A9sum%C3%A9.pdf").as_deref(), Some("Résumé.pdf"));
        assert_eq!(clean_filename("..%2F..%2Fetc%2Fpasswd").as_deref(), Some("_.._etc_passwd"));
        assert_eq!(clean_filename("a%0d%0ab.txt").as_deref(), Some("ab.txt"));
        assert_eq!(clean_filename("%20%20"), None);
        assert!(valid_client_id("0192f0c4-7b51-7cc3-a3c5-6a8b1f2d3e4f"));
        assert!(!valid_client_id("../x"));
        assert_eq!(valid_content_type("application/pdf; charset=binary").as_deref(), Some("application/pdf"));
        assert_eq!(valid_content_type("text/html\r\nx: y"), None);
        assert_eq!(valid_content_type("nonsense"), None);
    }
}
