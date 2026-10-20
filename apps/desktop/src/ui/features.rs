//! The workspace side of send later, snippets, AI write and rewrite, dictation, labels, Retry,
//! scheduled sends, the compose store, server and provider search, AI search and settings sync.
//! Every command here has an id in the shared key map (`packages/shortcuts`) or is a row of one of
//! its pickers, so the palette and keys reach all of it.

use gpui::{Context, Focusable, Window};

use akatsuki_ai::AaspEvent;
use akatsuki_core::convex::AiFeature;
use akatsuki_core::convex::rows::LabelView;
use akatsuki_core::convex::{SendKind, SendState};
use akatsuki_core::time::now_ms;
use akatsuki_shortcuts::{Platform, ids as sc};

use super::workspace::{Overlay, PickerItem, PickerKind, SearchState, SettingsTab, Workspace};
use crate::backend::{DictationState, Event, SearchSource};
use crate::commands::ids as cmd;
use crate::config::LocalSettings;
use crate::keys::Matcher;
use crate::vm::ai::{AiEdit, RewriteStyle};
use crate::vm::compose::{ComposeKind, parse_recipients};
use crate::vm::prefs::{self, Synced};
use crate::vm::{snippets, time, when};

/// Picker value prefix of the "Create label" row.
pub const CREATE_PREFIX: &str = "create:";

fn user_label(l: &LabelView) -> bool {
    l.role.is_none() && !l.hidden && !l.pending_delete && l.label_key.starts_with("l:")
}

impl Workspace {
    // ───────────── commands ─────────────

    /// Runs the commands this module owns; `false` when `id` is not one of them.
    pub(crate) fn run_feature_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if let Some(style) = id.strip_prefix(cmd::REWRITE_PREFIX) {
            match RewriteStyle::from_name(style) {
                Some(s) => self.run_rewrite(s, window, cx),
                None if id == cmd::AI_UNDO => self.undo_ai(window, cx),
                None => {}
            }
            return true;
        }
        if let Some(op) = id.strip_prefix(cmd::RETRY_PREFIX) {
            self.backend.retry_failure(op);
            self.toasts.retain(|t| t.key.as_deref() != Some(op));
            cx.notify();
            return true;
        }
        if let Some(op) = id.strip_prefix(cmd::DISMISS_PREFIX) {
            self.backend.dismiss_failure(op);
            self.toasts.retain(|t| t.key.as_deref() != Some(op));
            cx.notify();
            return true;
        }
        if let Some(v) = id.strip_prefix(cmd::KEEP_DRAFT_PREFIX) {
            if let (Some(c), Ok(version)) = (&mut self.compose, v.parse::<u64>()) {
                // "Keep this one": save again over the other copy (D-281).
                c.draft.base_version = version;
                c.conflict = None;
                c.dirty_at = Some(0);
            }
            self.toasts.retain(|t| t.key.as_deref() != Some("draft-conflict"));
            cx.notify();
            return true;
        }
        match id {
            cmd::SETTINGS_LABELS => self.open_settings(SettingsTab::Labels, window, cx),
            cmd::SETTINGS_SNIPPETS => self.open_settings(SettingsTab::Snippets, window, cx),
            _ if !self.signed_in() => return false,
            sc::COMPOSE_SEND_LATER if self.compose.is_some() => {
                self.open_palette(Overlay::Picker(PickerKind::SendLater), window, cx)
            }
            sc::COMPOSE_SNIPPET if self.compose.is_some() => {
                self.backend.refresh_snippets();
                self.open_palette(Overlay::Picker(PickerKind::Snippet), window, cx)
            }
            sc::COMPOSE_AI_WRITE if self.compose.is_some() => self.open_ai_bar(window, cx),
            sc::COMPOSE_REWRITE if self.compose.is_some() => {
                self.open_palette(Overlay::Picker(PickerKind::Rewrite), window, cx)
            }
            sc::COMPOSE_DICTATE if self.compose.is_some() => self.toggle_dictation(window, cx),
            sc::COMPOSE_SEND_LATER
            | sc::COMPOSE_SNIPPET
            | sc::COMPOSE_AI_WRITE
            | sc::COMPOSE_REWRITE
            | sc::COMPOSE_DICTATE => self.toast("Open a draft first", false, cx),
            sc::COMPOSE_CONTINUE_DRAFT => {
                if self.backend.local_drafts().is_empty() {
                    self.toast("No drafts on this device", false, cx);
                } else {
                    self.open_palette(Overlay::Picker(PickerKind::ContinueDraft), window, cx);
                }
            }
            sc::SENDS_SCHEDULED => {
                self.backend.list_scheduled();
                self.open_palette(Overlay::Picker(PickerKind::Scheduled), window, cx);
            }
            sc::OPS_RETRY => match self.failures.first().map(|f| f.id.as_str().to_owned()) {
                Some(op) => {
                    self.toasts.retain(|t| t.key.as_deref() != Some(op.as_str()));
                    self.backend.retry_failure(&op);
                }
                None => self.toast("Nothing to retry", false, cx),
            },
            sc::LABELS_CREATE => self.open_palette(Overlay::Picker(PickerKind::LabelCreate), window, cx),
            sc::LABELS_RENAME => self.open_palette(Overlay::Picker(PickerKind::LabelRename), window, cx),
            sc::LABELS_REMOVE => self.open_palette(Overlay::Picker(PickerKind::LabelRemove), window, cx),
            sc::SEARCH_FULL_MAILBOX => self.search_full_mailbox(window, cx),
            sc::SEARCH_AI => self.open_palette(Overlay::Picker(PickerKind::AiSearch), window, cx),
            _ => return false,
        }
        true
    }

    fn open_settings(&mut self, tab: SettingsTab, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.overlay = Overlay::Settings(tab);
        if tab == SettingsTab::Snippets {
            self.backend.refresh_snippets();
        }
        window.focus(&self.focus);
        cx.notify();
    }

    // ───────────── pickers ─────────────

    /// Rows that depend on what is typed: a time ("fri 3pm"), a new label's name, a search
    /// sentence. They come first and are not filtered.
    pub(crate) fn typed_picker_rows(&self, kind: &PickerKind, q: &str) -> Vec<PickerItem> {
        let q = q.trim();
        if q.is_empty() {
            return Vec::new();
        }
        match kind {
            PickerKind::Snooze(_) | PickerKind::Remind(_) | PickerKind::SendLater | PickerKind::Reschedule(_) => {
                match when::parse(q, now_ms(), &self.tz) {
                    Some(at) => vec![PickerItem {
                        value: at.to_string(),
                        label: q.to_owned(),
                        hint: Some(time::short_time(at, &self.tz)),
                    }],
                    None => Vec::new(),
                }
            }
            PickerKind::Label(ids) => {
                let store = self.store();
                let accounts: Vec<String> = store
                    .summaries(ids)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| s.account_id)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let [account] = accounts.as_slice() else { return Vec::new() };
                let taken =
                    store.labels(Some(account)).unwrap_or_default().iter().any(|l| l.name.eq_ignore_ascii_case(q));
                if taken {
                    return Vec::new();
                }
                vec![PickerItem {
                    value: format!("{CREATE_PREFIX}{q}"),
                    label: format!("Create label “{q}”"),
                    hint: None,
                }]
            }
            PickerKind::LabelCreate => {
                let accounts = self.store().accounts().unwrap_or_default();
                let multi = accounts.len() > 1;
                accounts
                    .into_iter()
                    .map(|a| PickerItem {
                        value: format!("{}|{q}", a.id.as_str()),
                        label: format!("Create label “{q}”"),
                        hint: multi.then(|| a.email.clone()),
                    })
                    .collect()
            }
            PickerKind::LabelRenameTo { name, .. } if q != name => {
                vec![PickerItem { value: q.to_owned(), label: format!("Rename “{name}” to “{q}”"), hint: None }]
            }
            PickerKind::AiSearch => vec![PickerItem {
                value: q.to_owned(),
                label: format!("Search with AI: “{q}”"),
                hint: self.backend.ai_egress(AiFeature::NlSearch),
            }],
            _ => Vec::new(),
        }
    }

    /// The fixed rows of this module's pickers.
    pub(crate) fn feature_picker_items(&self, kind: &PickerKind) -> Vec<PickerItem> {
        let presets = || {
            time::presets(now_ms(), &self.tz)
                .into_iter()
                .map(|p| PickerItem { value: p.at.to_string(), label: p.label.to_owned(), hint: Some(p.hint) })
                .collect::<Vec<_>>()
        };
        match kind {
            PickerKind::SendLater | PickerKind::Reschedule(_) => presets(),
            PickerKind::Snippet => {
                let snippets = snippets::ordered(&self.snippets);
                if snippets.is_empty() {
                    return vec![PickerItem {
                        value: cmd::SETTINGS_SNIPPETS.into(),
                        label: "No snippets yet: create one in Settings".into(),
                        hint: None,
                    }];
                }
                snippets
                    .into_iter()
                    .map(|s| PickerItem {
                        value: s.id.as_str().to_owned(),
                        label: s.name.clone(),
                        hint: s.shortcut.as_ref().map(|x| format!(";{x}")),
                    })
                    .collect()
            }
            PickerKind::Rewrite => RewriteStyle::ALL
                .into_iter()
                .map(|s| PickerItem { value: s.as_str().into(), label: s.label().into(), hint: None })
                .chain(self.backend.ai_egress(AiFeature::Rewrite).map(|e| PickerItem {
                    value: "noop".into(),
                    label: e,
                    hint: None,
                }))
                .collect(),
            PickerKind::Scheduled => {
                if self.scheduled.is_empty() {
                    return vec![PickerItem { value: "noop".into(), label: "Nothing is scheduled".into(), hint: None }];
                }
                self.scheduled
                    .iter()
                    .map(|s| {
                        let to = s.preview.to.first().map(|a| a.email.clone()).unwrap_or_default();
                        let subject =
                            if s.preview.subject.trim().is_empty() { "(no subject)" } else { &s.preview.subject };
                        let state = match s.state {
                            SendState::Scheduled if s.kind == SendKind::Later => time::full_time(s.send_at, &self.tz),
                            SendState::Scheduled | SendState::Due | SendState::Sending => "Sending".into(),
                            SendState::Failed => "Failed".into(),
                            SendState::Unknown => "Not sure it was sent".into(),
                            SendState::Sent | SendState::Cancelled => String::new(),
                        };
                        PickerItem {
                            value: s.client_send_id.clone(),
                            label: format!("{subject} · {to}"),
                            hint: Some(state),
                        }
                    })
                    .collect()
            }
            PickerKind::ScheduledAction(id) => {
                let state = self.scheduled.iter().find(|s| &s.client_send_id == id).map(|s| s.state);
                match state {
                    Some(SendState::Unknown) => vec![
                        PickerItem { value: "was_sent".into(), label: "It was sent".into(), hint: None },
                        PickerItem { value: "discard".into(), label: "It was not sent: drop it".into(), hint: None },
                    ],
                    Some(SendState::Scheduled) => vec![
                        PickerItem { value: "change".into(), label: "Change time".into(), hint: None },
                        PickerItem { value: "cancel".into(), label: "Cancel the send".into(), hint: None },
                    ],
                    _ => vec![PickerItem { value: "noop".into(), label: "Nothing to change".into(), hint: None }],
                }
            }
            PickerKind::LabelCreate => {
                vec![PickerItem { value: "noop".into(), label: "Type the new label's name".into(), hint: None }]
            }
            PickerKind::LabelRename | PickerKind::LabelRemove => {
                let store = self.store();
                let accounts = store.accounts().unwrap_or_default();
                let multi = accounts.len() > 1;
                store
                    .labels(None)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(user_label)
                    .map(|l| PickerItem {
                        value: l.id.as_str().to_owned(),
                        hint: multi.then(|| {
                            accounts.iter().find(|a| a.id == l.account_id).map(|a| a.email.clone()).unwrap_or_default()
                        }),
                        label: l.name,
                    })
                    .collect()
            }
            PickerKind::LabelRenameTo { name, .. } => {
                vec![PickerItem {
                    value: "noop".into(), label: format!("Type a new name for “{name}”"), hint: None
                }]
            }
            PickerKind::LabelRemoveConfirm { name, .. } => vec![
                PickerItem {
                    value: "delete".into(),
                    label: format!("Delete “{name}”"),
                    hint: Some("Mail stays; the label goes".into()),
                },
                PickerItem { value: "noop".into(), label: "Keep it".into(), hint: None },
            ],
            PickerKind::ContinueDraft => self
                .backend
                .local_drafts()
                .into_iter()
                .map(|d| PickerItem {
                    hint: Some(time::list_time(d.updated_at, now_ms(), &self.tz)),
                    value: d.client_draft_id.clone(),
                    label: d.title(),
                })
                .collect(),
            PickerKind::AiSearch => vec![PickerItem {
                value: "noop".into(),
                label: "Describe the mail you are looking for".into(),
                hint: None,
            }],
            _ => Vec::new(),
        }
    }

    /// A row of this module's pickers was chosen.
    pub(crate) fn picked_feature(
        &mut self,
        kind: PickerKind,
        value: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if value == "noop" {
            return;
        }
        if value == cmd::SETTINGS_SNIPPETS {
            return self.open_settings(SettingsTab::Snippets, window, cx);
        }
        match kind {
            PickerKind::SendLater => {
                if let Ok(at) = value.parse::<i64>() {
                    self.send_compose_at(Some(at), window, cx);
                }
            }
            PickerKind::Reschedule(id) => {
                if let Ok(at) = value.parse::<i64>() {
                    self.backend.reschedule(id, at);
                }
            }
            PickerKind::Snippet => self.insert_snippet(&value, None, window, cx),
            PickerKind::Rewrite => {
                if let Some(s) = RewriteStyle::from_name(&value) {
                    self.run_rewrite(s, window, cx);
                }
            }
            PickerKind::Scheduled => {
                self.open_palette(Overlay::Picker(PickerKind::ScheduledAction(value)), window, cx);
            }
            PickerKind::ScheduledAction(id) => match value.as_str() {
                "change" => self.open_palette(Overlay::Picker(PickerKind::Reschedule(id)), window, cx),
                "cancel" => self.backend.cancel_scheduled(id),
                "was_sent" => self.backend.resolve_unknown(id, true),
                "discard" => self.backend.resolve_unknown(id, false),
                _ => {}
            },
            PickerKind::LabelCreate => {
                if let Some((account, name)) = value.split_once('|') {
                    self.backend.create_label(account.to_owned(), name.to_owned(), Vec::new());
                }
            }
            PickerKind::LabelRename => {
                let name = self.label_name(&value);
                self.open_palette(Overlay::Picker(PickerKind::LabelRenameTo { label_id: value, name }), window, cx);
            }
            PickerKind::LabelRenameTo { label_id, .. } => self.backend.rename_label(&label_id, value),
            PickerKind::LabelRemove => {
                let name = self.label_name(&value);
                self.open_palette(
                    Overlay::Picker(PickerKind::LabelRemoveConfirm { label_id: value, name }),
                    window,
                    cx,
                );
            }
            PickerKind::LabelRemoveConfirm { label_id, name } => {
                if value == "delete" {
                    self.backend.remove_label(&label_id, &name);
                }
            }
            PickerKind::ContinueDraft => {
                if let Some(d) = self.backend.local_drafts().into_iter().find(|d| d.client_draft_id == value) {
                    self.open_compose(d, window, cx);
                }
            }
            PickerKind::AiSearch => self.run_ai_search(&value, cx),
            _ => {}
        }
    }

    fn label_name(&self, label_id: &str) -> String {
        self.store()
            .labels(None)
            .unwrap_or_default()
            .into_iter()
            .find(|l| l.id.as_str() == label_id)
            .map(|l| l.name)
            .unwrap_or_default()
    }

    /// "Create label" from the label picker: the label is created on the threads' account and
    /// added to them.
    pub(crate) fn create_label_for(&mut self, ids: Vec<String>, name: &str, cx: &mut Context<'_, Self>) {
        let rows = self.store().summaries(&ids).unwrap_or_default();
        let Some(account) = rows.first().map(|r| r.account_id.clone()) else { return };
        if rows.iter().any(|r| r.account_id != account) {
            return self.toast("Labels belong to one account: select conversations from one mailbox.", false, cx);
        }
        self.backend.create_label(account, name.to_owned(), rows.into_iter().map(|r| r.id).collect());
    }

    // ───────────── backend events ─────────────

    /// Events of this module's features.
    pub(crate) fn on_feature_event(&mut self, e: Event, window: &mut Window, cx: &mut Context<'_, Self>) {
        match e {
            Event::Me(me) => {
                // Server prefs win except for changes this device has not synced yet (D-400 rule).
                let local = Synced::of(&self.settings);
                let merged = prefs::merge(&local, &Synced::from_prefs(&me.profile.prefs), &self.settings.pending_prefs);
                if merged != local {
                    let before = local;
                    merged.apply_to(&mut self.settings);
                    self.settings_applied(&before, window, cx);
                    self.save_settings();
                }
                if !self.settings.pending_prefs.is_empty() {
                    let p = prefs::patch(&merged, &self.settings.pending_prefs);
                    self.backend.sync_prefs(merged, p);
                }
                // Dates and the search grammar resolve in the profile's zone (§1.3).
                if let Ok(tz) = jiff::tz::TimeZone::get(&me.profile.timezone) {
                    self.tz = tz;
                    self.tz_name = me.profile.timezone.clone();
                }
                self.me = Some(*me);
            }
            Event::PrefsSynced(sent) => {
                let now = Synced::of(&self.settings);
                let p = &mut self.settings.pending_prefs;
                p.theme &= now.theme != sent.theme;
                p.density &= now.density != sent.density;
                p.keymap &= now.keymap != sent.keymap;
                p.undo_send_seconds &= now.undo_send_seconds != sent.undo_send_seconds;
                p.cache_bodies &= now.cache_bodies != sent.cache_bodies;
                self.save_settings();
            }
            Event::Snippets(list) => self.snippets = list,
            Event::Failures(list) => {
                for f in list.iter().rev() {
                    let op = f.id.as_str().to_owned();
                    let mut actions = Vec::new();
                    if f.payload_kind != "send" {
                        actions.push(("Retry".to_owned(), format!("{}{op}", cmd::RETRY_PREFIX)));
                    }
                    actions.push(("Dismiss".to_owned(), format!("{}{op}", cmd::DISMISS_PREFIX)));
                    self.toast_with(crate::backend::failure_text(f), false, actions, Some(op), cx);
                }
                self.failures = list;
            }
            Event::Scheduled(list) => {
                self.scheduled = list;
                self.palette_sel = 0;
            }
            Event::SearchResults { query, source, hits, unsupported, error } => {
                if self.search_state.query != query {
                    return;
                }
                match source {
                    SearchSource::Server => {
                        self.search_state.server_pending = false;
                        self.search_state.unsupported = unsupported;
                    }
                    SearchSource::Provider => {
                        self.search_state.provider_pending = false;
                        self.search_state.provider_done = true;
                        self.search_state.provider_error = error;
                    }
                }
                let store = self.backend.store.clone();
                let guard = store.lock().unwrap_or_else(|e| e.into_inner());
                let _ = self.vm.add_hits(&query, &hits, &guard);
            }
            Event::DraftConflict { client_draft_id, version } => {
                if let Some(c) = &mut self.compose
                    && c.draft.client_draft_id == client_draft_id
                {
                    c.conflict = Some(version);
                    self.toast_with(
                        "This draft changed on another device",
                        false,
                        vec![("Keep this one".into(), format!("{}{version}", cmd::KEEP_DRAFT_PREFIX))],
                        Some("draft-conflict".into()),
                        cx,
                    );
                }
            }
            Event::Dictation(state) => self.on_dictation(state, window, cx),
            _ => {}
        }
    }

    /// AI events for the composer and AI search; `false` when the request is not theirs.
    pub(crate) fn on_feature_ai(
        &mut self,
        request: u64,
        event: &AaspEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.nl_request == Some(request) {
            match event {
                AaspEvent::Done(d) => {
                    self.nl_request = None;
                    let q = d.object.as_ref().and_then(|o| o.get("query")).and_then(|q| q.as_str()).unwrap_or("");
                    let why = d.object.as_ref().and_then(|o| o.get("explanation")).and_then(|q| q.as_str());
                    if q.trim().is_empty() {
                        self.toast("AI could not turn that into a search", false, cx);
                    } else {
                        // The query is shown for editing; Enter runs it (§11.2 "NL search").
                        self.search.update(cx, |f, cx| f.set_text(q, cx));
                        window.focus(&self.search.focus_handle(cx));
                        self.toast(
                            why.map_or_else(
                                || "Press Enter to search".to_owned(),
                                |w| format!("{w} Press Enter to search"),
                            ),
                            false,
                            cx,
                        );
                    }
                }
                AaspEvent::Error(e) => {
                    self.nl_request = None;
                    self.toast(e.message.clone(), false, cx);
                }
                _ => {}
            }
            return true;
        }
        let Some(c) = &mut self.compose else { return false };
        let Some(edit) = c.ai_edit.clone().filter(|e| e.request == request) else { return false };
        match event {
            AaspEvent::Delta { text } => {
                c.ai_output.push_str(text);
                let body = edit.apply(&c.ai_output);
                c.body.update(cx, |f, cx| f.set_text(&body, cx));
            }
            AaspEvent::Done(d) => {
                let out = d.text.clone().unwrap_or_else(|| c.ai_output.clone());
                let body = edit.apply(&out);
                c.body.update(cx, |f, cx| f.set_text(&body, cx));
                c.ai_undo = Some(edit.before.clone());
                c.ai_edit = None;
                c.ai_output.clear();
                c.ai_bar = false;
                c.instruction.update(cx, |f, cx| f.set_text("", cx));
                c.dirty_at = Some(now_ms());
                window.focus(&c.body.focus_handle(cx));
            }
            AaspEvent::Error(e) => {
                c.body.update(cx, |f, cx| f.set_text(&edit.before, cx));
                c.ai_edit = None;
                c.ai_output.clear();
                c.error = Some(e.message.clone());
            }
            _ => {}
        }
        cx.notify();
        true
    }

    // ───────────── composer: snippets ─────────────

    fn draft_recipients(&self, cx: &gpui::App) -> Vec<akatsuki_core::Addr> {
        self.compose.as_ref().map(|c| parse_recipients(&c.to.read(cx).content).unwrap_or_default()).unwrap_or_default()
    }

    /// Inserts a snippet at the caret (or over `replace`, a typed `;shortcut`).
    pub(crate) fn insert_snippet(
        &mut self,
        snippet_id: &str,
        replace: Option<std::ops::Range<usize>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(s) = self.snippets.iter().find(|s| s.id.as_str() == snippet_id).cloned() else { return };
        let to = self.draft_recipients(cx);
        let Some(c) = &self.compose else { return };
        let subject = c.subject.read(cx).content.clone();
        let e = snippets::expand(&s, &to, &subject, now_ms(), &self.tz);
        let (body, subject_field) = (c.body.clone(), c.subject.clone());
        body.update(cx, |f, cx| match replace {
            Some(r) => f.replace_range(r, &e.text, cx),
            None => f.insert(&e.text, cx),
        });
        if let Some(sub) = &e.subject {
            subject_field.update(cx, |f, cx| f.set_text(sub, cx));
        }
        window.focus(&body.focus_handle(cx));
        self.backend.mark_snippet_used(snippet_id);
        if let Some(n) = snippets::missing_notice(&e.missing) {
            self.toast(n, false, cx);
        }
        cx.notify();
    }

    /// `;shortcut` then Tab in the body expands the snippet (D-295). `true` when it did.
    pub(crate) fn expand_snippet_trigger(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        let Some(c) = &self.compose else { return false };
        if !c.body.focus_handle(cx).is_focused(window) {
            return false;
        }
        let (text, caret) = {
            let f = c.body.read(cx);
            (f.content.clone(), f.caret())
        };
        let Some((start, shortcut)) = snippets::trigger_before(&text, caret) else { return false };
        let Some(id) = snippets::by_shortcut(&self.snippets, &shortcut).map(|s| s.id.as_str().to_owned()) else {
            return false;
        };
        self.insert_snippet(&id, Some(start..caret), window, cx);
        true
    }

    // ───────────── composer: AI write, rewrite ─────────────

    fn open_ai_bar(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(c) = &mut self.compose else { return };
        c.ai_bar = true;
        window.focus(&c.instruction.focus_handle(cx));
        cx.notify();
    }

    fn user_name(&self) -> String {
        self.me
            .as_ref()
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| self.email.split('@').next().unwrap_or("").to_owned())
    }

    /// Write with AI: the instruction row's text runs `compose` over the editable part of the body.
    pub(crate) fn run_ai_write(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(c) = &self.compose else { return };
        let instruction = c.instruction.read(cx).content.clone();
        let body = c.body.read(cx).content.clone();
        let Some(edit) = AiEdit::new(0, AiFeature::Compose, &body, 0..0) else { return };
        let thread = match &c.draft.kind {
            ComposeKind::New => None,
            ComposeKind::Reply { thread_id, .. } | ComposeKind::Forward { thread_id, .. } => Some(thread_id.clone()),
        };
        let draft_text = edit.input().to_owned();
        match self.backend.run_compose(&instruction, &draft_text, thread.as_deref(), &self.user_name(), &self.tz_name) {
            Ok(request) => self.start_ai_edit(AiEdit { request, ..edit }, window, cx),
            Err(e) => self.compose_error(e, cx),
        }
    }

    fn run_rewrite(&mut self, style: RewriteStyle, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(c) = &self.compose else { return };
        let (body, sel) = {
            let f = c.body.read(cx);
            (f.content.clone(), f.selection())
        };
        let Some(edit) = AiEdit::new(0, AiFeature::Rewrite, &body, sel) else {
            return self.compose_error("Write something to rewrite first.".into(), cx);
        };
        match self.backend.run_rewrite(edit.input(), style) {
            Ok(request) => self.start_ai_edit(AiEdit { request, ..edit }, window, cx),
            Err(e) => self.compose_error(e, cx),
        }
    }

    fn start_ai_edit(&mut self, edit: AiEdit, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(c) = &mut self.compose {
            c.ai_edit = Some(edit);
            c.ai_output.clear();
            c.error = None;
            window.focus(&c.body.focus_handle(cx));
        }
        cx.notify();
    }

    /// Escape while AI writes: stop and put the body back.
    pub(crate) fn stop_ai_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(c) = &mut self.compose
            && let Some(edit) = c.ai_edit.take()
        {
            c.body.update(cx, |f, cx| f.set_text(&edit.before, cx));
            c.ai_output.clear();
            window.focus(&c.body.focus_handle(cx));
        }
        cx.notify();
    }

    fn undo_ai(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(c) = &mut self.compose
            && let Some(before) = c.ai_undo.take()
        {
            c.body.update(cx, |f, cx| f.set_text(&before, cx));
            c.dirty_at = Some(now_ms());
            window.focus(&c.body.focus_handle(cx));
        }
        cx.notify();
    }

    fn compose_error(&mut self, e: String, cx: &mut Context<'_, Self>) {
        if let Some(c) = &mut self.compose {
            c.error = Some(e);
        }
        cx.notify();
    }

    // ───────────── composer: dictation ─────────────

    fn toggle_dictation(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.backend.is_recording() {
            self.backend.stop_dictation();
        } else if let Err(e) = self.backend.start_dictation() {
            self.compose_error(e, cx);
        }
        cx.notify();
    }

    fn on_dictation(&mut self, state: DictationState, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(c) = &mut self.compose else { return };
        match state {
            DictationState::Text(t) => {
                c.dictation = None;
                let text = if t.ends_with(' ') { t } else { format!("{t} ") };
                c.body.update(cx, |f, cx| f.insert(&text, cx));
                c.dirty_at = Some(now_ms());
                window.focus(&c.body.focus_handle(cx));
            }
            DictationState::Failed(e) => {
                c.dictation = None;
                c.error = Some(e);
            }
            s => c.dictation = Some(s),
        }
        cx.notify();
    }

    /// "Remind me" on an action item: on its due date at 08:00 (or tomorrow), with its title as
    /// the note the user sees (§12.1).
    pub(crate) fn remind_action(
        &mut self,
        thread_id: &str,
        title: &str,
        due: Option<&str>,
        cx: &mut Context<'_, Self>,
    ) {
        let now = now_ms();
        let at = due.and_then(|d| when::parse(d, now, &self.tz)).or_else(|| when::parse("tomorrow", now, &self.tz));
        match at {
            Some(at) => {
                self.backend.remind_with_note(thread_id.to_owned(), at, title.to_owned());
                self.toast(format!("Reminder for {}", time::full_time(at, &self.tz)), false, cx);
            }
            None => self.toast("That date has passed", false, cx),
        }
    }

    // ───────────── search ─────────────

    fn search_full_mailbox(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let typed = self.search.read(cx).content.trim().to_owned();
        let q = match &self.vm.folder {
            crate::vm::mail::Folder::Search { query } if typed.is_empty() || typed == *query => query.clone(),
            _ if !typed.is_empty() => {
                self.submit_search(window, cx);
                typed
            }
            _ => {
                window.focus(&self.search.focus_handle(cx));
                return self.toast("Type a search first, then search the full mailbox", false, cx);
            }
        };
        if self.search_state.query != q {
            self.search_state = SearchState { query: q.clone(), ..SearchState::default() };
        }
        self.search_state.provider_pending = true;
        self.search_state.provider_error = None;
        self.backend.search_provider(&q, self.vm.account.clone());
        cx.notify();
    }

    fn run_ai_search(&mut self, sentence: &str, cx: &mut Context<'_, Self>) {
        match self.backend.run_nl_search(sentence, &self.tz_name, &self.splits) {
            Ok(r) => {
                self.nl_request = Some(r);
                self.toast("Turning that into a search", false, cx);
            }
            Err(e) => self.toast(e, false, cx),
        }
    }

    // ───────────── smoke scripts (AKATSUKI_SCRIPT) ─────────────

    /// `field=text` steps of a start-up script: fill a composer field, type into the palette or a
    /// picker, or choose a picker row by value.
    pub(crate) fn script_type(&mut self, field: &str, text: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let target = match (field, &self.compose) {
            ("to", Some(c)) => Some(c.to.clone()),
            ("subject", Some(c)) => Some(c.subject.clone()),
            ("body", Some(c)) => Some(c.body.clone()),
            ("instruction", Some(c)) => Some(c.instruction.clone()),
            ("query", _) => Some(self.palette_query.clone()),
            _ => None,
        };
        if let Some(f) = target {
            f.update(cx, |f, cx| f.set_text(text, cx));
            if let Some(c) = &mut self.compose {
                c.dirty_at = Some(now_ms());
            }
            self.palette_sel = 0;
        } else if field == "ai" {
            // A local OpenAI-compatible endpoint with consent, for smoke runs of the AI surfaces.
            let profile = akatsuki_core::convex::rows::AiProfile {
                profile_id: "smoke-local".into(),
                label: "Local".into(),
                kind: akatsuki_core::convex::AiProviderKind::OpenaiCompatible,
                base_url: Some(text.to_owned()),
                models: akatsuki_core::convex::rows::AiModels {
                    default: Some("local-model".into()),
                    fast: Some("local-model".into()),
                    transcription: Some("whisper-1".into()),
                    ..Default::default()
                },
                key_on_devices: vec![],
            };
            self.backend.save_ai_profile(profile, None, true);
        } else if field == "pick" {
            let rows = self.palette_rows(cx);
            if let Some(i) = rows.iter().position(|r| r.id == text) {
                self.palette_sel = i;
                self.run(sc::PALETTE_RUN, window, cx);
            }
        }
        cx.notify();
    }

    // ───────────── settings sync ─────────────

    /// Changes device settings; synced fields go to `me.updatePrefs` (only what changed).
    pub(crate) fn change_settings(
        &mut self,
        f: impl FnOnce(&mut LocalSettings),
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let before = Synced::of(&self.settings);
        f(&mut self.settings);
        let after = Synced::of(&self.settings);
        self.settings.pending_prefs.mark(&before, &after);
        self.settings_applied(&before, window, cx);
        self.save_settings();
        let patch = prefs::patch(&after, &self.settings.pending_prefs);
        self.backend.sync_prefs(after, patch);
        cx.notify();
    }

    /// Applies the effects of synced settings that changed from `before`.
    fn settings_applied(&mut self, before: &Synced, window: &mut Window, cx: &mut Context<'_, Self>) {
        let now = Synced::of(&self.settings);
        if now.keymap != before.keymap {
            self.matcher = Matcher::new(Platform::current(), self.settings.keymap.keymap());
        }
        if now.cache_bodies != before.cache_bodies {
            self.backend.set_cache_bodies(now.cache_bodies);
        }
        if now.theme != before.theme {
            self.apply_theme(window, cx);
        }
        cx.notify();
    }
}
