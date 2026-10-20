//! FETCH data → [`RemoteMessage`] (headers, flags, `BODYSTRUCTURE`, snippet, thread id).

use imap_proto::{BodyStructure, ContentEncoding};

use super::folders::{KEYWORD_PREFIX, is_ignored_keyword};
use super::session::FetchItem;
use crate::{ImapLocation, LabelRole, RemoteMessage, SysLabel, imap_thread};

/// Header fields fetched for list views (§9.3 InboxFirst).
pub const HEADER_FIELDS: &str = "FROM TO CC BCC REPLY-TO SUBJECT DATE MESSAGE-ID IN-REPLY-TO REFERENCES LIST-ID \
     LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST PRECEDENCE CONTENT-TYPE AUTHENTICATION-RESULTS";

/// Bytes of the text part fetched for the snippet (§9.5: the first 1 KB).
pub const SNIPPET_BYTES: u32 = 1024;

/// The `FETCH` item list for the metadata pass.
pub fn metadata_items(x_gm_ext: bool, condstore: bool) -> String {
    let mut s = format!("(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE BODY.PEEK[HEADER.FIELDS ({HEADER_FIELDS})]");
    if x_gm_ext {
        s.push_str(" X-GM-THRID");
    }
    if condstore {
        s.push_str(" MODSEQ");
    }
    s.push(')');
    s
}

/// What `BODYSTRUCTURE` tells the list view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyInfo {
    /// A part is an attachment (disposition `attachment`, a file name, or an attached message).
    pub has_attachments: bool,
    /// The part the snippet is read from: the first `text/plain`, else the first `text/html`.
    pub text_part: Option<TextPart>,
}

/// A text body part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPart {
    /// IMAP section path (`1`, `1.1`).
    pub path: String,
    /// `plain` or `html`.
    pub subtype: String,
    /// `charset` parameter.
    pub charset: Option<String>,
    /// `Content-Transfer-Encoding`.
    pub encoding: String,
}

fn param<'a>(params: &'a imap_proto::BodyParams<'_>, name: &str) -> Option<&'a str> {
    params.as_ref()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_ref())
}

fn encoding(e: &ContentEncoding<'_>) -> String {
    match e {
        ContentEncoding::SevenBit => "7bit".into(),
        ContentEncoding::EightBit => "8bit".into(),
        ContentEncoding::Binary => "binary".into(),
        ContentEncoding::Base64 => "base64".into(),
        ContentEncoding::QuotedPrintable => "quoted-printable".into(),
        ContentEncoding::Other(o) => o.to_ascii_lowercase(),
    }
}

/// Reads `BODYSTRUCTURE`.
pub fn analyze_structure(bs: &BodyStructure<'_>) -> BodyInfo {
    let mut info = BodyInfo::default();
    let mut html: Option<TextPart> = None;
    walk(bs, "", &mut info, &mut html, 0);
    if info.text_part.is_none() {
        info.text_part = html;
    }
    info
}

fn child_path(prefix: &str, i: usize) -> String {
    if prefix.is_empty() { (i + 1).to_string() } else { format!("{prefix}.{}", i + 1) }
}

fn walk(bs: &BodyStructure<'_>, path: &str, info: &mut BodyInfo, html: &mut Option<TextPart>, depth: u32) {
    if depth > 32 {
        return;
    }
    let leaf_path = if path.is_empty() { "1".to_owned() } else { path.to_owned() };
    match bs {
        BodyStructure::Multipart { bodies, .. } => {
            for (i, b) in bodies.iter().enumerate() {
                walk(b, &child_path(path, i), info, html, depth + 1);
            }
        }
        BodyStructure::Text { common, other, .. } => {
            let attachment = common.disposition.as_ref().is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
            if attachment {
                info.has_attachments = true;
                return;
            }
            let sub = common.ty.subtype.to_ascii_lowercase();
            let part = TextPart {
                path: leaf_path,
                subtype: sub.clone(),
                charset: param(&common.ty.params, "charset").map(str::to_owned),
                encoding: encoding(&other.transfer_encoding),
            };
            if sub == "plain" && info.text_part.is_none() {
                info.text_part = Some(part);
            } else if sub == "html" && html.is_none() {
                *html = Some(part);
            }
        }
        BodyStructure::Message { .. } => info.has_attachments = true,
        BodyStructure::Basic { common, .. } => {
            let disposition = common.disposition.as_ref();
            let attachment = disposition.is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
            let named = param(&common.ty.params, "name").is_some()
                || disposition.is_some_and(|d| param(&d.params, "filename").is_some());
            let inline = disposition.is_some_and(|d| d.ty.eq_ignore_ascii_case("inline"));
            if attachment || (named && !inline) {
                info.has_attachments = true;
            }
        }
    }
}

/// The snippet from the first bytes of a text part (decoded with its charset and transfer
/// encoding, quoted history removed, ≤ 200 chars).
pub fn snippet_from_partial(tp: &TextPart, data: &[u8]) -> String {
    let mut data = data;
    // A cut in the middle of a base64 quartet or a QP escape would garble the tail: keep whole lines.
    if matches!(tp.encoding.as_str(), "base64" | "quoted-printable")
        && data.len() >= SNIPPET_BYTES as usize
        && let Some(nl) = data.iter().rposition(|b| *b == b'\n')
    {
        data = &data[..nl];
    }
    let mut synthetic = format!(
        "Content-Type: text/{}; charset=\"{}\"\r\nContent-Transfer-Encoding: {}\r\n\r\n",
        tp.subtype,
        tp.charset.as_deref().unwrap_or("utf-8").replace(['"', '\r', '\n'], ""),
        tp.encoding.replace(['\r', '\n'], "")
    )
    .into_bytes();
    synthetic.extend_from_slice(data);
    match akatsuki_mime::parse::parse_message(&synthetic) {
        Ok(p) => akatsuki_mime::body::snippet(&p),
        Err(_) => String::new(),
    }
}

/// Where a fetched message lives.
#[derive(Clone, Debug)]
pub struct FolderCtx<'a> {
    /// Mailbox (UTF-8).
    pub name: &'a str,
    /// `UIDVALIDITY`.
    pub uid_validity: u32,
    /// The folder's role.
    pub role: Option<LabelRole>,
    /// Keywords map to labels (`PERMANENTFLAGS` has `\*`).
    pub keywords_allowed: bool,
}

/// `"<remoteFolderId>:<uidValidity>:<uid>"` (§9.5).
pub fn message_id(folder: &str, uid_validity: u32, uid: u32) -> String {
    format!("{folder}:{uid_validity}:{uid}")
}

/// Parses a §9.5 message id back into a location (the folder may itself contain `:`).
pub fn parse_message_id(id: &str) -> Option<ImapLocation> {
    let (rest, uid) = id.rsplit_once(':')?;
    let (folder, validity) = rest.rsplit_once(':')?;
    Some(ImapLocation {
        remote_folder_id: folder.to_owned(),
        uid_validity: validity.parse().ok()?,
        uid: uid.parse().ok()?,
    })
}

/// The system labels a flag set and a folder role imply.
pub fn sys_labels(flags: &[String], role: Option<LabelRole>) -> Vec<SysLabel> {
    let has = |f: &str| flags.iter().any(|x| x.eq_ignore_ascii_case(f));
    let mut sys = Vec::new();
    if !has("\\Seen") {
        sys.push(SysLabel::Unread);
    }
    if has("\\Flagged") {
        sys.push(SysLabel::Starred);
    }
    if has("$Important") {
        sys.push(SysLabel::Important);
    }
    if has("\\Draft") {
        sys.push(SysLabel::Drafts);
    }
    if let Some(s) = role.and_then(LabelRole::sys) {
        sys.push(s);
    }
    sys.sort();
    sys.dedup();
    sys
}

/// Builds the header-level message. `None` without a UID.
pub(crate) fn build_message(
    ctx: &FolderCtx<'_>,
    item: &FetchItem,
    account_id: &str,
    snippet: String,
) -> Option<RemoteMessage> {
    let uid = item.uid?;
    let header = item.section("HEADER").unwrap_or_default();
    let parsed = akatsuki_mime::parse::parse_message(header).unwrap_or_default();
    let flags = item.flags.clone().unwrap_or_default();
    let date = parsed.date.or(item.internal_date).unwrap_or(0);
    let from_email = parsed.from.as_ref().map(|a| a.email.clone()).unwrap_or_default();
    let root = imap_thread::thread_root(
        &parsed.references,
        parsed.in_reply_to.as_deref(),
        parsed.rfc_message_id.as_deref(),
        &from_email,
        date,
        &parsed.subject,
    );
    let provider_thread_id = match item.gm_thrid {
        Some(t) => format!("{t:x}"),
        None => imap_thread::thread_id(account_id, &root),
    };
    let mut label_ids = vec![ctx.name.to_owned()];
    if ctx.keywords_allowed {
        for f in &flags {
            if !f.starts_with('\\') && !is_ignored_keyword(f) {
                label_ids.push(format!("{KEYWORD_PREFIX}{f}"));
            }
        }
    }
    let sys = sys_labels(&flags, ctx.role);
    let is_draft = sys.contains(&SysLabel::Drafts);
    Some(RemoteMessage {
        provider_message_id: message_id(ctx.name, ctx.uid_validity, uid),
        provider_thread_id,
        rfc_message_id: parsed.rfc_message_id.clone(),
        in_reply_to: parsed.in_reply_to.clone(),
        references: parsed.references.clone(),
        from: parsed.from.clone(),
        to: parsed.to.clone(),
        cc: parsed.cc.clone(),
        bcc: parsed.bcc.clone(),
        reply_to: parsed.reply_to.clone(),
        subject: parsed.subject.clone(),
        snippet,
        date,
        provider_label_ids: label_ids,
        sys,
        is_draft,
        has_attachments: item.structure.as_ref().is_some_and(|s| s.has_attachments),
        size_bytes: item.size.map(u64::from),
        list_id: parsed.list_id.clone(),
        list_unsubscribe: parsed.list_unsubscribe.clone(),
        auth: parsed.auth.clone(),
        imap: vec![ImapLocation { remote_folder_id: ctx.name.to_owned(), uid_validity: ctx.uid_validity, uid }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip() {
        let id = message_id("Archive:2026", 7, 42);
        assert_eq!(id, "Archive:2026:7:42");
        let loc = parse_message_id(&id).unwrap();
        assert_eq!((loc.remote_folder_id.as_str(), loc.uid_validity, loc.uid), ("Archive:2026", 7, 42));
        assert!(parse_message_id("nope").is_none());
    }

    #[test]
    fn flags_to_sys() {
        let f = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(sys_labels(&f(&[]), Some(LabelRole::Inbox)), vec![SysLabel::Inbox, SysLabel::Unread]);
        assert_eq!(sys_labels(&f(&["\\Seen", "\\Flagged"]), None), vec![SysLabel::Starred]);
        assert_eq!(sys_labels(&f(&["\\Seen", "\\Draft"]), Some(LabelRole::Archive)), vec![SysLabel::Drafts]);
        assert_eq!(
            sys_labels(&f(&["\\seen", "$Important"]), Some(LabelRole::Snoozed)),
            vec![SysLabel::Important, SysLabel::Snoozed]
        );
    }

    #[test]
    fn snippet_decodes_partial_base64_and_qp() {
        use base64::Engine as _;
        let text = "Hello there, this is the body.\n".repeat(60);
        let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let wrapped: String =
            b64.as_bytes().chunks(76).map(|c| format!("{}\r\n", String::from_utf8_lossy(c))).collect();
        let partial = &wrapped.as_bytes()[..1024];
        let tp = TextPart {
            path: "1".into(),
            subtype: "plain".into(),
            charset: Some("utf-8".into()),
            encoding: "base64".into(),
        };
        let s = snippet_from_partial(&tp, partial);
        assert!(s.starts_with("Hello there, this is the body. Hello there"), "{s}");
        assert!(s.chars().count() <= 200);

        let qp = TextPart { encoding: "quoted-printable".into(), charset: Some("iso-8859-1".into()), ..tp };
        assert_eq!(snippet_from_partial(&qp, b"Gr=FC=DFe aus K=F6ln"), "Grüße aus Köln");
    }
}
