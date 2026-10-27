//! AI in the composer and in the background (§12.1): write with AI (`compose`), rewrite, BYO
//! auto-triage (§12.5), action items and AI search. Pure helpers; the backend runs the features on
//! the user's route and the workspace draws them. Nothing here talks to the network.

use std::collections::HashSet;
use std::ops::Range;

use akatsuki_ai::transcript::{RenderAddr, TriageItem};
use akatsuki_core::convex::client::SplitView;
use akatsuki_core::convex::rows::{AiSettings, LabelView, TriageInput};
use akatsuki_core::convex::{AiFeature, AiProviderKind, TriageCategory, TriageResult};
use serde::Deserialize;

/// Rewrite styles (`rewrite` input `style`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RewriteStyle {
    /// Shorter.
    Shorter,
    /// Friendlier.
    Friendlier,
    /// More formal.
    Formal,
    /// Fix spelling and grammar.
    Fix,
}

impl RewriteStyle {
    /// Every style, in menu order.
    pub const ALL: [RewriteStyle; 4] = [Self::Shorter, Self::Friendlier, Self::Formal, Self::Fix];

    /// The wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shorter => "shorter",
            Self::Friendlier => "friendlier",
            Self::Formal => "formal",
            Self::Fix => "fix",
        }
    }

    /// Menu label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Shorter => "Shorter",
            Self::Friendlier => "Friendlier",
            Self::Formal => "More formal",
            Self::Fix => "Fix spelling and grammar",
        }
    }

    /// Parses the wire value.
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// The part of a draft body AI may change: everything above the quoted reply or forwarded
/// message (the quote stays as it is).
pub fn editable_region(body: &str) -> Range<usize> {
    0..akatsuki_mime::quote::text_quote_start(body).unwrap_or(body.len())
}

/// What an AI request in the composer replaces, and the body before it ran (for Undo).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiEdit {
    /// The request.
    pub request: u64,
    /// `compose` or `rewrite`.
    pub feature: AiFeature,
    /// Replaced byte range of `before`.
    pub range: Range<usize>,
    /// The body when the request started.
    pub before: String,
}

impl AiEdit {
    /// A write over the editable region, or a rewrite of `selection` (or of the region when
    /// nothing is selected). `None` when there is nothing to rewrite.
    pub fn new(request: u64, feature: AiFeature, body: &str, selection: Range<usize>) -> Option<Self> {
        let region = editable_region(body);
        let range = if feature == AiFeature::Rewrite && selection.start < selection.end && selection.end <= body.len() {
            selection
        } else {
            region
        };
        if feature == AiFeature::Rewrite && body[range.clone()].trim().is_empty() {
            return None;
        }
        Some(Self { request, feature, range, before: body.to_owned() })
    }

    /// The text the model gets: the selection or region (`rewrite.text`, `compose.draft`).
    pub fn input(&self) -> &str {
        &self.before[self.range.clone()]
    }

    /// The body with `output` in place of the range. A write keeps a blank line before the quote.
    pub fn apply(&self, output: &str) -> String {
        let tail = &self.before[self.range.end..];
        let mut out = String::with_capacity(self.before.len() + output.len());
        out.push_str(&self.before[..self.range.start]);
        if self.feature == AiFeature::Compose && !tail.is_empty() {
            out.push_str(output.trim_end());
            out.push_str("\n\n");
            out.push_str(tail.trim_start_matches(['\n', '\r']));
        } else {
            out.push_str(output);
            out.push_str(tail);
        }
        out
    }
}

/// `{key, category, needsReply}` from the triage output schema.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TriageItemOut {
    key: String,
    category: String,
    needs_reply: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct TriageOut {
    items: Vec<TriageItemOut>,
}

/// The triage renderer's input from claimed threads (headers and snippet only, §12.7 rule 2).
pub fn triage_items(inputs: &[TriageInput]) -> Vec<TriageItem> {
    inputs
        .iter()
        .map(|t| TriageItem {
            thread_id: t.thread_id.as_str().to_owned(),
            from: t.from.as_ref().map(RenderAddr::from),
            to: t.to.iter().map(RenderAddr::from).collect(),
            subject: t.subject.clone(),
            snippet: t.snippet.clone(),
            list_id: t.list_id.clone(),
            is_bulk: t.is_bulk,
            has_attachments: t.has_attachments,
        })
        .collect()
}

/// Maps the model's keyed answer back to threads; unknown keys, unknown categories and
/// duplicates are dropped (`triageResults` in `packages/ai`).
pub fn triage_results(
    keys: &[(String, String)],
    output: &serde_json::Value,
    model: &str,
    prompt_version: u32,
) -> Vec<TriageResult> {
    let Ok(out) = serde_json::from_value::<TriageOut>(output.clone()) else { return Vec::new() };
    let mut seen = HashSet::new();
    out.items
        .into_iter()
        .filter_map(|item| {
            let thread = keys.iter().find(|(k, _)| *k == item.key).map(|(_, t)| t.clone())?;
            let category = TriageCategory::from_name(&item.category)?;
            seen.insert(thread.clone()).then(|| TriageResult {
                thread_id: thread.as_str().into(),
                category,
                needs_reply: item.needs_reply,
                model: model.to_owned(),
                prompt_version,
            })
        })
        .collect()
}

/// Whether this device runs auto-triage (§12.5): consent and the auto-triage switch are on, the
/// triage route is a BYO or local profile (the worker triages for the managed one), and the
/// profile's key is on this device when it needs one.
pub fn triages_here(settings: &AiSettings, has_key: impl Fn(&str) -> bool) -> bool {
    if settings.consent.granted_at.is_none() || !settings.consent.auto_triage {
        return false;
    }
    let Ok(r) = akatsuki_ai::resolve(AiFeature::Triage, settings, &akatsuki_ai::ROUTING) else { return false };
    match r.profile.kind {
        AiProviderKind::Akatsuki => false,
        AiProviderKind::Ollama | AiProviderKind::OpenaiCompatible => true,
        _ => has_key(&r.profile.profile_id),
    }
}

/// `nl_search` vars: the request, today and the zone, and the user's label and split names.
pub fn nl_search_vars(
    query: &str,
    today: &str,
    timezone: &str,
    labels: &[LabelView],
    splits: &[SplitView],
) -> serde_json::Map<String, serde_json::Value> {
    let mut names: Vec<String> =
        labels.iter().filter(|l| !l.hidden && l.role.is_none()).map(|l| l.name.clone()).collect();
    names.sort();
    names.dedup();
    let mut vars = serde_json::Map::new();
    vars.insert("query".into(), query.into());
    vars.insert("today".into(), today.into());
    vars.insert("timezone".into(), timezone.into());
    vars.insert("labels".into(), names.join("\n").into());
    vars.insert("splits".into(), splits.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("\n").into());
    vars
}

/// One action item (`extract_actions` output).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ActionItem {
    /// `todo`, `deadline`, `meeting`, `payment` or `reply`.
    pub kind: String,
    /// What to do.
    pub title: String,
    /// `YYYY-MM-DD`, when the mail gives one.
    pub due: Option<String>,
    /// The words it came from.
    pub quote: String,
}

impl ActionItem {
    /// The kind as a sentence-case chip label ("To-do", "Deadline"); unknown kinds show as sent.
    pub fn kind_label(&self) -> &str {
        match self.kind.as_str() {
            "todo" => "To-do",
            "deadline" => "Deadline",
            "meeting" => "Meeting",
            "payment" => "Payment",
            "reply" => "Reply",
            k => k,
        }
    }
}

/// The action items of an `extract_actions` result.
pub fn action_items(output: &serde_json::Value) -> Vec<ActionItem> {
    output.get("actions").and_then(|a| serde_json::from_value::<Vec<ActionItem>>(a.clone()).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use akatsuki_core::convex::rows::{AiConsent, AiModels, AiProfile, AiRoute, AiRouteFeature};
    use serde_json::json;

    #[test]
    fn writes_keep_the_quote_and_rewrites_take_the_selection() {
        let body = "draft\n\nOn Mon, Ada wrote:\n> Lunch?\n";
        assert_eq!(&body[editable_region(body)], "draft\n\n");
        let w = AiEdit::new(1, AiFeature::Compose, body, 0..0).unwrap();
        assert_eq!(w.input(), "draft\n\n");
        assert_eq!(w.apply("Yes, lunch works.\n"), "Yes, lunch works.\n\nOn Mon, Ada wrote:\n> Lunch?\n");
        let new_mail = AiEdit::new(2, AiFeature::Compose, "", 0..0).unwrap();
        assert_eq!(new_mail.apply("Hello"), "Hello", "no quote, nothing appended");

        let r = AiEdit::new(3, AiFeature::Rewrite, "one two three", 4..7).unwrap();
        assert_eq!(r.input(), "two");
        assert_eq!(r.apply("2"), "one 2 three");
        let whole = AiEdit::new(4, AiFeature::Rewrite, "fix me pls\n\nOn Mon, Ada wrote:\n> x", 0..0).unwrap();
        assert_eq!(whole.input(), "fix me pls\n\n");
        assert!(AiEdit::new(5, AiFeature::Rewrite, "  \n\nOn Mon, Ada wrote:\n> x", 0..0).is_none());
        assert_eq!(RewriteStyle::from_name("formal"), Some(RewriteStyle::Formal));
    }

    #[test]
    fn triage_answers_map_back_to_threads() {
        let keys = vec![("1".to_owned(), "t1".to_owned()), ("2".to_owned(), "t2".to_owned())];
        let out = json!({"items": [
            {"key": "1", "category": "receipts", "needsReply": false},
            {"key": "1", "category": "important", "needsReply": true},
            {"key": "9", "category": "other", "needsReply": false},
            {"key": "2", "category": "bogus", "needsReply": false}
        ]});
        let r = triage_results(&keys, &out, "m", 1);
        assert_eq!(r.len(), 1, "duplicates, unknown keys and unknown categories are dropped");
        assert_eq!(r[0].thread_id.as_str(), "t1");
        assert_eq!(r[0].category, TriageCategory::Receipts);
        assert!(triage_results(&keys, &json!({"nope": 1}), "m", 1).is_empty());
    }

    fn settings(kind: AiProviderKind, auto: bool) -> AiSettings {
        AiSettings {
            profiles: vec![AiProfile {
                profile_id: "p1".into(),
                label: "P".into(),
                kind,
                base_url: (kind == AiProviderKind::OpenaiCompatible).then(|| "http://127.0.0.1:8080/v1".into()),
                models: AiModels { fast: Some("m".into()), default: Some("m".into()), ..AiModels::default() },
                key_on_devices: vec![],
            }],
            routes: vec![AiRoute {
                feature: AiRouteFeature::DEFAULT,
                profile_id: "p1".into(),
                model: None,
                enabled: true,
            }],
            consent: AiConsent { granted_at: Some(1), version: Some(1), auto_triage: auto, sync_voice_profile: false },
            ..AiSettings::default()
        }
    }

    #[test]
    fn only_byo_routes_with_consent_and_a_key_triage_on_the_device() {
        assert!(triages_here(&settings(AiProviderKind::OpenaiCompatible, true), |_| false), "local needs no key");
        assert!(!triages_here(&settings(AiProviderKind::OpenaiCompatible, false), |_| true), "switch off");
        assert!(!triages_here(&settings(AiProviderKind::Openrouter, true), |_| false), "key not on this device");
        assert!(triages_here(&settings(AiProviderKind::Openrouter, true), |id| id == "p1"));
        assert!(!triages_here(&settings(AiProviderKind::Akatsuki, true), |_| true), "the worker triages managed");
        let mut no_consent = settings(AiProviderKind::Ollama, true);
        no_consent.consent.granted_at = None;
        assert!(!triages_here(&no_consent, |_| true));
    }

    #[test]
    fn nl_search_gets_names_only_and_actions_decode() {
        let labels: Vec<LabelView> = serde_json::from_value(json!([
            {"_id": "l1", "_creationTime": 1.0, "accountId": "a", "name": "Travel", "kind": "user", "exclusive": false,
             "hidden": false, "pendingDelete": false, "labelKey": "l:l1"},
            {"_id": "l2", "_creationTime": 1.0, "accountId": "a", "name": "INBOX", "kind": "system", "role": "inbox",
             "exclusive": true, "hidden": false, "pendingDelete": false, "labelKey": "sys:inbox"}
        ]))
        .unwrap();
        let v = nl_search_vars("mail from mira last week", "2026-10-20", "UTC", &labels, &[]);
        assert_eq!(v["labels"], "Travel");
        assert_eq!(v["splits"], "");
        let a = action_items(
            &json!({"actions": [{"kind": "todo", "title": "Send deck", "due": null, "quote": "send the deck"}]}),
        );
        assert_eq!(a[0].title, "Send deck");
        assert!(action_items(&json!({})).is_empty());
    }
}
