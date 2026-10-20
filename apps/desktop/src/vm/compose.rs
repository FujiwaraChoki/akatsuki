//! Compose (ARCHITECTURE §10): drafts typed on this device, autosaved to the provider through the
//! worker every 2 s (§10.1), sent with an undo window (§10.2, §10.3). The worker builds the MIME;
//! the desktop only sends `DraftContent`.

use akatsuki_core::Addr;
use akatsuki_core::convex::rows::MessageMeta;
use akatsuki_core::store::{Store, StoreError};
use akatsuki_core::wire::worker::{DraftAttachment, DraftContent, SendRequest};

/// Autosave debounce (§10.1).
pub const AUTOSAVE_DEBOUNCE_MS: u64 = 2000;

/// Attachments per message (§1.6; the worker refuses more).
pub const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// Where an attachment is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentState {
    /// Being read and uploaded.
    Uploading,
    /// In the worker's spool (`POST /v1/uploads`, §10.4).
    Ready {
        /// Upload id.
        upload_id: String,
    },
    /// Could not be uploaded; the text says why.
    Failed(String),
}

/// A file attached on this device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Local key.
    pub key: String,
    /// File name as it goes out.
    pub filename: String,
    /// Bytes.
    pub size: u64,
    /// Upload state.
    pub state: AttachmentState,
}

/// "12 KB", "3.4 MB".
pub fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// The media type of a file, from its extension (the worker checks the syntax only).
pub fn content_type_for(filename: &str) -> &'static str {
    let ext = filename.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "txt" | "md" | "log" => "text/plain",
        "csv" => "text/csv",
        "ics" => "text/calendar",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// What the draft answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeKind {
    /// A new message.
    New,
    /// A reply.
    Reply {
        /// Thread.
        thread_id: String,
        /// The message replied to.
        message_id: String,
    },
    /// A forward.
    Forward {
        /// Thread.
        thread_id: String,
        /// The message forwarded.
        message_id: String,
    },
}

/// A draft being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Idempotency key of the draft (UUIDv7).
    pub client_draft_id: String,
    /// Account it sends from.
    pub account_id: String,
    /// Identity (`From`).
    pub identity_email: String,
    /// To, as typed.
    pub to: String,
    /// Cc, as typed.
    pub cc: String,
    /// Bcc, as typed.
    pub bcc: String,
    /// Subject.
    pub subject: String,
    /// Plain-text body.
    pub body: String,
    /// Reply or forward.
    pub kind: ComposeKind,
    /// Provider draft version (0 until the first autosave).
    pub base_version: u64,
    /// Files attached on this device.
    pub attachments: Vec<Attachment>,
}

/// Why a draft cannot be sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    /// No recipient.
    #[error("Add at least one recipient.")]
    NoRecipients,
    /// A recipient is not an address.
    #[error("\"{0}\" is not an email address.")]
    BadAddress(String),
    /// No account to send from.
    #[error("Connect a mailbox before sending.")]
    NoAccount,
    /// An attachment is still uploading.
    #[error("Wait for the attachments to finish uploading.")]
    Uploading,
    /// An attachment failed.
    #[error("Remove the attachments that failed to upload.")]
    FailedAttachment,
    /// Over 25 MB.
    #[error("Attachments are limited to 25 MB per message.")]
    TooLarge,
}

fn is_address(s: &str) -> bool {
    let Some((local, domain)) = s.rsplit_once('@') else { return false };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !s.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '"' | '\r' | '\n'))
}

/// Parses `Ada <ada@example.com>, bo@example.org; "Lee, C" <c@x.example>`.
pub fn parse_recipients(input: &str) -> Result<Vec<Addr>, ComposeError> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut parts = Vec::new();
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ',' | ';' | '\n' if !quoted => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    parts.push(current);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, email) = match (part.rfind('<'), part.rfind('>')) {
            (Some(l), Some(r)) if l < r => {
                let name = part[..l].trim().trim_matches('"').trim();
                (Some(name).filter(|n| !n.is_empty()), part[l + 1..r].trim())
            }
            _ => (None, part),
        };
        if !is_address(email) {
            return Err(ComposeError::BadAddress(part.to_owned()));
        }
        let addr = Addr::new(name, email);
        if !out.iter().any(|a: &Addr| a.email == addr.email) {
            out.push(addr);
        }
    }
    Ok(out)
}

/// Formats addresses for a recipient field.
pub fn format_recipients(addrs: &[Addr]) -> String {
    addrs
        .iter()
        .map(|a| match &a.name {
            Some(n) if n.contains([',', ';', '"']) => format!("\"{}\" <{}>", n.replace('"', ""), a.email),
            Some(n) => format!("{n} <{}>", a.email),
            None => a.email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn prefixed(subject: &str, prefix: &str) -> String {
    let s = subject.trim();
    let lower = s.to_lowercase();
    let p = prefix.to_lowercase();
    if lower.starts_with(&format!("{p}:")) { s.to_owned() } else { format!("{prefix}: {s}") }
}

fn quote(text: &str) -> String {
    text.lines().map(|l| if l.is_empty() { ">".to_owned() } else { format!("> {l}") }).collect::<Vec<_>>().join("\n")
}

impl Draft {
    /// A new, empty draft from an account.
    pub fn new(client_draft_id: String, account_id: &str, identity_email: &str) -> Self {
        Self {
            client_draft_id,
            account_id: account_id.to_owned(),
            identity_email: identity_email.to_owned(),
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            subject: String::new(),
            body: String::new(),
            kind: ComposeKind::New,
            base_version: 0,
            attachments: Vec::new(),
        }
    }

    /// Bytes attached.
    pub fn attachment_bytes(&self) -> u64 {
        self.attachments.iter().map(|a| a.size).sum()
    }

    /// Why the attachments block a send, if they do.
    pub fn attachment_problem(&self) -> Option<ComposeError> {
        if self.attachments.iter().any(|a| a.state == AttachmentState::Uploading) {
            Some(ComposeError::Uploading)
        } else if self.attachments.iter().any(|a| matches!(a.state, AttachmentState::Failed(_))) {
            Some(ComposeError::FailedAttachment)
        } else if self.attachment_bytes() > MAX_ATTACHMENT_BYTES {
            Some(ComposeError::TooLarge)
        } else {
            None
        }
    }

    /// A reply to the newest message of a thread that is not from the user (or the newest one).
    /// `quoted_text` is the body of that message, when it is cached; `when` its formatted date.
    pub fn reply(
        client_draft_id: String,
        store: &Store,
        thread_id: &str,
        all: bool,
        my_addresses: &[String],
        quoted_text: Option<&str>,
        when: &str,
    ) -> Result<Option<Self>, StoreError> {
        let messages = store.messages(thread_id)?;
        let Some(thread) = store.thread(thread_id)? else { return Ok(None) };
        let mine = |a: &Addr| my_addresses.iter().any(|m| m.eq_ignore_ascii_case(&a.email));
        let target: Option<&MessageMeta> = messages
            .iter()
            .rev()
            .find(|m| m.from.as_ref().is_some_and(|f| !mine(f)) && !m.is_draft)
            .or(messages.last());
        let Some(target) = target else { return Ok(None) };
        let account = store.accounts()?.into_iter().find(|a| a.id == thread.account_id);
        let identity = account.as_ref().map(|a| a.email.clone()).unwrap_or_default();
        let reply_to = if target.reply_to.is_empty() {
            target.from.clone().into_iter().collect()
        } else {
            target.reply_to.clone()
        };
        let to: Vec<Addr> = if target.from.as_ref().is_some_and(mine) { target.to.clone() } else { reply_to };
        let cc: Vec<Addr> = if all {
            target
                .to
                .iter()
                .chain(&target.cc)
                .filter(|a| !mine(a) && !to.iter().any(|t| t.email == a.email))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let who = target.from.as_ref().map(|f| f.name.clone().unwrap_or_else(|| f.email.clone())).unwrap_or_default();
        let body = match quoted_text {
            Some(q) => format!("\n\nOn {when}, {who} wrote:\n{}", quote(q.trim_end())),
            None => String::new(),
        };
        Ok(Some(Self {
            to: format_recipients(&to),
            cc: format_recipients(&cc),
            subject: prefixed(&thread.subject, "Re"),
            body,
            kind: ComposeKind::Reply { thread_id: thread_id.to_owned(), message_id: target.id.as_str().to_owned() },
            ..Self::new(client_draft_id, thread.account_id.as_str(), &identity)
        }))
    }

    /// A forward of the newest message of a thread.
    pub fn forward(
        client_draft_id: String,
        store: &Store,
        thread_id: &str,
        quoted_text: Option<&str>,
        when: &str,
    ) -> Result<Option<Self>, StoreError> {
        let Some(thread) = store.thread(thread_id)? else { return Ok(None) };
        let Some(last) = store.messages(thread_id)?.pop() else { return Ok(None) };
        let account = store.accounts()?.into_iter().find(|a| a.id == thread.account_id);
        let identity = account.as_ref().map(|a| a.email.clone()).unwrap_or_default();
        let from = last.from.as_ref().map(|f| format_recipients(std::slice::from_ref(f))).unwrap_or_default();
        let body = format!(
            "\n\n---------- Forwarded message ----------\nFrom: {from}\nDate: {when}\nSubject: {}\n\n{}",
            last.subject,
            quoted_text.unwrap_or("").trim_end()
        );
        Ok(Some(Self {
            subject: prefixed(&thread.subject, "Fwd"),
            body,
            kind: ComposeKind::Forward { thread_id: thread_id.to_owned(), message_id: last.id.as_str().to_owned() },
            ..Self::new(client_draft_id, thread.account_id.as_str(), &identity)
        }))
    }

    /// The draft as the worker takes it. Validates the recipients.
    pub fn content(&self) -> Result<DraftContent, ComposeError> {
        if self.account_id.is_empty() {
            return Err(ComposeError::NoAccount);
        }
        let to = parse_recipients(&self.to)?;
        let cc = parse_recipients(&self.cc)?;
        let bcc = parse_recipients(&self.bcc)?;
        if to.is_empty() && cc.is_empty() && bcc.is_empty() {
            return Err(ComposeError::NoRecipients);
        }
        if let Some(e) = self.attachment_problem() {
            return Err(e);
        }
        Ok(self.content_unchecked(to, cc, bcc))
    }

    /// The draft for autosave: invalid recipients are dropped rather than refused.
    pub fn autosave_content(&self) -> Option<DraftContent> {
        if self.account_id.is_empty()
            || (self.body.trim().is_empty() && self.subject.trim().is_empty() && self.attachments.is_empty())
        {
            return None;
        }
        let lenient = |s: &str| parse_recipients(s).unwrap_or_default();
        Some(self.content_unchecked(lenient(&self.to), lenient(&self.cc), lenient(&self.bcc)))
    }

    fn content_unchecked(&self, to: Vec<Addr>, cc: Vec<Addr>, bcc: Vec<Addr>) -> DraftContent {
        let (in_reply_to, forward_of) = match &self.kind {
            ComposeKind::New => (None, None),
            ComposeKind::Reply { message_id, .. } => (Some(message_id.as_str().into()), None),
            ComposeKind::Forward { message_id, .. } => (None, Some(message_id.as_str().into())),
        };
        DraftContent {
            account_id: self.account_id.as_str().into(),
            identity_email: self.identity_email.clone(),
            to,
            cc,
            bcc,
            subject: self.subject.trim().to_owned(),
            html: None,
            text: self.body.clone(),
            in_reply_to_message_id: in_reply_to,
            forward_of_message_id: forward_of,
            // Uploaded files only; one still uploading joins the next autosave.
            attachments: self
                .attachments
                .iter()
                .filter_map(|a| match &a.state {
                    AttachmentState::Ready { upload_id } => {
                        Some(DraftAttachment::Upload { upload_id: upload_id.clone() })
                    }
                    _ => None,
                })
                .collect(),
            read_receipt: None,
        }
    }

    /// The `POST /v1/send` request (`sendAt` absent: now + `undo_seconds`).
    pub fn send_request(
        &self,
        client_send_id: String,
        undo_seconds: u32,
        device_id: &str,
    ) -> Result<SendRequest, ComposeError> {
        Ok(SendRequest {
            client_send_id,
            client_draft_id: self.client_draft_id.clone(),
            base_version: self.base_version,
            draft: Some(self.content()?),
            send_at: None,
            undo_seconds,
            device_id: device_id.to_owned(),
        })
    }
}

/// A send in its undo window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSend {
    /// Idempotency key.
    pub client_send_id: String,
    /// The draft, kept so Undo can reopen it from this device's copy (§10.3).
    pub draft: Draft,
    /// When it leaves (ms).
    pub send_at: i64,
}

impl PendingSend {
    /// Seconds left in the undo window.
    pub fn seconds_left(&self, now_ms: i64) -> i64 {
        ((self.send_at - now_ms) + 999).div_euclid(1000).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn recipients_parse_names_quotes_and_separators() {
        let r =
            parse_recipients("Ada Moreau <Ada@Example.com>, bo@example.org; \"Lee, C\" <c@x.example>, ada@example.com")
                .unwrap();
        assert_eq!(r.len(), 3, "duplicates are dropped");
        assert_eq!(r[0], Addr::new(Some("Ada Moreau"), "ada@example.com"));
        assert_eq!(r[2].name.as_deref(), Some("Lee, C"));
        assert_eq!(parse_recipients("  ").unwrap(), vec![]);
        assert_eq!(parse_recipients("not an address"), Err(ComposeError::BadAddress("not an address".into())));
        assert!(parse_recipients("x@localhost").is_err());
        assert_eq!(format_recipients(&r), "Ada Moreau <ada@example.com>, bo@example.org, \"Lee, C\" <c@x.example>");
    }

    #[test]
    fn replies_answer_the_last_message_from_someone_else() {
        let mut store = Store::open_in_memory().unwrap();
        demo::seed_store(&mut store).unwrap();
        let d =
            Draft::reply("d1".into(), &store, "t-hiring", false, &[demo::ME.into()], Some("Done.\nOctober."), "Mon")
                .unwrap()
                .unwrap();
        assert_eq!(d.to, "Jonas Weber <jonas@northwind.example>");
        assert_eq!(d.subject, "Re: Hiring plan Q4", "no double Re:");
        assert!(d.body.contains("On Mon, Jonas Weber wrote:\n> Done.\n> October."));
        assert_eq!(d.account_id, demo::GMAIL);
        assert_eq!(d.identity_email, demo::ME);
        let c = d.content().unwrap();
        assert_eq!(c.in_reply_to_message_id.as_ref().map(|m| m.as_str()), Some("t-hiring-m2"));

        let f = Draft::forward("d2".into(), &store, "t-review", Some("Notes"), "Mon").unwrap().unwrap();
        assert_eq!(f.subject, "Fwd: Design review notes");
        assert!(f.body.contains("Forwarded message"));
        assert_eq!(f.content(), Err(ComposeError::NoRecipients));
    }

    #[test]
    fn send_requests_carry_the_undo_window() {
        let mut d = Draft::new("d1".into(), "a1", "me@example.com");
        d.to = "ada@example.com".into();
        d.subject = "Hello".into();
        d.body = "Hi".into();
        d.base_version = 3;
        let r = d.send_request("s1".into(), 10, "dev").unwrap();
        assert_eq!(r.undo_seconds, 10);
        assert_eq!(r.base_version, 3);
        assert_eq!(r.send_at, None);
        assert_eq!(r.draft.unwrap().text, "Hi");
        let p = PendingSend { client_send_id: "s1".into(), draft: d.clone(), send_at: 10_000 };
        assert_eq!(p.seconds_left(1_500), 9);
        assert_eq!(p.seconds_left(20_000), 0);
        assert!(Draft::new("d".into(), "a", "m").autosave_content().is_none(), "empty drafts are not saved");
        d.to = "garbage".into();
        assert!(d.autosave_content().unwrap().to.is_empty());
    }

    #[test]
    fn attachments_go_out_once_uploaded_and_block_the_send_until_then() {
        let mut d = Draft::new("d1".into(), "a1", "me@example.com");
        d.to = "ada@example.com".into();
        d.attachments.push(Attachment {
            key: "k1".into(),
            filename: "plan.pdf".into(),
            size: 2048,
            state: AttachmentState::Uploading,
        });
        assert_eq!(d.content(), Err(ComposeError::Uploading));
        assert!(d.autosave_content().unwrap().attachments.is_empty(), "an attachment-only draft is saved");
        d.attachments[0].state = AttachmentState::Ready { upload_id: "u1".into() };
        let c = d.content().unwrap();
        assert_eq!(c.attachments, vec![DraftAttachment::Upload { upload_id: "u1".into() }]);
        d.attachments.push(Attachment {
            key: "k2".into(),
            filename: "big.mov".into(),
            size: MAX_ATTACHMENT_BYTES,
            state: AttachmentState::Ready { upload_id: "u2".into() },
        });
        assert_eq!(d.content(), Err(ComposeError::TooLarge));
        d.attachments[1].state = AttachmentState::Failed("x".into());
        assert_eq!(d.content(), Err(ComposeError::FailedAttachment));
        assert_eq!(content_type_for("Q3.PDF"), "application/pdf");
        assert_eq!(content_type_for("noext"), "application/octet-stream");
        assert_eq!(file_size(2048), "2 KB");
        assert_eq!(file_size(3 * 1024 * 1024 + 1), "3.0 MB");
    }
}
