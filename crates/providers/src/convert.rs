//! `FetchedThread` → Convex `ThreadUpsert`, applying the hard caps of ARCHITECTURE §1.6.

use akatsuki_core::convex::values::ConvexInt64;
use akatsuki_core::convex::{MessageUpsert, ThreadUpsert};

use crate::{Addr, FetchedThread, LabelMap, RemoteMessage, SysLabel};

/// `subject` cap (RFC 5322 line limit).
pub const SUBJECT_MAX: usize = 998;
/// `snippet` cap.
pub const SNIPPET_MAX: usize = 200;
/// `to/cc/bcc` cap per list.
pub const RECIPIENTS_MAX: usize = 100;
/// `references` kept (the last N).
pub const REFERENCES_MAX: usize = 20;

/// Truncates to at most `max` characters.
pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_owned(),
        None => s.to_owned(),
    }
}

fn cap(list: &[Addr]) -> Vec<Addr> {
    list.iter().take(RECIPIENTS_MAX).cloned().collect()
}

/// Converts one message. `identities` are the account's own addresses (lowercased) and decide
/// `isOutbound`. Provider label ids missing from `labels` are dropped.
pub fn message_upsert(m: &RemoteMessage, labels: &LabelMap, identities: &[String]) -> MessageUpsert {
    let mut keys: Vec<String> = m.provider_label_ids.iter().filter_map(|id| labels.get(id).cloned()).collect();
    keys.extend(m.sys.iter().map(|s| s.key()));
    keys.sort();
    keys.dedup();
    let is_outbound = m.from.as_ref().is_some_and(|f| identities.iter().any(|i| i.eq_ignore_ascii_case(&f.email)));
    let refs_start = m.references.len().saturating_sub(REFERENCES_MAX);
    MessageUpsert {
        provider_message_id: m.provider_message_id.clone(),
        rfc_message_id: m.rfc_message_id.clone(),
        in_reply_to: m.in_reply_to.clone(),
        references: m.references[refs_start..].to_vec(),
        from: m.from.clone(),
        to: cap(&m.to),
        cc: cap(&m.cc),
        bcc: cap(&m.bcc),
        reply_to: cap(&m.reply_to),
        subject: truncate_chars(&m.subject, SUBJECT_MAX),
        snippet: truncate_chars(&m.snippet, SNIPPET_MAX),
        date: m.date,
        is_unread: m.sys.contains(&SysLabel::Unread),
        is_draft: m.is_draft || m.sys.contains(&SysLabel::Drafts),
        label_keys: keys,
        is_outbound,
        has_attachments: m.has_attachments,
        size_bytes: m.size_bytes,
        list_id: m.list_id.clone(),
        list_unsubscribe: m.list_unsubscribe.clone(),
        auth_summary: m.auth.clone(),
        imap_locations: (!m.imap.is_empty()).then(|| m.imap.clone()),
    }
}

/// Converts a thread. `complete` means the message list is the whole thread.
pub fn thread_upsert(t: &FetchedThread, labels: &LabelMap, identities: &[String], complete: bool) -> ThreadUpsert {
    ThreadUpsert {
        provider_thread_id: t.provider_thread_id.clone(),
        provider_version: t.provider_version.map(ConvexInt64),
        complete,
        messages: t.messages.iter().map(|m| message_upsert(m, labels, identities)).collect(),
        removed_provider_message_ids: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_labels_flags_and_caps() {
        let mut labels = LabelMap::new();
        labels.insert("INBOX".into(), "sys:inbox".into());
        labels.insert("Label_1".into(), "l:abc".into());
        let m = RemoteMessage {
            provider_message_id: "m1".into(),
            provider_thread_id: "t1".into(),
            from: Some(Addr::new(None, "Me@Example.com")),
            to: (0..150).map(|i| Addr::new(None, &format!("u{i}@x.com"))).collect(),
            references: (0..30).map(|i| format!("r{i}@x")).collect(),
            subject: "é".repeat(1200),
            snippet: "s".repeat(300),
            provider_label_ids: vec!["INBOX".into(), "Label_1".into(), "Label_unknown".into()],
            sys: vec![SysLabel::Unread],
            ..Default::default()
        };
        let u = message_upsert(&m, &labels, &["me@example.com".into()]);
        assert_eq!(u.label_keys, vec!["l:abc", "sys:inbox", "sys:unread"]);
        assert!(u.is_unread && u.is_outbound && !u.is_draft);
        assert_eq!(u.to.len(), 100);
        assert_eq!(u.references.len(), 20);
        assert_eq!(u.references[0], "r10@x");
        assert_eq!(u.subject.chars().count(), 998);
        assert_eq!(u.snippet.len(), 200);
        assert_eq!(u.imap_locations, None);

        let t = FetchedThread {
            provider_thread_id: "t1".into(),
            provider_version: Some(9),
            messages: vec![m],
            partial: false,
        };
        let tu = thread_upsert(&t, &labels, &[], true);
        assert_eq!(tu.provider_version, Some(ConvexInt64(9)));
        assert!(!tu.messages[0].is_outbound);
    }
}
