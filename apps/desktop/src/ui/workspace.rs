//! The root view: owns the view models and the overlays, routes every key through the shared key
//! map (`crate::keys`), runs commands, and applies backend events.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, Focusable, KeyDownEvent, ScrollHandle, Subscription,
    UniformListScrollHandle, Window, div, prelude::*, px,
};
use jiff::tz::TimeZone;

use akatsuki_core::convex::rows::{AiSettings, LabelView, MessageMeta};
use akatsuki_core::convex::{AiFeature, ThreadAction};
use akatsuki_core::time::now_ms;
use akatsuki_shortcuts::{Platform, Scope, ids as sc};

use crate::backend::{Backend, Event, SessionState};
use crate::commands::{self, PaletteContext, PaletteItem, ids as cmd};
use crate::config::{Density, KeymapPref, LocalSettings, MotionPref, Paths};
use crate::keys::{self, MatchResult, Matcher};
use crate::render::message::Rendered;
use crate::theme::{Theme, ThemePref};
use crate::vm::actions::{self, AccountCtx};
use crate::vm::compose::{Attachment, AttachmentState, Draft, PendingSend};
use crate::vm::mail::{ActionKind, Folder, Intent, MailVm, plan};
use crate::vm::time;

use super::ThemeGlobal;
use super::input::{FieldEvent, TextField};

/// What covers the mail view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// Nothing.
    None,
    /// The command palette.
    Palette,
    /// A list to choose from.
    Picker(PickerKind),
    /// Keyboard shortcuts.
    Help,
    /// Settings.
    Settings(SettingsTab),
}

/// Settings sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    /// Theme, density, keys.
    General,
    /// AI providers.
    Ai,
    /// Labels: create, rename, delete.
    Labels,
    /// Snippets.
    Snippets,
    /// Account and device.
    Account,
}

/// What a picker chooses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerKind {
    /// Snooze until.
    Snooze(Vec<String>),
    /// Remind at.
    Remind(String),
    /// Add a label.
    Label(Vec<String>),
    /// Move to a folder.
    Move(Vec<String>),
    /// Go to a label.
    GoLabel,
    /// Confirm a mailto unsubscribe: the address it goes to, and whether that is not at the sender's
    /// domain (an email leaves the user's mailbox, so it is never sent unseen).
    Unsubscribe {
        /// Thread.
        thread_id: String,
        /// Destination address.
        to: String,
        /// Not at the sender's domain.
        foreign: bool,
    },
    /// Send the open draft at a time.
    SendLater,
    /// Insert a snippet.
    Snippet,
    /// Rewrite the selection or the body.
    Rewrite,
    /// Scheduled and unresolved sends.
    Scheduled,
    /// What to do with one scheduled send.
    ScheduledAction(String),
    /// A new time for a scheduled send.
    Reschedule(String),
    /// Create a label (typed name; one row per account).
    LabelCreate,
    /// Choose a label to rename.
    LabelRename,
    /// The new name of a label.
    LabelRenameTo {
        /// Label id.
        label_id: String,
        /// Current name.
        name: String,
    },
    /// Choose a label to delete.
    LabelRemove,
    /// Confirm a delete.
    LabelRemoveConfirm {
        /// Label id.
        label_id: String,
        /// Name.
        name: String,
    },
    /// Reopen a draft kept on this device.
    ContinueDraft,
    /// Describe a search in words.
    AiSearch,
}

/// One picker row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerItem {
    /// What it does when chosen (label key or ms instant).
    pub value: String,
    /// Text.
    pub label: String,
    /// Right-aligned hint.
    pub hint: Option<String>,
}

/// A notice at the bottom of the window.
#[derive(Clone, Debug)]
pub struct Toast {
    /// Id.
    pub id: u64,
    /// Text.
    pub text: String,
    /// Offers Undo (`z`).
    pub undo: bool,
    /// Created.
    pub at: i64,
    /// Buttons: `(label, command id)` (Retry, Dismiss, Keep this one).
    pub actions: Vec<(String, String)>,
    /// Stable key (an op id) so one failure is shown once.
    pub key: Option<String>,
}

/// The open thread.
pub struct ThreadView {
    /// Thread.
    pub id: String,
    /// Messages, oldest first.
    pub messages: Vec<MessageMeta>,
    /// Expanded messages.
    pub expanded: HashSet<String>,
    /// Messages with quoted history shown.
    pub show_quoted: HashSet<String>,
    /// Bodies.
    pub bodies: HashMap<String, Result<Arc<Rendered>, String>>,
}

/// Search beyond the local index for the list on screen (§11.2).
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    /// The query.
    pub query: String,
    /// Convex is still answering.
    pub server_pending: bool,
    /// Grammar Convex could not apply.
    pub unsupported: Vec<String>,
    /// The provider search runs.
    pub provider_pending: bool,
    /// The provider search ran.
    pub provider_done: bool,
    /// Its error.
    pub provider_error: Option<String>,
}

/// The AI panel over the reading pane.
pub struct AiPanel {
    /// Request id.
    pub request: u64,
    /// Feature.
    pub feature: AiFeature,
    /// Thread.
    pub thread_id: String,
    /// Streamed text or the latest object.
    pub text: String,
    /// The object (summary or replies).
    pub object: Option<serde_json::Value>,
    /// Error.
    pub error: Option<String>,
    /// Finished.
    pub done: bool,
    /// Egress line.
    pub egress: Option<String>,
    /// The model that answered (from the stream's `meta`).
    pub model: Option<String>,
}

/// An open draft.
pub struct ComposeState {
    /// The draft.
    pub draft: Draft,
    /// Fields.
    pub to: Entity<TextField>,
    /// Cc.
    pub cc: Entity<TextField>,
    /// Bcc.
    pub bcc: Entity<TextField>,
    /// Subject.
    pub subject: Entity<TextField>,
    /// Body.
    pub body: Entity<TextField>,
    /// Cc and Bcc rows shown.
    pub show_cc: bool,
    /// Validation error.
    pub error: Option<String>,
    /// Last edit, for the 2 s autosave debounce.
    pub dirty_at: Option<i64>,
    /// The "write with AI" instruction field.
    pub instruction: Entity<TextField>,
    /// The instruction row is open.
    pub ai_bar: bool,
    /// The AI request changing the body, if one runs.
    pub ai_edit: Option<crate::vm::ai::AiEdit>,
    /// Streamed text so far.
    pub ai_output: String,
    /// The body before the last AI change (Undo).
    pub ai_undo: Option<String>,
    /// Dictation state, while it is not idle.
    pub dictation: Option<crate::backend::DictationState>,
    /// The worker holds a newer copy (version); saving waits for "Keep this one".
    pub conflict: Option<u64>,
    _subs: Vec<Subscription>,
}

/// The sign-in screen's fields.
pub struct SignIn {
    /// Email.
    pub email: Entity<TextField>,
    /// Code.
    pub code: Entity<TextField>,
    /// A code was sent.
    pub code_sent: bool,
    /// Progress or error.
    pub status: Option<String>,
}

/// The AI provider form in settings.
pub struct AiForm {
    /// Kind index into [`crate::ui::settings::KINDS`].
    pub kind: usize,
    /// Label.
    pub label: Entity<TextField>,
    /// Base URL.
    pub base_url: Entity<TextField>,
    /// Model.
    pub model: Entity<TextField>,
    /// Key (masked).
    pub key: Entity<TextField>,
    /// Consent checkbox.
    pub consent: bool,
    /// Test result.
    pub status: Option<String>,
}

/// The root view.
pub struct Workspace {
    pub(crate) backend: Arc<Backend>,
    pub(crate) settings: LocalSettings,
    pub(crate) paths: Paths,
    pub(crate) session: SessionState,
    pub(crate) signin: SignIn,
    pub(crate) vm: MailVm,
    pub(crate) focus: FocusHandle,
    pub(crate) matcher: Matcher,
    pub(crate) list_scroll: UniformListScrollHandle,
    pub(crate) reading_scroll: ScrollHandle,
    pub(crate) thread: Option<ThreadView>,
    pub(crate) overlay: Overlay,
    pub(crate) palette_query: Entity<TextField>,
    pub(crate) palette_sel: usize,
    pub(crate) palette_scroll: ScrollHandle,
    pub(crate) search: Entity<TextField>,
    pub(crate) search_error: Option<String>,
    pub(crate) compose: Option<ComposeState>,
    pub(crate) pending_send: Option<PendingSend>,
    pub(crate) toasts: Vec<Toast>,
    pub(crate) toast_seq: u64,
    pub(crate) ai: Option<AiPanel>,
    pub(crate) ai_settings: Option<AiSettings>,
    pub(crate) ai_form: AiForm,
    pub(crate) unread: BTreeMap<String, usize>,
    pub(crate) tz: TimeZone,
    pub(crate) tz_name: String,
    pub(crate) reduce_motion: bool,
    pub(crate) system_reduces_motion: bool,
    pub(crate) theme: Theme,
    pub(crate) email: String,
    pub(crate) me: Option<akatsuki_core::convex::client::MeView>,
    pub(crate) snippets: Vec<akatsuki_core::convex::client::SnippetView>,
    pub(crate) failures: Vec<akatsuki_core::convex::rows::OpFailure>,
    pub(crate) scheduled: Vec<akatsuki_core::convex::rows::ScheduledSendView>,
    pub(crate) splits: Vec<akatsuki_core::convex::client::SplitView>,
    pub(crate) search_state: SearchState,
    pub(crate) nl_request: Option<u64>,
    pub(crate) forms: super::settings_extra::Forms,
    pub(crate) hud: Vec<(String, i64)>,
    _subs: Vec<Subscription>,
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn field(cx: &mut Context<'_, Workspace>, placeholder: &str) -> Entity<TextField> {
    cx.new(|cx| TextField::new(placeholder, cx))
}

impl Workspace {
    /// Builds the workspace and starts draining backend events.
    pub fn new(
        backend: Arc<Backend>,
        events: UnboundedReceiver<Event>,
        settings: LocalSettings,
        paths: Paths,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let palette_query = field(cx, "Type a command or search…");
        let search = field(cx, "Search mail   from: to: subject: has:attachment is:unread");
        let email = field(cx, "you@example.com");
        let code = cx.new(|cx| TextField::new("6-digit code", cx).masked());
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&palette_query, window, |this, _, e: &FieldEvent, window, cx| match e {
            FieldEvent::Changed => {
                this.palette_sel = 0;
                cx.notify();
            }
            FieldEvent::Submit => this.run(sc::PALETTE_RUN, window, cx),
            FieldEvent::Cancel => this.run(sc::PALETTE_CLOSE, window, cx),
            FieldEvent::SubmitBack => {}
        }));
        subs.push(cx.subscribe_in(&search, window, |this, _, e: &FieldEvent, window, cx| match e {
            FieldEvent::Submit => this.run(sc::SEARCH_SUBMIT, window, cx),
            FieldEvent::Cancel => this.run(sc::SEARCH_CLOSE, window, cx),
            _ => {}
        }));
        subs.push(cx.subscribe_in(&email, window, |this, _, e: &FieldEvent, window, cx| {
            if *e == FieldEvent::Submit {
                this.signin_primary(window, cx);
            }
        }));
        subs.push(cx.subscribe_in(&code, window, |this, _, e: &FieldEvent, window, cx| {
            if *e == FieldEvent::Submit {
                this.signin_primary(window, cx);
            }
        }));
        let ai_form = AiForm {
            kind: 0,
            label: field(cx, "Label, e.g. My OpenRouter"),
            base_url: field(cx, "Base URL (optional)"),
            model: field(cx, "Model, e.g. anthropic/claude-sonnet-5.5"),
            key: cx.new(|cx| TextField::new("API key (stored in the system keychain)", cx).masked()),
            consent: false,
            status: None,
        };
        subs.push(cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_theme(window, cx);
        }));
        let tz = time::local_tz();
        let tz_name = time::tz_name(&tz);
        let system_reduces_motion = crate::config::system_reduces_motion();
        let mut this = Self {
            backend,
            matcher: Matcher::new(Platform::current(), settings.keymap.keymap()),
            reduce_motion: false,
            system_reduces_motion,
            settings,
            paths,
            session: SessionState::Connecting,
            signin: SignIn { email, code, code_sent: false, status: None },
            vm: MailVm::default(),
            focus: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            reading_scroll: ScrollHandle::new(),
            thread: None,
            overlay: Overlay::None,
            palette_query,
            palette_sel: 0,
            palette_scroll: ScrollHandle::new(),
            search,
            search_error: None,
            compose: None,
            pending_send: None,
            toasts: Vec::new(),
            toast_seq: 0,
            ai: None,
            ai_settings: None,
            ai_form,
            unread: BTreeMap::new(),
            tz,
            tz_name,
            theme: Theme::new(akatsuki_tokens::Appearance::Light),
            email: String::new(),
            me: None,
            snippets: Vec::new(),
            failures: Vec::new(),
            scheduled: Vec::new(),
            splits: Vec::new(),
            search_state: SearchState::default(),
            nl_request: None,
            forms: super::settings_extra::Forms::new(cx),
            hud: Vec::new(),
            _subs: subs,
        };
        this.apply_motion();
        this.apply_theme(window, cx);
        // Backend events.
        cx.spawn_in(window, async move |this, cx| {
            let mut events = events;
            while let Some(e) = events.next().await {
                if this.update_in(cx, |this, window, cx| this.on_event(e, window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        // A slow ticker: toast expiry, the undo-send countdown and the compose autosave debounce.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                if this.update_in(cx, |this, _, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        window.focus(&this.focus);
        this
    }

    pub(crate) fn store(&self) -> std::sync::MutexGuard<'_, akatsuki_core::store::Store> {
        self.backend.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn apply_motion(&mut self) {
        self.reduce_motion = match self.settings.motion {
            MotionPref::System => self.system_reduces_motion,
            MotionPref::Reduce => true,
            MotionPref::Full => false,
        };
    }

    pub(crate) fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.theme = Theme::new(self.settings.theme.resolve(window.appearance()));
        cx.set_global(ThemeGlobal(self.theme));
        cx.notify();
    }

    pub(crate) fn save_settings(&self) {
        let _ = self.settings.save(&self.paths.config_file);
    }

    fn tick(&mut self, cx: &mut Context<'_, Self>) {
        let now = now_ms();
        let before = self.toasts.len();
        self.toasts.retain(|t| {
            now - t.at < if t.undo || !t.actions.is_empty() { crate::backend::UNDO_WINDOW_MS } else { 4000 }
        });
        let huds = self.hud.len();
        self.hud.retain(|(_, at)| now - at < super::status::HUD_MS);
        let mut changed = before != self.toasts.len() || self.pending_send.is_some() || huds != self.hud.len();
        // The status bar echoes the last action for 4 s.
        changed |= self.toasts.last().is_some_and(|t| (4000..4600).contains(&(now - t.at)));
        if let Some(p) = &self.pending_send
            && p.undo_until + 1500 < now
        {
            self.pending_send = None;
            changed = true;
        }
        if let Some(c) = &mut self.compose
            && c.dirty_at.is_some_and(|d| now - d >= crate::vm::compose::AUTOSAVE_DEBOUNCE_MS as i64)
        {
            c.dirty_at = None;
            let draft = Self::read_draft(c, cx);
            self.backend.save_local_draft(&draft);
            if c.conflict.is_none() {
                self.backend.autosave(draft);
            }
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn toast(&mut self, text: impl Into<String>, undo: bool, cx: &mut Context<'_, Self>) {
        self.toast_with(text, undo, Vec::new(), None, cx);
    }

    /// A toast with buttons; a `key` already shown is not shown again.
    pub(crate) fn toast_with(
        &mut self,
        text: impl Into<String>,
        undo: bool,
        actions: Vec<(String, String)>,
        key: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        if key.as_ref().is_some_and(|k| self.toasts.iter().any(|t| t.key.as_ref() == Some(k))) {
            return;
        }
        self.toast_seq += 1;
        self.toasts.push(Toast { id: self.toast_seq, text: sentence(text.into()), undo, at: now_ms(), actions, key });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        cx.notify();
    }

    // ───────────── backend events ─────────────

    fn on_event(&mut self, e: Event, window: &mut Window, cx: &mut Context<'_, Self>) {
        match e {
            Event::Session(s) => {
                let signed_in = matches!(s, SessionState::SignedIn { .. });
                if let SessionState::SignedIn { email, .. } = &s {
                    self.email = email.clone();
                }
                self.session = s;
                if signed_in {
                    self.signin.status = None;
                    self.refresh(cx);
                    window.focus(&self.focus);
                } else {
                    self.vm = MailVm::default();
                    self.thread = None;
                    self.compose = None;
                    self.overlay = Overlay::None;
                }
            }
            Event::SignInStatus(s) => self.signin.status = Some(s),
            Event::OpenUrl(url) => cx.open_url(&url),
            Event::MirrorChanged | Event::AccountsChanged => self.refresh(cx),
            Event::Splits(splits) => {
                self.vm.set_splits(&splits);
                self.splits = splits;
                self.refresh(cx);
            }
            Event::AiSettings(s) => self.ai_settings = s,
            Event::Body { message_id, result } => {
                if let Some(t) = &mut self.thread
                    && t.messages.iter().any(|m| m.id.as_str() == message_id)
                {
                    t.bodies.insert(message_id, result);
                }
            }
            Event::Toast { text, undo } => self.toast(text, undo, cx),
            Event::AttachmentDone { client_draft_id, key, result } => {
                if let Some(c) = &mut self.compose
                    && c.draft.client_draft_id == client_draft_id
                    && let Some(a) = c.draft.attachments.iter_mut().find(|a| a.key == key)
                {
                    a.state = match result {
                        Ok(upload_id) => AttachmentState::Ready { upload_id },
                        Err(reason) => AttachmentState::Failed { reason },
                    };
                    // The provider draft gets the file with the next autosave.
                    c.dirty_at = Some(now_ms());
                }
            }
            Event::DraftSaved { client_draft_id, version } => {
                if let Some(c) = &mut self.compose
                    && c.draft.client_draft_id == client_draft_id
                {
                    c.draft.base_version = version;
                    let draft = Self::read_draft(c, cx);
                    self.backend.save_local_draft(&draft);
                }
            }
            e @ (Event::Me(_)
            | Event::PrefsSynced(_)
            | Event::Snippets(_)
            | Event::Failures(_)
            | Event::Scheduled(_)
            | Event::SearchResults { .. }
            | Event::DraftConflict { .. }
            | Event::Dictation(_)) => self.on_feature_event(e, window, cx),
            Event::SendDone { client_send_id, reopen, error } => {
                if self.pending_send.as_ref().is_some_and(|p| p.client_send_id == client_send_id) {
                    self.pending_send = None;
                }
                match (reopen, error) {
                    (Some(draft), None) => {
                        self.open_compose(draft, window, cx);
                        self.toast("Send undone. The draft is open again.", false, cx);
                    }
                    (reopen, Some(e)) => {
                        self.toast(e, false, cx);
                        if let Some(d) = reopen {
                            self.open_compose(d, window, cx);
                        }
                    }
                    (None, None) => self.toast("Sent", false, cx),
                }
            }
            Event::Ai { request, event } => {
                if !self.on_feature_ai(request, &event, window, cx) {
                    self.on_ai(request, event);
                }
            }
            Event::AiTest { profile_id, result } => {
                let name = self
                    .ai_settings
                    .as_ref()
                    .and_then(|s| s.profiles.iter().find(|p| p.profile_id == profile_id))
                    .map(|p| p.label.clone())
                    .unwrap_or_default();
                self.ai_form.status = Some(match result {
                    Ok(()) => format!("{name}: the connection works."),
                    Err(e) => format!("{name}: {e}"),
                });
            }
        }
        cx.notify();
    }

    fn on_ai(&mut self, request: u64, event: akatsuki_ai::AaspEvent) {
        let Some(p) = &mut self.ai else { return };
        if p.request != request {
            return;
        }
        match event {
            akatsuki_ai::AaspEvent::Meta(m) => p.model = Some(m.model),
            akatsuki_ai::AaspEvent::Delta { text } => p.text.push_str(&text),
            akatsuki_ai::AaspEvent::Partial { object } => p.object = Some(object),
            akatsuki_ai::AaspEvent::Done(d) => {
                if let Some(o) = d.object {
                    p.object = Some(o);
                }
                if let Some(t) = d.text {
                    p.text = t;
                }
                p.done = true;
            }
            akatsuki_ai::AaspEvent::Error(e) => {
                p.error = Some(e.message);
                p.done = true;
            }
        }
    }

    /// Re-reads lists and the open thread from the mirror.
    pub(crate) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        let before_open = self.vm.open.clone();
        {
            let store = self.backend.store.lock().unwrap_or_else(|e| e.into_inner());
            let _ = self.vm.refresh(&store);
            let mut keys: Vec<String> =
                ["sys:inbox", "sys:drafts", "sys:spam"].iter().map(|s| (*s).to_owned()).collect();
            keys.extend(self.vm.tabs.iter().map(|t| t.key.clone()));
            keys.extend(store.labels(None).unwrap_or_default().into_iter().map(|l| l.label_key));
            self.unread = store.unread_counts(&keys, self.vm.account.as_deref(), 100).unwrap_or_default();
        }
        if self.vm.open != before_open {
            match self.vm.open.clone() {
                Some(id) => self.open_thread(&id, cx),
                None => self.thread = None,
            }
        } else if let Some(id) = self.vm.open.clone() {
            let messages = self.store().messages(&id).unwrap_or_default();
            if let Some(t) = &mut self.thread
                && t.messages.len() != messages.len()
            {
                let newest = messages.last().map(|m| m.id.as_str().to_owned());
                t.messages = messages;
                if let Some(n) = newest {
                    t.expanded.insert(n.clone());
                    self.backend.load_body(&n);
                }
            }
        }
        cx.notify();
    }

    fn open_thread(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let messages = self.store().messages(id).unwrap_or_default();
        let mut expanded = HashSet::new();
        // Unread messages and the newest one start expanded.
        for m in &messages {
            if m.is_unread {
                expanded.insert(m.id.as_str().to_owned());
            }
        }
        if let Some(last) = messages.last() {
            expanded.insert(last.id.as_str().to_owned());
        }
        let mut bodies = HashMap::new();
        for m in &messages {
            if let Some(b) = self.backend.cached_body(m.id.as_str()) {
                bodies.insert(m.id.as_str().to_owned(), Ok(b));
            } else if expanded.contains(m.id.as_str()) {
                self.backend.load_body(m.id.as_str());
            }
        }
        self.thread = Some(ThreadView { id: id.to_owned(), messages, expanded, show_quoted: HashSet::new(), bodies });
        self.reading_scroll = ScrollHandle::new();
        if self.ai.as_ref().is_some_and(|a| a.thread_id != id) {
            self.ai = None;
        }
        // Opening an unread thread marks it read.
        let plans = plan(ActionKind::MarkRead, &[id.to_owned()], &Folder::Inbox, &self.store()).unwrap_or_default();
        if !plans.is_empty() {
            self.backend.apply(plans, None);
        }
        cx.notify();
    }

    // ───────────── keys ─────────────

    /// List and reader side by side (§5.1: from 1100 px).
    pub(crate) fn three_panes(&self, window: &Window) -> bool {
        f32::from(window.viewport_size().width) >= 1100.
    }

    pub(crate) fn signed_in(&self) -> bool {
        matches!(self.session, SessionState::SignedIn { .. })
    }

    pub(crate) fn compose_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.compose_focused(window, cx)
    }

    fn compose_focused(&self, window: &Window, cx: &App) -> bool {
        self.compose.as_ref().is_some_and(|c| {
            [&c.to, &c.cc, &c.bcc, &c.subject, &c.body].iter().any(|f| f.focus_handle(cx).contains_focused(window, cx))
        })
    }

    /// The scopes the matcher sees (`packages/shortcuts` scopes).
    pub(crate) fn scopes(&self, window: &Window, cx: &App) -> HashSet<Scope> {
        let mut s = HashSet::from([Scope::Global]);
        match &self.overlay {
            Overlay::Palette | Overlay::Picker(_) => {
                s.insert(Scope::Palette);
                return s;
            }
            Overlay::Help | Overlay::Settings(_) => return s,
            Overlay::None => {}
        }
        if self.compose_focused(window, cx) {
            s.insert(Scope::Compose);
            return s;
        }
        if self.search.focus_handle(cx).is_focused(window) {
            s.insert(Scope::Search);
            return s;
        }
        if !self.signed_in() {
            // Sign-in fields are text fields.
            if self.signin.email.focus_handle(cx).is_focused(window)
                || self.signin.code.focus_handle(cx).is_focused(window)
            {
                s.insert(Scope::Search);
            }
            return s;
        }
        s.insert(Scope::Mail);
        s.insert(if self.vm.open.is_some() { Scope::Thread } else { Scope::List });
        s
    }

    fn on_key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let ks = &e.keystroke;
        if ks.key == "tab" && !ks.modifiers.modified() && self.expand_snippet_trigger(window, cx) {
            cx.stop_propagation();
            return;
        }
        if ks.key == "escape" && matches!(self.overlay, Overlay::Help | Overlay::Settings(_)) {
            self.close_overlay(window, cx);
            cx.stop_propagation();
            return;
        }
        let combo = keys::combo_from_keystroke(
            &ks.key,
            ks.modifiers.control,
            ks.modifiers.alt,
            ks.modifiers.shift,
            ks.modifiers.platform,
        );
        let scopes = self.scopes(window, cx);
        let shown = hud_label(&combo);
        match self.matcher.handle(combo, &scopes, now_ms()) {
            MatchResult::Match(id) => {
                cx.stop_propagation();
                if !scopes.iter().any(|s| keys::TEXT_SCOPES.contains(s)) {
                    self.hud_push(shown);
                }
                self.run(id, window, cx);
            }
            MatchResult::Pending => {
                self.hud_push(shown);
                cx.stop_propagation();
            }
            MatchResult::None => {}
        }
    }

    // ───────────── commands ─────────────

    pub(crate) fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.overlay = Overlay::None;
        if let Some(c) = &self.compose {
            window.focus(&c.body.focus_handle(cx));
        } else {
            window.focus(&self.focus);
        }
        cx.notify();
    }

    pub(crate) fn palette_context(&self) -> PaletteContext {
        let store = self.backend.store.lock().unwrap_or_else(|e| e.into_inner());
        PaletteContext {
            keymap: self.settings.keymap,
            theme: self.settings.theme,
            density: self.settings.density,
            labels: store
                .labels(None)
                .unwrap_or_default()
                .into_iter()
                .filter(|l| !l.hidden)
                .map(|l| (l.label_key, l.name))
                .collect(),
            splits: self.vm.tabs.iter().map(|t| (t.key.clone(), t.name.clone())).collect(),
            accounts: store.accounts().unwrap_or_default().into_iter().map(|a| (a.id.into_string(), a.email)).collect(),
            signed_in: self.signed_in(),
            thread_open: self.vm.open.is_some(),
            composing: self.compose.is_some(),
            failures: !self.failures.is_empty(),
            local_drafts: store.local_drafts().is_ok_and(|d| !d.is_empty()),
            ai_undo: self.compose.as_ref().is_some_and(|c| c.ai_undo.is_some()),
        }
    }

    /// The palette's or picker's rows for the current query.
    pub(crate) fn palette_rows(&self, cx: &App) -> Vec<PaletteItem> {
        let q = self.palette_query.read(cx).content.clone();
        match &self.overlay {
            Overlay::Picker(kind) => {
                let typed: Vec<PaletteItem> = self
                    .typed_picker_rows(kind, &q)
                    .into_iter()
                    .map(|p| PaletteItem {
                        id: p.value,
                        label: p.label,
                        group: String::new(),
                        hint: p.hint,
                        checked: false,
                        keywords: Vec::new(),
                    })
                    .collect();
                let items: Vec<PaletteItem> = self
                    .picker_items(kind)
                    .into_iter()
                    .map(|p| PaletteItem {
                        id: p.value,
                        label: p.label,
                        group: String::new(),
                        hint: p.hint,
                        checked: false,
                        keywords: Vec::new(),
                    })
                    .collect();
                let mut rows = typed;
                rows.extend(commands::filter(items, &q));
                rows
            }
            _ => commands::filter(commands::palette_items(&self.palette_context()), &q),
        }
    }

    pub(crate) fn picker_items(&self, kind: &PickerKind) -> Vec<PickerItem> {
        let labels = |pred: &dyn Fn(&LabelView) -> bool| -> Vec<PickerItem> {
            let store = self.backend.store.lock().unwrap_or_else(|e| e.into_inner());
            let accounts = store.accounts().unwrap_or_default();
            let multi = accounts.len() > 1;
            store
                .labels(None)
                .unwrap_or_default()
                .into_iter()
                .filter(|l| !l.hidden && pred(l))
                .map(|l| PickerItem {
                    hint: multi.then(|| {
                        accounts.iter().find(|a| a.id == l.account_id).map(|a| a.email.clone()).unwrap_or_default()
                    }),
                    value: l.label_key,
                    label: l.name,
                })
                .collect()
        };
        match kind {
            PickerKind::Snooze(_) | PickerKind::Remind(_) => time::presets(now_ms(), &self.tz)
                .into_iter()
                .map(|p| PickerItem { value: p.at.to_string(), label: p.label.to_owned(), hint: Some(p.hint) })
                .collect(),
            PickerKind::Label(_) => labels(&|l| !l.exclusive),
            PickerKind::Move(_) => labels(&|l| l.role.is_none() || l.exclusive),
            PickerKind::GoLabel => labels(&|_| true),
            PickerKind::Unsubscribe { to, foreign, .. } => vec![
                PickerItem {
                    value: "send".into(),
                    label: format!("Email {to} to ask to be removed"),
                    hint: foreign.then(|| "Not the sender's domain".to_owned()),
                },
                PickerItem { value: "cancel".into(), label: "Cancel".into(), hint: None },
            ],
            other => self.feature_picker_items(other),
        }
    }

    pub(crate) fn open_palette(&mut self, overlay: Overlay, window: &mut Window, cx: &mut Context<'_, Self>) {
        let placeholder = match &overlay {
            Overlay::Picker(
                PickerKind::Snooze(_) | PickerKind::Remind(_) | PickerKind::SendLater | PickerKind::Reschedule(_),
            ) => "tomorrow 9am, 3d, fri 2pm",
            Overlay::Picker(PickerKind::Label(_)) => "Label, or a new name",
            Overlay::Picker(PickerKind::LabelCreate | PickerKind::LabelRenameTo { .. }) => "Label name",
            Overlay::Picker(PickerKind::AiSearch) => "mail from Mira about the deck last week",
            Overlay::Picker(PickerKind::Snippet) => "Snippet",
            Overlay::Picker(_) => "Filter",
            _ => "Type a command or search mail",
        };
        self.overlay = overlay;
        self.palette_sel = 0;
        self.palette_query.update(cx, |f, cx| {
            f.set_text("", cx);
            f.set_placeholder(placeholder, cx);
        });
        window.focus(&self.palette_query.focus_handle(cx));
        self.matcher.reset();
        cx.notify();
    }

    fn choose(&mut self, value: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        self.close_overlay(window, cx);
        match overlay {
            Overlay::Palette => self.run(&value, window, cx),
            Overlay::Picker(kind) => self.picked(kind, value, window, cx),
            _ => {}
        }
    }

    fn picked(&mut self, kind: PickerKind, value: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        match kind {
            PickerKind::Snooze(ids) => {
                if let Ok(at) = value.parse::<i64>() {
                    self.backend.snooze(ids, at);
                }
            }
            PickerKind::Remind(id) => {
                if let Ok(at) = value.parse::<i64>() {
                    self.backend.set_reminder(id, at);
                }
            }
            PickerKind::Unsubscribe { thread_id, .. } => {
                if value == "send" {
                    self.unsubscribe_now(thread_id, cx);
                }
            }
            PickerKind::GoLabel => {
                let name = self
                    .store()
                    .labels(None)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|l| l.label_key == value)
                    .map(|l| l.name);
                let folder = Folder::Label { key: value, name: name.unwrap_or_default() };
                let _ = self.vm.go(folder, &self.backend.store.lock().unwrap_or_else(|e| e.into_inner()));
                self.thread = None;
            }
            PickerKind::Label(ref ids) if value.starts_with(super::features::CREATE_PREFIX) => {
                self.create_label_for(ids.clone(), &value[super::features::CREATE_PREFIX.len()..], cx);
            }
            PickerKind::Label(ref ids) | PickerKind::Move(ref ids) => {
                let ids = ids.clone();
                let store = self.backend.store.lock().unwrap_or_else(|e| e.into_inner());
                let Some(label) = store.labels(None).unwrap_or_default().into_iter().find(|l| l.label_key == value)
                else {
                    return;
                };
                let accounts = store.accounts().unwrap_or_default();
                let labels = store.labels(None).unwrap_or_default();
                let rows = store.summaries(&ids).unwrap_or_default();
                drop(store);
                let rows: Vec<_> = rows.into_iter().filter(|r| r.account_id == label.account_id.as_str()).collect();
                // `l` on threads that all carry the label takes it off again.
                let has_all = !rows.is_empty() && rows.iter().all(|r| r.label_keys.contains(&label.label_key));
                let in_account: Vec<String> = rows.into_iter().map(|r| r.id).collect();
                if in_account.is_empty() {
                    self.toast("That label belongs to another account.", false, cx);
                    return;
                }
                let ctx = accounts
                    .iter()
                    .find(|a| a.id == label.account_id)
                    .map(|a| AccountCtx::new(a, &labels))
                    .unwrap_or_else(AccountCtx::gmail);
                let is_move = matches!(kind, PickerKind::Move(_));
                let action = if is_move && ctx.multi_label {
                    ThreadAction::Modify { add: vec![label.label_key.clone()], remove: vec!["sys:inbox".into()] }
                } else if is_move {
                    ThreadAction::Move { to: label.label_key.clone() }
                } else if has_all {
                    actions::remove_label(&label, &ctx)
                } else {
                    actions::add_label(&label)
                };
                let done = if is_move {
                    format!("Moved to {}", label.name)
                } else if has_all {
                    format!("Removed {}", label.name)
                } else {
                    format!("Labelled {}", label.name)
                };
                self.backend.apply(
                    vec![crate::vm::mail::PlannedApply {
                        account_id: label.account_id.as_str().to_owned(),
                        thread_ids: in_account,
                        action,
                    }],
                    Some(&done),
                );
            }
            other => self.picked_feature(other, value, window, cx),
        }
        self.refresh(cx);
    }

    /// Runs one command (from a key, the palette or a click).
    pub(crate) fn run(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        match id {
            sc::PALETTE_OPEN => return self.open_palette(Overlay::Palette, window, cx),
            sc::PALETTE_CLOSE => return self.close_overlay(window, cx),
            sc::PALETTE_NEXT | sc::PALETTE_PREV => {
                let n = self.palette_rows(cx).len();
                if n > 0 {
                    let d: isize = if id == sc::PALETTE_NEXT { 1 } else { -1 };
                    self.palette_sel = (self.palette_sel as isize + d).rem_euclid(n as isize) as usize;
                    self.palette_scroll.scroll_to_item(self.palette_sel);
                }
                return cx.notify();
            }
            sc::PALETTE_RUN => {
                if let Some(item) = self.palette_rows(cx).into_iter().nth(self.palette_sel) {
                    self.choose(item.id, window, cx);
                }
                return;
            }
            sc::HELP_SHORTCUTS => {
                self.overlay = Overlay::Help;
                return cx.notify();
            }
            sc::SETTINGS_OPEN => {
                self.overlay = Overlay::Settings(SettingsTab::General);
                window.focus(&self.focus);
                return cx.notify();
            }
            cmd::SETTINGS_AI => {
                self.overlay = Overlay::Settings(SettingsTab::Ai);
                window.focus(&self.focus);
                return cx.notify();
            }
            cmd::SIGN_OUT => {
                self.backend.sign_out();
                return;
            }
            cmd::QUIT => return cx.quit(),
            cmd::DEMO => {
                self.backend.start_demo();
                return;
            }
            sc::SEARCH_OPEN => {
                window.focus(&self.search.focus_handle(cx));
                return cx.notify();
            }
            sc::SEARCH_CLOSE => {
                self.search.update(cx, |f, cx| f.set_text("", cx));
                self.search_error = None;
                if matches!(self.vm.folder, Folder::Search { .. }) {
                    let _ = self.vm.go(Folder::Inbox, &self.backend.store.lock().unwrap_or_else(|e| e.into_inner()));
                    self.thread = None;
                }
                window.focus(&self.focus);
                return cx.notify();
            }
            sc::SEARCH_SUBMIT => return self.submit_search(window, cx),
            sc::COMPOSE_SEND => return self.send_compose_at(None, window, cx),
            sc::COMPOSE_CLOSE => return self.close_compose(true, window, cx),
            sc::COMPOSE_DISCARD => return self.close_compose(false, window, cx),
            sc::COMPOSE_CC | sc::COMPOSE_BCC => {
                if let Some(c) = &mut self.compose {
                    c.show_cc = true;
                    let f = if id == sc::COMPOSE_CC { c.cc.clone() } else { c.bcc.clone() };
                    window.focus(&f.focus_handle(cx));
                }
                return cx.notify();
            }
            sc::COMPOSE_ATTACH => return self.attach_dialog(window, cx),
            _ => {}
        }
        if self.run_feature_command(id, window, cx) {
            return;
        }
        if let Some(t) = id.strip_prefix(cmd::THEME) {
            let theme = match t {
                "light" => ThemePref::Light,
                "dark" => ThemePref::Dark,
                _ => ThemePref::System,
            };
            return self.change_settings(|s| s.theme = theme, window, cx);
        }
        if let Some(k) = id.strip_prefix(cmd::KEYMAP) {
            let keymap = if k == "gmail" { KeymapPref::Gmail } else { KeymapPref::Superhuman };
            return self.change_settings(|s| s.keymap = keymap, window, cx);
        }
        if let Some(d) = id.strip_prefix(cmd::DENSITY) {
            let density = if d == "compact" { Density::Compact } else { Density::Comfortable };
            return self.change_settings(|s| s.density = density, window, cx);
        }
        if !self.signed_in() {
            return;
        }
        if let Some(key) = id.strip_prefix(cmd::GO_LABEL_PREFIX) {
            let name =
                self.store().labels(None).unwrap_or_default().into_iter().find(|l| l.label_key == key).map(|l| l.name);
            let store = self.backend.store.clone();
            let guard = store.lock().unwrap_or_else(|e| e.into_inner());
            let _ = self.vm.go(Folder::Label { key: key.to_owned(), name: name.unwrap_or_default() }, &guard);
            drop(guard);
            self.thread = None;
            return cx.notify();
        }
        let intent = {
            let store = self.backend.store.lock().unwrap_or_else(|e| e.into_inner());
            self.vm.handle(id, &store).unwrap_or(Intent::None)
        };
        self.execute(intent, window, cx);
        if let Some(open) = self.vm.open.clone() {
            if self.thread.as_ref().is_none_or(|t| t.id != open) {
                self.open_thread(&open, cx);
            }
        } else {
            self.thread = None;
            self.list_scroll.scroll_to_item(self.vm.cursor, gpui::ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn execute(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<'_, Self>) {
        match intent {
            Intent::None | Intent::Unhandled => {}
            Intent::Open(id) => self.open_thread(&id, cx),
            Intent::Apply { thread_ids, kind } => {
                let here = self.vm.folder.clone();
                let plans = plan(kind, &thread_ids, &here, &self.store());
                match plans {
                    Ok(p) => {
                        let label = match kind {
                            ActionKind::ToggleRead | ActionKind::ToggleStar | ActionKind::MarkRead => None,
                            k => Some(k.done_label()),
                        };
                        self.backend.apply(p, label);
                        self.refresh(cx);
                    }
                    Err(e) => self.toast(e.to_string(), false, cx),
                }
            }
            Intent::Snooze(ids) => self.open_palette(Overlay::Picker(PickerKind::Snooze(ids)), window, cx),
            Intent::Remind(id) => self.open_palette(Overlay::Picker(PickerKind::Remind(id)), window, cx),
            Intent::Label(ids) => self.open_palette(Overlay::Picker(PickerKind::Label(ids)), window, cx),
            Intent::Move(ids) => self.open_palette(Overlay::Picker(PickerKind::Move(ids)), window, cx),
            Intent::GoLabel => self.open_palette(Overlay::Picker(PickerKind::GoLabel), window, cx),
            Intent::Reply { thread_id, all } => self.reply(&thread_id, Some(all), window, cx),
            Intent::Forward(thread_id) => self.reply(&thread_id, None, window, cx),
            Intent::Unsubscribe(thread_id) => {
                let thread = self.store().thread(&thread_id).ok().flatten();
                let options = thread.as_ref().and_then(|t| t.list_unsubscribe.clone());
                match actions::unsubscribe(options.as_ref()) {
                    // A mailto unsubscribe sends mail from the user's account: confirm where it goes first.
                    Ok(ThreadAction::Unsubscribe {
                        method: akatsuki_core::convex::schema::UnsubscribeMethod::Mailto,
                    }) => {
                        let sender = thread.as_ref().and_then(|t| t.last_from.as_ref()).map(|a| a.email.clone());
                        match options
                            .as_ref()
                            .and_then(|o| o.mailto.as_deref())
                            .and_then(|m| actions::mailto_target(m, sender.as_deref()))
                        {
                            Some((to, foreign)) => self.open_palette(
                                Overlay::Picker(PickerKind::Unsubscribe { thread_id, to, foreign }),
                                window,
                                cx,
                            ),
                            None => self.toast("This message has no usable unsubscribe address.", false, cx),
                        }
                    }
                    Ok(_) => self.unsubscribe_now(thread_id, cx),
                    Err(e) => self.toast(e.to_string(), false, cx),
                }
            }
            Intent::OpenOriginal { thread_id, images } => self.open_original(&thread_id, images, cx),
            Intent::ExpandAll => {
                if let Some(t) = &mut self.thread {
                    for m in &t.messages {
                        t.expanded.insert(m.id.as_str().to_owned());
                        if !t.bodies.contains_key(m.id.as_str()) {
                            self.backend.load_body(m.id.as_str());
                        }
                    }
                }
            }
            Intent::Summarize(id) => self.run_ai(AiFeature::Summarize, &id, cx),
            Intent::InstantReply(id) => self.run_ai(AiFeature::InstantReply, &id, cx),
            Intent::Compose => self.new_compose(window, cx),
            Intent::Search => {
                window.focus(&self.search.focus_handle(cx));
            }
            Intent::Palette => self.open_palette(Overlay::Palette, window, cx),
            Intent::Settings => self.overlay = Overlay::Settings(SettingsTab::General),
            Intent::ExtractActions(id) => self.run_ai(AiFeature::ExtractActions, &id, cx),
            Intent::Help => self.overlay = Overlay::Help,
            Intent::Undo => {
                if !self.backend.undo_last() {
                    self.toast("Nothing to undo", false, cx);
                } else {
                    self.pending_send = None;
                    self.toasts.retain(|t| !t.undo);
                    self.refresh(cx);
                }
            }
        }
    }

    /// Types a query into the search bar and runs it.
    pub(crate) fn search_for(&mut self, q: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.search.update(cx, |f, cx| f.set_text(q, cx));
        self.submit_search(window, cx);
    }

    pub(crate) fn submit_search(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let q = self.search.read(cx).content.trim().to_owned();
        if q.is_empty() {
            return self.run(sc::SEARCH_CLOSE, window, cx);
        }
        match self.backend.search(&q, &self.tz_name) {
            Ok(hits) => {
                self.search_error = None;
                let _ = self.vm.show_search(&q, hits, &self.backend.store.lock().unwrap_or_else(|e| e.into_inner()));
                self.thread = None;
                window.focus(&self.focus);
                // Convex search runs too and its hits are merged by thread id (§11.2).
                self.search_state = SearchState { query: q.clone(), server_pending: true, ..SearchState::default() };
                self.backend.search_server(&q, self.vm.account.clone());
            }
            Err(e) => self.search_error = Some(e),
        }
        cx.notify();
    }

    fn unsubscribe_now(&mut self, thread_id: String, cx: &mut Context<'_, Self>) {
        let thread = self.store().thread(&thread_id).ok().flatten();
        let options = thread.as_ref().and_then(|t| t.list_unsubscribe.clone());
        match (thread, actions::unsubscribe(options.as_ref())) {
            (Some(t), Ok(action)) => self.backend.apply(
                vec![crate::vm::mail::PlannedApply {
                    account_id: t.account_id.into_string(),
                    thread_ids: vec![thread_id],
                    action,
                }],
                Some("Unsubscribing"),
            ),
            (_, Err(e)) => self.toast(e.to_string(), false, cx),
            _ => {}
        }
    }

    fn open_original(&mut self, thread_id: &str, images: bool, cx: &mut Context<'_, Self>) {
        let Some(t) = &self.thread else { return };
        if t.id != thread_id {
            return;
        }
        let subject = self.store().thread(thread_id).ok().flatten().map(|t| t.subject).unwrap_or_default();
        let newest = t.messages.iter().rev().find(|m| matches!(t.bodies.get(m.id.as_str()), Some(Ok(_))));
        let Some(m) = newest else { return self.toast("The message is still loading.", false, cx) };
        if images {
            // Remote images only through the worker's proxy (D-399); the page opens when it is written.
            self.backend.open_original_with_images(m.id.as_str(), &subject);
            return;
        }
        match self.backend.write_original(m.id.as_str(), &subject) {
            Some(path) => cx.open_url(&format!("file://{}", path.display())),
            None => self.toast("The message is still loading.", false, cx),
        }
    }

    pub(crate) fn run_ai(&mut self, feature: AiFeature, thread_id: &str, cx: &mut Context<'_, Self>) {
        let name = self.email.split('@').next().unwrap_or("").to_owned();
        match self.backend.run_ai(feature, thread_id, &name, &self.tz_name) {
            Ok(request) => {
                self.ai = Some(AiPanel {
                    request,
                    feature,
                    thread_id: thread_id.to_owned(),
                    text: String::new(),
                    object: None,
                    error: None,
                    done: false,
                    egress: self.backend.ai_egress(feature),
                    model: None,
                });
            }
            Err(e) => self.toast(e, false, cx),
        }
        cx.notify();
    }

    // ───────────── compose ─────────────

    pub(crate) fn new_compose(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let accounts = self.store().accounts().unwrap_or_default();
        let account =
            self.vm.account.as_ref().and_then(|a| accounts.iter().find(|x| x.id.as_str() == a)).or(accounts.first());
        let Some(account) = account else {
            return self.toast("Connect a mailbox first (in the web app).", false, cx);
        };
        let draft = Draft::new(uuid::Uuid::now_v7().to_string(), account.id.as_str(), &account.email);
        self.open_compose(draft, window, cx);
    }

    fn reply(&mut self, thread_id: &str, all: Option<bool>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (quoted, when) = self
            .thread
            .as_ref()
            .filter(|t| t.id == thread_id)
            .and_then(|t| t.messages.last().map(|m| (t.bodies.get(m.id.as_str()), m.date)))
            .map(|(b, date)| {
                let text = b
                    .and_then(|r| r.as_ref().ok())
                    .and_then(|r| r.text.clone())
                    .map(|t| akatsuki_ai::transcript::strip_quoted(&t).to_owned());
                (text, time::full_time(date, &self.tz))
            })
            .unwrap_or((None, String::new()));
        let id = uuid::Uuid::now_v7().to_string();
        let draft = {
            let store = self.backend.store.lock().unwrap_or_else(|e| e.into_inner());
            match all {
                Some(all) => Draft::reply(
                    id,
                    &store,
                    thread_id,
                    all,
                    &self.my_addresses_locked(&store),
                    quoted.as_deref(),
                    &when,
                ),
                None => Draft::forward(id, &store, thread_id, quoted.as_deref(), &when),
            }
        };
        if let Ok(Some(d)) = draft {
            let forward = all.is_none();
            self.open_compose(d, window, cx);
            if let Some(c) = &self.compose {
                let target = if forward { c.to.clone() } else { c.body.clone() };
                target.update(cx, |f, cx| f.caret_to_start(cx));
                window.focus(&target.focus_handle(cx));
            }
        }
    }

    fn my_addresses_locked(&self, store: &akatsuki_core::store::Store) -> Vec<String> {
        store.accounts().unwrap_or_default().into_iter().map(|a| a.email).collect()
    }

    pub(crate) fn open_compose(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<'_, Self>) {
        // One draft at a time: the open one is saved to the provider's Drafts first.
        if let Some(old) = self.compose.take() {
            let previous = Self::read_draft(&old, cx);
            if previous.client_draft_id != draft.client_draft_id {
                self.backend.save_local_draft(&previous);
                self.backend.autosave(previous);
                self.toast("The previous draft was saved to Drafts.", false, cx);
            }
        }
        let mk = |cx: &mut Context<'_, Self>, placeholder: &str, value: &str, multiline: bool| {
            let f = cx.new(|cx| {
                let f = TextField::new(placeholder, cx);
                if multiline { f.multiline(18) } else { f }
            });
            f.update(cx, |f, cx| f.set_text(value, cx));
            f
        };
        let to = mk(cx, "name@example.com", &draft.to, false);
        let cc = mk(cx, "", &draft.cc, false);
        let bcc = mk(cx, "", &draft.bcc, false);
        let subject = mk(cx, "", &draft.subject, false);
        let body = mk(cx, "Write your message", &draft.body, true);
        let mut subs = Vec::new();
        for f in [&to, &cc, &bcc, &subject, &body] {
            subs.push(cx.subscribe_in(f, window, |this, _, e: &FieldEvent, window, cx| match e {
                FieldEvent::Changed => {
                    if let Some(c) = &mut this.compose {
                        c.dirty_at = Some(now_ms());
                        c.error = None;
                    }
                }
                FieldEvent::Cancel => this.run(sc::COMPOSE_CLOSE, window, cx),
                _ => {}
            }));
        }
        let instruction = mk(cx, "Tell AI what to write, then Enter", "", false);
        subs.push(cx.subscribe_in(&instruction, window, |this, _, e: &FieldEvent, window, cx| match e {
            FieldEvent::Submit => this.run_ai_write(window, cx),
            FieldEvent::Cancel => {
                if let Some(c) = &mut this.compose {
                    c.ai_bar = false;
                    window.focus(&c.body.focus_handle(cx));
                }
                cx.notify();
            }
            _ => {}
        }));
        let show_cc = !draft.cc.is_empty() || !draft.bcc.is_empty();
        let focus_to = draft.to.is_empty();
        self.backend.save_local_draft(&draft);
        self.compose = Some(ComposeState {
            draft,
            to,
            cc,
            bcc,
            subject,
            body,
            show_cc,
            error: None,
            dirty_at: None,
            instruction,
            ai_bar: false,
            ai_edit: None,
            ai_output: String::new(),
            ai_undo: None,
            dictation: None,
            conflict: None,
            _subs: subs,
        });
        if let Some(c) = &self.compose {
            let f = if focus_to { c.to.clone() } else { c.body.clone() };
            window.focus(&f.focus_handle(cx));
        }
        cx.notify();
    }

    /// The system file dialog (xdg-desktop-portal on Linux); the picked files are uploaded.
    fn attach_dialog(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.compose.is_none() {
            return;
        }
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let picked = rx.await;
            let _ = this.update_in(cx, |this, window, cx| match picked {
                Ok(Ok(Some(paths))) => this.attach_paths(paths, window, cx),
                Ok(Ok(None)) => {}
                _ => this.toast("The file dialog is not available. Drag files onto the message instead.", false, cx),
            });
        })
        .detach();
    }

    /// Attaches files (from the dialog or dropped on the composer) and starts their uploads.
    pub(crate) fn attach_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(c) = &mut self.compose else { return };
        for path in paths {
            let filename =
                path.file_name().and_then(|n| n.to_str()).map(str::to_owned).unwrap_or_else(|| "attachment".into());
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let key = uuid::Uuid::now_v7().to_string();
            c.draft.attachments.push(Attachment {
                key: key.clone(),
                filename,
                size,
                state: AttachmentState::Uploading,
            });
            self.backend.upload_attachment(c.draft.client_draft_id.clone(), key, path);
        }
        c.error = None;
        // Keep typing where the user was.
        window.focus(&c.body.focus_handle(cx));
        cx.notify();
    }

    /// Removes an attachment from the open draft.
    pub(crate) fn remove_attachment(&mut self, key: &str, cx: &mut Context<'_, Self>) {
        if let Some(c) = &mut self.compose {
            c.draft.attachments.retain(|a| a.key != key);
            c.dirty_at = Some(now_ms());
            cx.notify();
        }
    }

    pub(crate) fn read_draft(c: &ComposeState, cx: &App) -> Draft {
        Draft {
            to: c.to.read(cx).content.clone(),
            cc: c.cc.read(cx).content.clone(),
            bcc: c.bcc.read(cx).content.clone(),
            subject: c.subject.read(cx).content.clone(),
            body: c.body.read(cx).content.clone(),
            ..c.draft.clone()
        }
    }

    pub(crate) fn send_compose_at(&mut self, at: Option<i64>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(c) = &self.compose else { return };
        if c.ai_edit.is_some() {
            if let Some(c) = &mut self.compose {
                c.error = Some("Wait for the AI to finish, or press Escape to stop it.".into());
            }
            return cx.notify();
        }
        self.backend.cancel_dictation();
        let draft = Self::read_draft(c, cx);
        self.backend.save_local_draft(&draft);
        match self.backend.send(draft, self.settings.undo_send_seconds, at) {
            Ok(p) => {
                if p.later {
                    let when = time::full_time(p.send_at, &self.tz);
                    self.toast(format!("Scheduled for {when}"), true, cx);
                }
                self.pending_send = (!p.later).then_some(p);
                self.compose = None;
                window.focus(&self.focus);
            }
            Err(e) => {
                if let Some(c) = &mut self.compose {
                    c.error = Some(e);
                }
            }
        }
        cx.notify();
    }

    fn close_compose(&mut self, keep: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(c) = &mut self.compose {
            // Escape first stops a running AI request or closes the instruction row.
            if keep && c.ai_edit.is_some() {
                return self.stop_ai_edit(window, cx);
            }
            if keep && c.ai_bar {
                c.ai_bar = false;
                window.focus(&c.body.focus_handle(cx));
                return cx.notify();
            }
        }
        self.backend.cancel_dictation();
        if let Some(c) = self.compose.take() {
            let draft = Self::read_draft(&c, cx);
            if keep && draft.is_blank() {
                // An empty composer leaves nothing behind (D-281).
                self.backend.discard_draft(&draft);
            } else if keep {
                self.backend.save_local_draft(&draft);
                if c.conflict.is_none() {
                    self.backend.autosave(draft);
                }
                self.toast("Draft saved", false, cx);
            } else {
                self.backend.discard_draft(&draft);
                self.toast("Draft discarded", false, cx);
            }
        }
        window.focus(&self.focus);
        cx.notify();
    }

    // ───────────── sign-in ─────────────

    pub(crate) fn signin_primary(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let email = self.signin.email.read(cx).content.trim().to_owned();
        if email.is_empty() || !email.contains('@') {
            self.signin.status = Some("Enter your email address.".into());
            return cx.notify();
        }
        if self.signin.code_sent {
            let code = self.signin.code.read(cx).content.trim().to_owned();
            self.backend.verify_code(email, code);
        } else {
            self.backend.send_code(email);
            self.signin.code_sent = true;
        }
        cx.notify();
    }

    /// The root element.
    fn root(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        if !self.signed_in() {
            return self.render_signin(window, cx).into_any_element();
        }
        let t = self.theme;
        let width = f32::from(window.viewport_size().width);
        let content: AnyElement = if self.three_panes(window) {
            // ≥ 1100 px: list and reader side by side; compose takes the reader slot (§5.1).
            let slot = (width * 0.38).clamp(420., 720.);
            let reader = if self.compose.is_some() {
                self.render_compose_slot(window, cx)
            } else if self.vm.open.is_some() {
                self.render_reading(window, cx)
            } else {
                self.render_no_thread(cx)
            };
            div()
                .size_full()
                .flex()
                .child(div().flex_1().min_w(px(0.)).h_full().child(self.render_list(window, cx)))
                .child(div().w(px(slot)).flex_none().h_full().border_l_1().border_color(t.line).child(reader))
                .into_any_element()
        } else if self.vm.open.is_some() {
            self.render_reading(window, cx)
        } else {
            self.render_list(window, cx)
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div().flex_1().min_h(px(0.)).flex().child(self.render_sidebar(window, cx)).child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(self.render_header(window, cx))
                        .child(div().flex_1().min_h(px(0.)).child(content)),
                ),
            )
            .child(self.render_status(cx))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let t = self.theme;
        let root = self.root(window, cx);
        let overlays = self.render_overlays(window, cx);
        div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .size_full()
            .relative()
            .bg(t.bg)
            .text_color(t.fg)
            .font_family(crate::theme::fonts::SANS)
            .text_size(px(crate::theme::styles::BODY.size))
            .child(root)
            .children(overlays)
    }
}

/// The HUD's text of a key press: `Ctrl K`, `⇧E`, `G`.
fn hud_label(c: &keys::Combo) -> String {
    let mac = cfg!(target_os = "macos");
    let mut parts: Vec<String> = Vec::new();
    for (on, name, sym) in [(c.ctrl, "Ctrl", "⌃"), (c.alt, "Alt", "⌥"), (c.shift, "Shift", "⇧"), (c.cmd, "Win", "⌘")]
    {
        if on {
            parts.push(if mac { sym } else { name }.to_owned());
        }
    }
    let key = match c.key.as_str() {
        "enter" => "↵".to_owned(),
        "escape" => "Esc".to_owned(),
        "up" => "↑".to_owned(),
        "down" => "↓".to_owned(),
        "left" => "←".to_owned(),
        "right" => "→".to_owned(),
        k => k.to_uppercase(),
    };
    parts.push(key);
    parts.join(if mac { "" } else { " " })
}

/// Toasts are sentences: one rule for every message, however it was built ("Sent" → "Sent.").
pub(crate) fn sentence(text: String) -> String {
    let t = text.trim_end();
    if t.is_empty() || t.ends_with(['.', '!', '?', '…']) { t.to_owned() } else { format!("{t}.") }
}

#[cfg(test)]
mod sentence_tests {
    use super::sentence;

    #[test]
    fn toasts_end_with_a_full_stop() {
        assert_eq!(sentence("Sent".into()), "Sent.");
        assert_eq!(sentence("Draft saved ".into()), "Draft saved.");
        assert_eq!(sentence("The message is still loading.".into()), "The message is still loading.");
        assert_eq!(sentence("Really?".into()), "Really?");
        assert_eq!(sentence(String::new()), "");
    }
}
