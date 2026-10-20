//! Drafts, send and send verification for Graph (ARCHITECTURE §5.6, §10.1, §10.2; D-213).
//!
//! - **MIME drafts** (the common case, up to `mime_draft_max_bytes`): `POST /me/messages` with
//!   `Content-Type: text/plain` and the base64 MIME from `crates/mime::compose`. Exchange keeps
//!   the `Message-ID`, `In-Reply-To` and `References` headers, the HTML and inline parts exactly
//!   as composed.
//! - **Large drafts** (Graph rejects requests over 4 MB): the MIME is parsed and recreated as a
//!   JSON message. Replies start from `createReply` on the parent (found by `internetMessageId`)
//!   so Outlook threads them; attachments under `upload_session_min_bytes` are posted inline,
//!   larger ones go through `createUploadSession` in chunks.
//! - Graph cannot replace a draft's MIME, so saving over an existing draft creates the new one
//!   first and then deletes the old one (the draft id changes; `worker.recordDraft` stores it).
//! - `send_draft` uses `POST /messages/{id}/send`. With immutable ids the sent copy in Sent Items
//!   keeps the draft's id, which is returned as the sent message id.
//! - `find_sent` looks the `Message-ID` up across the mailbox and ignores drafts.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use reqwest::Method;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE};
use serde_json::{Value, json};

use crate::{Addr, OutgoingMime, ProviderError, RemoteDraftRef, SentRef};

use super::GraphProvider;
use super::client::{Body, enc, map_status, odata_str, query, retry_after, transport};
use super::model::{Message, Page, UploadSession};

fn recipients(list: &[Addr]) -> Value {
    Value::Array(
        list.iter()
            .map(|a| match &a.name {
                Some(n) => json!({ "emailAddress": { "name": n, "address": a.email } }),
                None => json!({ "emailAddress": { "address": a.email } }),
            })
            .collect(),
    )
}

/// Finds a non-draft message by `Message-ID` (without brackets).
async fn find_by_message_id(p: &GraphProvider, rfc_id: &str) -> Result<Option<Message>, ProviderError> {
    let id = rfc_id.trim().trim_start_matches('<').trim_end_matches('>');
    let url = p.http.url(&format!(
        "/me/messages?{}",
        query(&[
            ("$filter", &format!("internetMessageId eq {}", odata_str(&format!("<{id}>")))),
            ("$select", "id,conversationId,isDraft,parentFolderId"),
            ("$top", "10"),
        ])
    ));
    let page: Page<Message> = p.http.get_json(&url, None).await?;
    Ok(page.value.into_iter().find(|m| !m.draft()))
}

pub(super) async fn find_sent(p: &GraphProvider, rfc_message_id: &str) -> Result<Option<SentRef>, ProviderError> {
    Ok(find_by_message_id(p, rfc_message_id)
        .await?
        .map(|m| SentRef { provider_message_id: m.id, provider_thread_id: m.conversation_id }))
}

pub(super) async fn delete_message(p: &GraphProvider, id: &str) -> Result<(), ProviderError> {
    let url = p.http.url(&format!("/me/messages/{}", enc(id)));
    match p.http.send(Method::DELETE, &url, None, Body::Empty).await {
        Ok(_) | Err(ProviderError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

pub(super) async fn send_draft(
    p: &GraphProvider,
    d: &RemoteDraftRef,
    _rfc_message_id: &str,
) -> Result<SentRef, ProviderError> {
    let id = &d.provider_draft_id;
    let url = p.http.url(&format!("/me/messages/{}?{}", enc(id), query(&[("$select", "id,conversationId,isDraft")])));
    // A draft deleted elsewhere is NotFound (the send becomes `failed`, `draft_deleted`).
    let current: Message = p.http.get_json(&url, None).await?;
    if current.draft() {
        let send = p.http.url(&format!("/me/messages/{}/send", enc(id)));
        p.http.send(Method::POST, &send, None, Body::Empty).await?;
    }
    Ok(SentRef { provider_message_id: current.id, provider_thread_id: current.conversation_id })
}

pub(super) async fn save_draft(
    p: &GraphProvider,
    mime: &OutgoingMime,
    existing: Option<&RemoteDraftRef>,
) -> Result<RemoteDraftRef, ProviderError> {
    let id = if mime.bytes.len() <= p.cfg.mime_draft_max_bytes {
        create_mime(p, mime).await?
    } else {
        create_json(p, mime).await?
    };
    // Best effort: the new draft is already durable at the provider. Failing the save here would
    // make the client retry and leave one more orphaned draft each time; a stale copy left in
    // Drafts is harmless (`find_sent` ignores drafts) and is removed by the next save or by
    // `delete_draft`.
    if let Some(old) = existing.filter(|o| o.provider_draft_id != id) {
        let _ = delete_message(p, &old.provider_draft_id).await;
    }
    Ok(RemoteDraftRef { provider_draft_id: id.clone(), provider_message_id: Some(id) })
}

async fn create_mime(p: &GraphProvider, mime: &OutgoingMime) -> Result<String, ProviderError> {
    let body = Body::Raw { content_type: "text/plain", bytes: Bytes::from(STANDARD.encode(&mime.bytes)) };
    let m: Message = p.http.send(Method::POST, &p.http.url("/me/messages"), None, body).await?.json()?;
    Ok(m.id)
}

async fn create_json(p: &GraphProvider, mime: &OutgoingMime) -> Result<String, ProviderError> {
    let parsed = akatsuki_mime::parse_message(&mime.bytes)
        .map_err(|_| ProviderError::Invalid("graph: draft MIME is not parseable".into()))?;
    let (content_type, content) = match (&parsed.html, &parsed.text) {
        (Some(h), _) => ("HTML", h.clone()),
        (None, Some(t)) => ("Text", t.clone()),
        (None, None) => ("Text", String::new()),
    };
    let mut message = json!({
        "subject": parsed.subject,
        "body": { "contentType": content_type, "content": content },
        "toRecipients": recipients(&parsed.to),
        "ccRecipients": recipients(&parsed.cc),
        "bccRecipients": recipients(&parsed.bcc),
        "replyTo": recipients(&parsed.reply_to),
        "internetMessageId": format!("<{}>", mime.rfc_message_id.trim_start_matches('<').trim_end_matches('>')),
    });
    if let Some(from) = &parsed.from {
        message["from"] = recipients(std::slice::from_ref(from))[0].clone();
    }

    // Replies start from createReply so Outlook files them in the conversation.
    let parent = match &parsed.in_reply_to {
        Some(irt) => find_by_message_id(p, irt).await?,
        None => None,
    };
    let id = match parent {
        Some(parent) => {
            let url = p.http.url(&format!("/me/messages/{}/createReply", enc(&parent.id)));
            let draft: Message = p.http.send(Method::POST, &url, None, Body::Json(json!({}))).await?.json()?;
            let patch = p.http.url(&format!("/me/messages/{}", enc(&draft.id)));
            p.http.send(Method::PATCH, &patch, None, Body::Json(message)).await?;
            draft.id
        }
        None => {
            let m: Message =
                p.http.send(Method::POST, &p.http.url("/me/messages"), None, Body::Json(message)).await?.json()?;
            m.id
        }
    };

    for att in &parsed.attachments {
        let Some(part) = akatsuki_mime::part_content(&mime.bytes, &att.part_id)
            .map_err(|_| ProviderError::Invalid("graph: draft MIME is not parseable".into()))?
        else {
            continue;
        };
        if part.bytes.len() < p.cfg.upload_session_min_bytes {
            let mut body = json!({
                "@odata.type": "#microsoft.graph.fileAttachment",
                "name": att.filename,
                "contentType": att.mime_type,
                "contentBytes": STANDARD.encode(&part.bytes),
                "isInline": att.inline,
            });
            if let Some(cid) = &att.content_id {
                body["contentId"] = json!(cid);
            }
            let url = p.http.url(&format!("/me/messages/{}/attachments", enc(&id)));
            p.http.send(Method::POST, &url, None, Body::Json(body)).await?;
        } else {
            upload_large(p, &id, &att.filename, &att.mime_type, att.inline, att.content_id.as_deref(), &part.bytes)
                .await?;
        }
    }
    Ok(id)
}

/// Upload URLs point at Outlook's upload host, not the API base. They carry their own
/// authorization, so no bearer token is sent; the host is still checked so attachment bytes only
/// go to Microsoft (or to the configured base in tests).
fn check_upload_url(p: &GraphProvider, upload_url: &str) -> Result<(), ProviderError> {
    let parsed = url::Url::parse(upload_url).map_err(|_| ProviderError::Invalid("graph: bad upload URL".into()))?;
    let base = url::Url::parse(p.http.base()).ok();
    let same_origin = base.is_some_and(|b| b.origin() == parsed.origin());
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let microsoft = parsed.scheme() == "https"
        && [".office.com", ".office365.com", ".outlook.com", ".microsoft.com"].iter().any(|s| host.ends_with(s));
    if same_origin || microsoft {
        Ok(())
    } else {
        Err(ProviderError::Invalid("graph: upload URL outside Microsoft".into()))
    }
}

async fn upload_large(
    p: &GraphProvider,
    message_id: &str,
    name: &str,
    content_type: &str,
    inline: bool,
    content_id: Option<&str>,
    bytes: &[u8],
) -> Result<(), ProviderError> {
    let mut item = json!({
        "attachmentType": "file",
        "name": name,
        "size": bytes.len(),
        "contentType": content_type,
        "isInline": inline,
    });
    if let Some(cid) = content_id {
        item["contentId"] = json!(cid);
    }
    let url = p.http.url(&format!("/me/messages/{}/attachments/createUploadSession", enc(message_id)));
    let session: UploadSession =
        p.http.send(Method::POST, &url, None, Body::Json(json!({ "AttachmentItem": item }))).await?.json()?;
    check_upload_url(p, &session.upload_url)?;

    let total = bytes.len();
    let chunk = p.cfg.upload_chunk_bytes.max(320 * 1024);
    let mut start = 0;
    while start < total {
        let end = (start + chunk).min(total);
        p.http.throttle(1).await;
        let resp = p
            .http
            .http
            .put(&session.upload_url)
            .header(CONTENT_RANGE, format!("bytes {start}-{}/{total}", end - 1))
            .header(CONTENT_LENGTH, end - start)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(bytes[start..end].to_vec())
            .send()
            .await
            .map_err(|e| transport(&e))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let ra = retry_after(resp.headers());
            let body = resp.bytes().await.unwrap_or_default();
            return Err(map_status(status, ra, &body));
        }
        start = end;
    }
    Ok(())
}
