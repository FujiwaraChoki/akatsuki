//! Graph resources → provider-neutral types (ARCHITECTURE §9.5 "Graph").
//!
//! - Thread id: `conversationId`. Version: the newest `lastModifiedDateTime` of the thread, in ms.
//! - A message's provider label ids are its `parentFolderId` plus one `cat:<name>` id per
//!   category. Categories are referenced by display name on messages, and Graph does not allow
//!   renaming a master category, so the name is the stable id (D-212).
//! - `isRead=false` → `sys:unread`, `flag.flagStatus=flagged` → `sys:starred`, `isDraft` →
//!   `sys:drafts`. Folder roles reach Convex through `RemoteLabel.role`.

use std::collections::HashMap;

use akatsuki_mime::parse::{parse_auth_results, parse_list_unsubscribe};

use crate::labels::SNOOZED_LABEL_NAME;
use crate::{Addr, FetchedThread, LabelKind, LabelRole, RemoteLabel, RemoteMessage, SysLabel, UnixMs};

use super::model::{Category, Folder, Header, Message, Recipient};

/// Prefix of category label ids.
pub const CATEGORY_PREFIX: &str = "cat:";

/// The label id of a category.
pub fn category_label_id(name: &str) -> String {
    format!("{CATEGORY_PREFIX}{name}")
}

/// The category name of a label id, if it is one.
pub fn category_name(provider_label_id: &str) -> Option<&str> {
    provider_label_id.strip_prefix(CATEGORY_PREFIX)
}

/// Parses an ISO 8601 instant (`2026-10-01T09:30:00Z`, with or without fractional seconds).
pub fn parse_time(s: &str) -> Option<UnixMs> {
    s.parse::<jiff::Timestamp>().ok().map(|t| t.as_millisecond())
}

/// Formats an instant for OData filters (`2026-10-01T09:30:00Z`).
pub fn format_time(ms: UnixMs) -> String {
    jiff::Timestamp::from_millisecond(ms).map(|t| t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default()
}

/// The UTC calendar date nearest to an instant (`2026-10-01`), for KQL date terms.
pub fn format_date(ms: UnixMs) -> String {
    jiff::Timestamp::from_millisecond(ms).map(|t| t.strftime("%Y-%m-%d").to_string()).unwrap_or_default()
}

fn addr(r: &Recipient) -> Option<Addr> {
    let e = r.email_address.as_ref()?;
    let address = e.address.as_deref()?.trim();
    (!address.is_empty()).then(|| Addr::new(e.name.as_deref(), address))
}

fn addrs(list: Option<&Vec<Recipient>>) -> Vec<Addr> {
    list.map(|l| l.iter().filter_map(addr).collect()).unwrap_or_default()
}

fn clean_id(id: &str) -> Option<String> {
    let id = id.trim().trim_start_matches('<').trim_end_matches('>').trim();
    (!id.is_empty()).then(|| id.to_owned())
}

fn ids(v: &str) -> Vec<String> {
    v.split(|c: char| c.is_whitespace() || c == ',').filter_map(clean_id).collect()
}

fn header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers.iter().find(|h| h.name.eq_ignore_ascii_case(name)).map(|h| h.value.trim()).filter(|v| !v.is_empty())
}

fn list_id(raw: &str) -> Option<String> {
    match (raw.rfind('<'), raw.rfind('>')) {
        (Some(a), Some(b)) if b > a + 1 => Some(raw[a + 1..b].trim().to_lowercase()),
        _ => Some(raw.trim().to_lowercase()).filter(|s| !s.is_empty()),
    }
}

fn snippet(preview: &str) -> String {
    preview.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Maps one message. `None` when Graph sent no `conversationId` (never for real messages).
pub fn message(m: &Message) -> Option<RemoteMessage> {
    let thread = m.conversation_id.clone().filter(|c| !c.is_empty())?;
    let headers = m.internet_message_headers.as_deref().unwrap_or(&[]);
    let mut labels = Vec::new();
    if let Some(f) = &m.parent_folder_id {
        labels.push(f.clone());
    }
    labels.extend(m.category_names().iter().map(|c| category_label_id(c)));
    let mut sys = Vec::new();
    if m.unread() && !m.draft() {
        sys.push(SysLabel::Unread);
    }
    if m.flagged() {
        sys.push(SysLabel::Starred);
    }
    if m.draft() {
        sys.push(SysLabel::Drafts);
    }
    let date = m
        .received_date_time
        .as_deref()
        .and_then(parse_time)
        .or_else(|| m.sent_date_time.as_deref().and_then(parse_time))
        .or_else(|| m.last_modified_date_time.as_deref().and_then(parse_time))
        .unwrap_or(0);
    let references = header(headers, "References").map(ids).unwrap_or_default();
    Some(RemoteMessage {
        provider_message_id: m.id.clone(),
        provider_thread_id: thread,
        rfc_message_id: m.internet_message_id.as_deref().and_then(clean_id),
        in_reply_to: header(headers, "In-Reply-To").and_then(|v| ids(v).into_iter().next()),
        references,
        from: m.from.as_ref().and_then(addr).or_else(|| m.sender.as_ref().and_then(addr)),
        to: addrs(m.to_recipients.as_ref()),
        cc: addrs(m.cc_recipients.as_ref()),
        bcc: addrs(m.bcc_recipients.as_ref()),
        reply_to: addrs(m.reply_to.as_ref()),
        subject: m.subject.clone().unwrap_or_default().trim().to_owned(),
        snippet: snippet(m.body_preview.as_deref().unwrap_or_default()),
        date,
        provider_label_ids: labels,
        sys,
        is_draft: m.draft(),
        has_attachments: m.has_attachments.unwrap_or(false),
        size_bytes: None,
        list_id: header(headers, "List-Id").and_then(list_id),
        list_unsubscribe: header(headers, "List-Unsubscribe")
            .and_then(|h| parse_list_unsubscribe(h, header(headers, "List-Unsubscribe-Post"))),
        auth: header(headers, "Authentication-Results").and_then(parse_auth_results),
        imap: Vec::new(),
    })
}

/// Groups messages into threads by `conversationId`, oldest message first, and sets the version
/// to the newest `lastModifiedDateTime`. Threads keep the order of `order`; ids without messages
/// are left out.
pub fn threads(order: &[String], messages: Vec<Message>) -> Vec<FetchedThread> {
    let mut by: HashMap<String, (Vec<RemoteMessage>, Option<i64>)> = HashMap::new();
    for m in &messages {
        let Some(r) = message(m) else { continue };
        let modified = m.last_modified_date_time.as_deref().and_then(parse_time);
        let e = by.entry(r.provider_thread_id.clone()).or_default();
        e.1 = match (e.1, modified) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if !e.0.iter().any(|x| x.provider_message_id == r.provider_message_id) {
            e.0.push(r);
        }
    }
    order
        .iter()
        .filter_map(|id| {
            let (mut msgs, version) = by.remove(id)?;
            msgs.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.provider_message_id.cmp(&b.provider_message_id)));
            Some(FetchedThread {
                provider_thread_id: id.clone(),
                provider_version: version,
                messages: msgs,
                partial: false,
            })
        })
        .collect()
}

/// Graph well-known folder names that become labels, with their role.
pub const SYNCED_WELL_KNOWN: [(&str, LabelRole); 6] = [
    ("inbox", LabelRole::Inbox),
    ("sentitems", LabelRole::Sent),
    ("drafts", LabelRole::Drafts),
    ("deleteditems", LabelRole::Trash),
    ("junkemail", LabelRole::Spam),
    ("archive", LabelRole::Archive),
];

/// Well-known folders that are not mail lists and are never synced (with their subtrees).
pub const SKIPPED_WELL_KNOWN: [&str; 5] = ["outbox", "conversationhistory", "syncissues", "scheduled", "clutter"];

/// Folder tree → labels. `well_known` maps well-known names to folder ids.
pub fn folder_labels(folders: &[Folder], well_known: &HashMap<String, String>) -> Vec<(RemoteLabel, String)> {
    let by_id: HashMap<&str, &Folder> = folders.iter().map(|f| (f.id.as_str(), f)).collect();
    let role_of: HashMap<&str, LabelRole> = SYNCED_WELL_KNOWN
        .iter()
        .filter_map(|(name, role)| well_known.get(*name).map(|id| (id.as_str(), *role)))
        .collect();
    let skipped: Vec<&str> = SKIPPED_WELL_KNOWN.iter().filter_map(|n| well_known.get(*n).map(String::as_str)).collect();

    // Path and skip status by walking up the parents (bounded against cycles).
    let resolve = |f: &Folder| -> Option<String> {
        let mut parts = vec![f.display_name.clone().unwrap_or_default()];
        let mut cur = f;
        for _ in 0..32 {
            if skipped.contains(&cur.id.as_str()) || cur.is_hidden == Some(true) {
                return None;
            }
            match cur.parent_folder_id.as_deref().and_then(|p| by_id.get(p)) {
                Some(parent) => {
                    parts.push(parent.display_name.clone().unwrap_or_default());
                    cur = parent;
                }
                None => break,
            }
        }
        parts.reverse();
        Some(parts.join("/"))
    };

    folders
        .iter()
        .filter_map(|f| {
            let path = resolve(f)?;
            let (kind, role) = match role_of.get(f.id.as_str()) {
                Some(LabelRole::Archive) => (LabelKind::Folder, Some(LabelRole::Archive)),
                Some(r) => (LabelKind::System, Some(*r)),
                None if path == SNOOZED_LABEL_NAME => (LabelKind::Folder, Some(LabelRole::Snoozed)),
                None => (LabelKind::Folder, None),
            };
            Some((
                RemoteLabel {
                    provider_label_id: f.id.clone(),
                    name: path.clone(),
                    kind,
                    role,
                    exclusive: true,
                    color: None,
                    hidden: false,
                },
                path,
            ))
        })
        .collect()
}

/// Outlook's 25 category colour presets with their approximate RGB values.
pub const PRESETS: [(&str, &str); 25] = [
    ("preset0", "#e7a1a2"),
    ("preset1", "#f9ba89"),
    ("preset2", "#f7dd8f"),
    ("preset3", "#fcfa90"),
    ("preset4", "#78d168"),
    ("preset5", "#9fdcc9"),
    ("preset6", "#c6d2b0"),
    ("preset7", "#9db7e8"),
    ("preset8", "#b5a1e2"),
    ("preset9", "#daaec2"),
    ("preset10", "#dad9dc"),
    ("preset11", "#6b7994"),
    ("preset12", "#bfbfbf"),
    ("preset13", "#6f6f6f"),
    ("preset14", "#4f4f4f"),
    ("preset15", "#c11a25"),
    ("preset16", "#e2620d"),
    ("preset17", "#c79930"),
    ("preset18", "#b9b300"),
    ("preset19", "#368f2b"),
    ("preset20", "#329b7a"),
    ("preset21", "#778b45"),
    ("preset22", "#2858a5"),
    ("preset23", "#5c3fa3"),
    ("preset24", "#93446b"),
];

/// A preset → its colour (`None` for `none` or unknown values).
pub fn preset_hex(preset: &str) -> Option<String> {
    PRESETS.iter().find(|(p, _)| p.eq_ignore_ascii_case(preset)).map(|(_, h)| (*h).to_owned())
}

fn rgb(hex: &str) -> Option<(i32, i32, i32)> {
    let h = hex.strip_prefix('#').unwrap_or(hex);
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| i32::from_str_radix(&h[i..i + 2], 16).ok();
    Some((c(0)?, c(2)?, c(4)?))
}

/// A colour (a preset name or `#rrggbb`) → the nearest preset.
pub fn nearest_preset(color: &str) -> Option<&'static str> {
    if let Some((p, _)) = PRESETS.iter().find(|(p, _)| p.eq_ignore_ascii_case(color)) {
        return Some(p);
    }
    let (r, g, b) = rgb(color)?;
    PRESETS
        .iter()
        .filter_map(|(p, h)| rgb(h).map(|(r2, g2, b2)| (*p, (r - r2).pow(2) + (g - g2).pow(2) + (b - b2).pow(2))))
        .min_by_key(|(_, d)| *d)
        .map(|(p, _)| p)
}

/// A master category → label.
pub fn category_label(c: &Category) -> RemoteLabel {
    RemoteLabel {
        provider_label_id: category_label_id(&c.display_name),
        name: c.display_name.clone(),
        kind: LabelKind::Category,
        role: None,
        exclusive: false,
        color: c.color.as_deref().and_then(preset_hex),
        hidden: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(parse_time("2026-10-01T00:00:00Z"), Some(1_790_812_800_000));
        assert_eq!(parse_time("2026-10-01T00:00:00.5Z"), Some(1_790_812_800_500));
        assert_eq!(format_time(1_790_812_800_000), "2026-10-01T00:00:00Z");
        assert_eq!(format_date(1_790_812_800_000), "2026-10-01");
        assert_eq!(parse_time("yesterday"), None);
    }

    #[test]
    fn colours() {
        assert_eq!(preset_hex("preset15").as_deref(), Some("#c11a25"));
        assert_eq!(preset_hex("none"), None);
        assert_eq!(nearest_preset("#C1272D"), Some("preset15"));
        assert_eq!(nearest_preset("preset3"), Some("preset3"));
        assert_eq!(nearest_preset("crimson"), None);
    }

    #[test]
    fn folder_paths_roles_and_skips() {
        let f = |id: &str, name: &str, parent: &str| Folder {
            id: id.into(),
            display_name: Some(name.into()),
            parent_folder_id: Some(parent.into()),
            child_folder_count: None,
            is_hidden: None,
        };
        let folders = vec![
            f("in", "Inbox", "root"),
            f("arc", "Archive", "root"),
            f("ak", "Akatsuki", "root"),
            f("sn", "Snoozed", "ak"),
            f("proj", "Projects", "in"),
            f("si", "Sync Issues", "root"),
            f("conf", "Conflicts", "si"),
        ];
        let wk: HashMap<String, String> = [("inbox", "in"), ("archive", "arc"), ("syncissues", "si")]
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .into();
        let labels = folder_labels(&folders, &wk);
        let names: Vec<_> = labels.iter().map(|(l, _)| (l.name.as_str(), l.kind, l.role)).collect();
        assert_eq!(
            names,
            vec![
                ("Inbox", LabelKind::System, Some(LabelRole::Inbox)),
                ("Archive", LabelKind::Folder, Some(LabelRole::Archive)),
                ("Akatsuki", LabelKind::Folder, None),
                ("Akatsuki/Snoozed", LabelKind::Folder, Some(LabelRole::Snoozed)),
                ("Inbox/Projects", LabelKind::Folder, None),
            ]
        );
        assert!(labels.iter().all(|(l, _)| l.exclusive));
    }
}
