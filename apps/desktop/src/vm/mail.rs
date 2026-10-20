//! The mail view model: which list is shown, the cursor, the selection, the open thread, and what
//! each command means. Pure logic over the mirror ([`Store`]); the GPUI views only draw it and the
//! workspace carries out the [`Intent`]s. Tested without a window.

use std::collections::{BTreeMap, BTreeSet};

use akatsuki_core::convex::ThreadAction;
use akatsuki_core::convex::client::SplitView;
use akatsuki_core::store::{ListQuery, Store, StoreError};
use akatsuki_shortcuts::ids as sc;

use super::actions::{self, AccountCtx, ActionError};
use crate::commands::ids as cmd;

/// A list in the sidebar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Folder {
    /// The inbox (with split tabs when the user has splits).
    #[default]
    Inbox,
    /// Starred.
    Starred,
    /// Sent.
    Sent,
    /// Drafts.
    Drafts,
    /// Snoozed.
    Snoozed,
    /// Threads with a reminder.
    Reminders,
    /// All mail (everything but trash and spam).
    All,
    /// Spam.
    Spam,
    /// Trash.
    Trash,
    /// A user label or folder.
    Label {
        /// `l:<id>`.
        key: String,
        /// Display name.
        name: String,
    },
    /// Search results.
    Search {
        /// The query as typed.
        query: String,
    },
}

impl Folder {
    /// The system lists in sidebar order.
    pub const SYSTEM: [Folder; 9] = [
        Folder::Inbox,
        Folder::Starred,
        Folder::Snoozed,
        Folder::Reminders,
        Folder::Sent,
        Folder::Drafts,
        Folder::All,
        Folder::Spam,
        Folder::Trash,
    ];

    /// Title.
    pub fn title(&self) -> String {
        match self {
            Folder::Inbox => "Inbox".into(),
            Folder::Starred => "Starred".into(),
            Folder::Sent => "Sent".into(),
            Folder::Drafts => "Drafts".into(),
            Folder::Snoozed => "Snoozed".into(),
            Folder::Reminders => "Reminders".into(),
            Folder::All => "All mail".into(),
            Folder::Spam => "Spam".into(),
            Folder::Trash => "Trash".into(),
            Folder::Label { name, .. } => name.clone(),
            Folder::Search { query } => format!("Search: {query}"),
        }
    }

    /// The label key the list reads, if it is a label list.
    pub fn label_key(&self) -> Option<&str> {
        Some(match self {
            Folder::Inbox => "sys:inbox",
            Folder::Starred => "sys:starred",
            Folder::Sent => "sys:sent",
            Folder::Drafts => "sys:drafts",
            Folder::Snoozed => "sys:snoozed",
            Folder::Spam => "sys:spam",
            Folder::Trash => "sys:trash",
            Folder::Label { key, .. } => key,
            Folder::Reminders | Folder::All | Folder::Search { .. } => return None,
        })
    }

    /// The folder a `go.*` command opens.
    pub fn for_command(id: &str) -> Option<Folder> {
        Some(match id {
            sc::GO_INBOX => Folder::Inbox,
            sc::GO_STARRED => Folder::Starred,
            sc::GO_SENT => Folder::Sent,
            sc::GO_DRAFTS => Folder::Drafts,
            sc::GO_SNOOZED => Folder::Snoozed,
            sc::GO_REMINDERS => Folder::Reminders,
            sc::GO_ALL => Folder::All,
            sc::GO_SPAM => Folder::Spam,
            sc::GO_TRASH => Folder::Trash,
            _ => return None,
        })
    }
}

/// A split tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitTab {
    /// `split:<id>` or `split:other`.
    pub key: String,
    /// Name.
    pub name: String,
    /// Show the unread count.
    pub show_count: bool,
}

/// Thread actions that depend on the account and the current state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// `e`.
    Archive,
    /// `shift+e`.
    MoveToInbox,
    /// `u`: read if any target is unread, else unread.
    ToggleRead,
    /// `s`: unstar if every target is starred, else star.
    ToggleStar,
    /// `#`.
    Trash,
    /// `!`: spam, or not spam inside Spam.
    Spam,
    /// Mark read (opening an unread thread).
    MarkRead,
}

impl ActionKind {
    /// Toast text after the action.
    pub fn done_label(self) -> &'static str {
        match self {
            ActionKind::Archive => "Archived",
            ActionKind::MoveToInbox => "Moved to Inbox",
            ActionKind::ToggleRead | ActionKind::MarkRead => "Updated",
            ActionKind::ToggleStar => "Updated",
            ActionKind::Trash => "Moved to Trash",
            ActionKind::Spam => "Marked as spam",
        }
    }
}

/// What a command asks the workspace to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Nothing more (the view model already changed).
    None,
    /// A thread was opened: load its bodies, mark it read.
    Open(String),
    /// Run an action on threads.
    Apply {
        /// Targets, in list order.
        thread_ids: Vec<String>,
        /// What.
        kind: ActionKind,
    },
    /// Choose a snooze time.
    Snooze(Vec<String>),
    /// Choose a reminder time.
    Remind(String),
    /// Choose a label to add.
    Label(Vec<String>),
    /// Choose a folder to move to.
    Move(Vec<String>),
    /// Choose a label to go to.
    GoLabel,
    /// Reply (or reply all) to a thread.
    Reply {
        /// Thread.
        thread_id: String,
        /// Reply all.
        all: bool,
    },
    /// Forward a thread.
    Forward(String),
    /// Unsubscribe from a thread's list.
    Unsubscribe(String),
    /// Open the sanitized original in the browser.
    OpenOriginal {
        /// Thread.
        thread_id: String,
        /// Load remote images.
        images: bool,
    },
    /// Expand every message of the open thread.
    ExpandAll,
    /// AI summary of the open thread.
    Summarize(String),
    /// AI reply suggestions for the open thread.
    InstantReply(String),
    /// AI action items of the open thread.
    ExtractActions(String),
    /// A new message.
    Compose,
    /// Focus search.
    Search,
    /// Open the palette.
    Palette,
    /// Open settings.
    Settings,
    /// Show shortcuts.
    Help,
    /// Undo the last action.
    Undo,
    /// Not a mail command.
    Unhandled,
}

/// The mail view model.
#[derive(Clone, Debug, Default)]
pub struct MailVm {
    /// The list shown.
    pub folder: Folder,
    /// Split tabs (inbox only; empty when the user has no splits).
    pub tabs: Vec<SplitTab>,
    /// The active tab key.
    pub tab: Option<String>,
    /// Only this account (unified when `None`).
    pub account: Option<String>,
    /// The list, newest first.
    pub ids: Vec<String>,
    /// The cursor row.
    pub cursor: usize,
    /// Selected threads.
    pub selected: BTreeSet<String>,
    anchor: Option<usize>,
    /// The thread in the reading pane.
    pub open: Option<String>,
    /// Search hits (for `Folder::Search`).
    pub hits: Vec<String>,
}

impl MailVm {
    /// Sets the split tabs from `splits.list`. The "Other" tab comes last.
    pub fn set_splits(&mut self, splits: &[SplitView]) {
        let mut sorted: Vec<&SplitView> = splits.iter().collect();
        sorted.sort_by(|a, b| a.order.total_cmp(&b.order));
        self.tabs = sorted
            .into_iter()
            .map(|s| SplitTab { key: s.label_key.clone(), name: s.name.clone(), show_count: s.show_count })
            .collect();
        if !self.tabs.is_empty() {
            self.tabs.push(SplitTab { key: "split:other".into(), name: "Other".into(), show_count: false });
        }
        if self.tab.as_ref().is_none_or(|t| !self.tabs.iter().any(|x| &x.key == t)) {
            let tab = self.tabs.first().map(|t| t.key.clone());
            if tab != self.tab && self.folder == Folder::Inbox {
                // A different list: start at its top.
                self.cursor = 0;
                self.selected.clear();
                self.anchor = None;
            }
            self.tab = tab;
        }
    }

    /// The mirror query of the current list (`None` for reminders and search).
    pub fn query(&self) -> Option<ListQuery> {
        let label_key = match (&self.folder, &self.tab) {
            (Folder::Inbox, Some(tab)) if !self.tabs.is_empty() => Some(tab.clone()),
            (Folder::Reminders | Folder::Search { .. }, _) => return None,
            (f, _) => f.label_key().map(str::to_owned),
        };
        Some(ListQuery { label_key, account_id: self.account.clone(), unread_only: false })
    }

    /// Re-reads the list. The cursor stays on its thread (or its row when the thread left); an
    /// open thread that left the list is replaced by the one that took its row (Superhuman's
    /// auto-advance), or closed at the end of the list.
    pub fn refresh(&mut self, store: &Store) -> Result<(), StoreError> {
        let ids = match self.query() {
            Some(q) => store.list_ids(&q)?,
            None if matches!(self.folder, Folder::Reminders) => store.reminder_ids(self.account.as_deref())?,
            None => {
                // Search hits that still exist and are not deleted.
                let known: BTreeSet<String> = store.summaries(&self.hits)?.into_iter().map(|s| s.id).collect();
                self.hits.iter().filter(|h| known.contains(*h)).cloned().collect()
            }
        };
        let cursor_id = self.ids.get(self.cursor).cloned();
        let open_row = self.open.as_ref().and_then(|o| self.ids.iter().position(|i| i == o));
        self.ids = ids;
        self.cursor = match cursor_id.and_then(|c| self.ids.iter().position(|i| *i == c)) {
            Some(i) => i,
            None => self.cursor.min(self.ids.len().saturating_sub(1)),
        };
        if let Some(open) = self.open.clone()
            && !self.ids.contains(&open)
            && let Some(row) = open_row
        {
            self.open = self.ids.get(row).cloned();
            self.cursor = row.min(self.ids.len().saturating_sub(1));
        }
        self.selected.retain(|s| self.ids.contains(s));
        Ok(())
    }

    /// Switches lists.
    pub fn go(&mut self, folder: Folder, store: &Store) -> Result<(), StoreError> {
        self.folder = folder;
        self.reset_position();
        self.refresh(store)
    }

    fn reset_position(&mut self) {
        self.ids.clear();
        self.cursor = 0;
        self.selected.clear();
        self.anchor = None;
        self.open = None;
    }

    /// The thread under the cursor.
    pub fn cursor_id(&self) -> Option<&String> {
        self.ids.get(self.cursor)
    }

    /// What thread commands act on: the selection, else the open thread, else the cursor row.
    pub fn targets(&self) -> Vec<String> {
        if !self.selected.is_empty() {
            return self.ids.iter().filter(|i| self.selected.contains(*i)).cloned().collect();
        }
        if let Some(o) = &self.open {
            return vec![o.clone()];
        }
        self.cursor_id().cloned().into_iter().collect()
    }

    fn single(&self) -> Option<String> {
        self.open.clone().or_else(|| self.cursor_id().cloned())
    }

    fn move_cursor(&mut self, delta: isize) -> Intent {
        if self.ids.is_empty() {
            return Intent::None;
        }
        let max = self.ids.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
        if self.open.is_some() {
            let id = self.ids[self.cursor].clone();
            if self.open.as_ref() != Some(&id) {
                self.open = Some(id.clone());
                return Intent::Open(id);
            }
        }
        Intent::None
    }

    fn extend(&mut self, delta: isize) {
        if self.ids.is_empty() {
            return;
        }
        let anchor = *self.anchor.get_or_insert(self.cursor);
        let max = self.ids.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
        let (lo, hi) = if anchor <= self.cursor { (anchor, self.cursor) } else { (self.cursor, anchor) };
        self.selected = self.ids[lo..=hi].iter().cloned().collect();
    }

    fn cycle_tab(&mut self, delta: isize, store: &Store) -> Result<Intent, StoreError> {
        if self.folder != Folder::Inbox || self.tabs.is_empty() {
            return Ok(Intent::None);
        }
        let at = self.tab.as_ref().and_then(|t| self.tabs.iter().position(|x| &x.key == t)).unwrap_or(0) as isize;
        let n = self.tabs.len() as isize;
        self.tab = Some(self.tabs[(at + delta).rem_euclid(n) as usize].key.clone());
        self.reset_position();
        self.refresh(store)?;
        Ok(Intent::None)
    }

    /// Runs a command id (from a key or the palette).
    pub fn handle(&mut self, id: &str, store: &Store) -> Result<Intent, StoreError> {
        if let Some(folder) = Folder::for_command(id) {
            self.go(folder, store)?;
            return Ok(Intent::None);
        }
        if let Some(key) = id.strip_prefix(cmd::GO_SPLIT_PREFIX) {
            self.folder = Folder::Inbox;
            self.tab = Some(key.to_owned());
            self.reset_position();
            self.refresh(store)?;
            return Ok(Intent::None);
        }
        if let Some(account) = id.strip_prefix(cmd::GO_ACCOUNT_PREFIX) {
            self.account = (!account.is_empty()).then(|| account.to_owned());
            self.reset_position();
            self.refresh(store)?;
            return Ok(Intent::None);
        }
        let thread_op = |kind| {
            let t = self.targets();
            if t.is_empty() { Intent::None } else { Intent::Apply { thread_ids: t, kind } }
        };
        let intent = match id {
            sc::NAV_NEXT => self.move_cursor(1),
            sc::NAV_PREV => self.move_cursor(-1),
            sc::NAV_OPEN => match self.cursor_id().cloned() {
                Some(t) => {
                    self.open = Some(t.clone());
                    Intent::Open(t)
                }
                None => Intent::None,
            },
            sc::NAV_BACK => {
                if self.open.take().is_none() {
                    if !self.selected.is_empty() {
                        self.selected.clear();
                        self.anchor = None;
                    } else if !matches!(self.folder, Folder::Inbox) {
                        self.go(Folder::Inbox, store)?;
                    }
                }
                Intent::None
            }
            sc::SELECT_TOGGLE => {
                if let Some(t) = self.cursor_id().cloned()
                    && !self.selected.remove(&t)
                {
                    self.selected.insert(t);
                }
                self.anchor = Some(self.cursor);
                Intent::None
            }
            sc::SELECT_EXTEND_DOWN => {
                self.extend(1);
                Intent::None
            }
            sc::SELECT_EXTEND_UP => {
                self.extend(-1);
                Intent::None
            }
            sc::SELECT_ALL => {
                if self.selected.len() == self.ids.len() {
                    self.selected.clear();
                } else {
                    self.selected = self.ids.iter().cloned().collect();
                }
                Intent::None
            }
            sc::SPLIT_NEXT => self.cycle_tab(1, store)?,
            sc::SPLIT_PREV => self.cycle_tab(-1, store)?,
            sc::GO_LABEL => Intent::GoLabel,
            sc::THREAD_ARCHIVE => thread_op(ActionKind::Archive),
            sc::THREAD_MOVE_TO_INBOX => thread_op(ActionKind::MoveToInbox),
            sc::THREAD_TOGGLE_READ => thread_op(ActionKind::ToggleRead),
            sc::THREAD_STAR => thread_op(ActionKind::ToggleStar),
            sc::THREAD_TRASH => thread_op(ActionKind::Trash),
            sc::THREAD_SPAM => thread_op(ActionKind::Spam),
            sc::THREAD_SNOOZE => non_empty(self.targets(), Intent::Snooze),
            sc::THREAD_LABEL => non_empty(self.targets(), Intent::Label),
            sc::THREAD_MOVE => non_empty(self.targets(), Intent::Move),
            sc::THREAD_REMIND => self.single().map_or(Intent::None, Intent::Remind),
            sc::THREAD_REPLY => self.single().map_or(Intent::None, |t| Intent::Reply { thread_id: t, all: false }),
            sc::THREAD_REPLY_ALL => self.single().map_or(Intent::None, |t| Intent::Reply { thread_id: t, all: true }),
            sc::THREAD_FORWARD => self.single().map_or(Intent::None, Intent::Forward),
            sc::THREAD_UNSUBSCRIBE => self.single().map_or(Intent::None, Intent::Unsubscribe),
            sc::THREAD_OPEN_ORIGINAL => {
                self.single().map_or(Intent::None, |t| Intent::OpenOriginal { thread_id: t, images: false })
            }
            sc::THREAD_SHOW_IMAGES => {
                self.single().map_or(Intent::None, |t| Intent::OpenOriginal { thread_id: t, images: true })
            }
            sc::THREAD_EXPAND_ALL => Intent::ExpandAll,
            sc::AI_SUMMARIZE => self.single().map_or(Intent::None, Intent::Summarize),
            sc::AI_INSTANT_REPLY => self.single().map_or(Intent::None, Intent::InstantReply),
            sc::AI_EXTRACT_ACTIONS => self.single().map_or(Intent::None, Intent::ExtractActions),
            sc::COMPOSE_NEW => Intent::Compose,
            sc::SEARCH_OPEN => Intent::Search,
            sc::PALETTE_OPEN => Intent::Palette,
            sc::SETTINGS_OPEN => Intent::Settings,
            sc::HELP_SHORTCUTS => Intent::Help,
            sc::UNDO => Intent::Undo,
            _ => Intent::Unhandled,
        };
        Ok(intent)
    }

    /// Adds hits from another search backend to the search list without moving the cursor.
    pub fn add_hits(&mut self, query: &str, more: &[String], store: &Store) -> Result<(), StoreError> {
        if !matches!(&self.folder, Folder::Search { query: q } if q == query) {
            return Ok(());
        }
        for h in more {
            if !self.hits.contains(h) {
                self.hits.push(h.clone());
            }
        }
        self.refresh(store)
    }

    /// Shows search results.
    pub fn show_search(&mut self, query: &str, hits: Vec<String>, store: &Store) -> Result<(), StoreError> {
        self.hits = hits;
        self.go(Folder::Search { query: query.to_owned() }, store)
    }
}

fn non_empty(t: Vec<String>, f: impl FnOnce(Vec<String>) -> Intent) -> Intent {
    if t.is_empty() { Intent::None } else { f(t) }
}

/// One `mail.apply` call of a gesture: threads of one account and the action for that account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedApply {
    /// Account.
    pub account_id: String,
    /// Threads (≤ 500 per call; larger selections are chunked by the caller).
    pub thread_ids: Vec<String>,
    /// Action.
    pub action: ThreadAction,
}

/// Builds the per-account actions of a gesture from the mirror's current visible state. `here` is
/// the list it runs in: `!` in Spam means "not spam" and `#` in Trash restores.
pub fn plan(
    kind: ActionKind,
    thread_ids: &[String],
    here: &Folder,
    store: &Store,
) -> Result<Vec<PlannedApply>, PlanError> {
    let (in_spam, in_trash) = (*here == Folder::Spam, *here == Folder::Trash);
    let rows = store.summaries(thread_ids)?;
    let accounts = store.accounts()?;
    let labels = store.labels(None)?;
    let any_unread = rows.iter().any(|r| r.unread);
    let all_starred = !rows.is_empty() && rows.iter().all(|r| r.starred);
    let mut by_account: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &rows {
        by_account.entry(r.account_id.clone()).or_default().push(r.id.clone());
    }
    let mut out = Vec::new();
    for (account_id, ids) in by_account {
        let ctx = accounts
            .iter()
            .find(|a| a.id.as_str() == account_id)
            .map(|a| AccountCtx::new(a, &labels))
            .unwrap_or_else(AccountCtx::gmail);
        let ids = match kind {
            // Only the threads the action changes.
            ActionKind::MarkRead => {
                rows.iter().filter(|r| r.account_id == account_id && r.unread).map(|r| r.id.clone()).collect()
            }
            _ => ids,
        };
        if ids.is_empty() {
            continue;
        }
        let action = match kind {
            ActionKind::Archive => actions::archive(&ctx)?,
            ActionKind::MoveToInbox => actions::move_to_inbox(&ctx),
            ActionKind::ToggleRead => actions::set_read(any_unread),
            ActionKind::MarkRead => actions::set_read(true),
            ActionKind::ToggleStar => actions::set_starred(!all_starred),
            ActionKind::Trash if in_trash => actions::untrash(),
            ActionKind::Trash => actions::trash(),
            ActionKind::Spam if in_spam => actions::not_spam(&ctx),
            ActionKind::Spam => actions::spam(&ctx),
        };
        out.push(PlannedApply { account_id, thread_ids: ids, action });
    }
    Ok(out)
}

/// Why a gesture cannot run.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// The mirror failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The account cannot do it.
    #[error(transparent)]
    Action(#[from] ActionError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn seeded() -> (Store, MailVm) {
        let mut store = Store::open_in_memory().unwrap();
        demo::seed_store(&mut store).unwrap();
        let mut vm = MailVm::default();
        vm.set_splits(&demo::splits());
        vm.refresh(&store).unwrap();
        (store, vm)
    }

    fn apply(store: &mut Store, vm: &mut MailVm, intent: Intent, op: &str) {
        let Intent::Apply { thread_ids, kind } = intent else { panic!("expected apply, got {intent:?}") };
        for p in plan(kind, &thread_ids, &Folder::Inbox, store).unwrap() {
            store.add_overlay(op, &p.thread_ids, &p.action).unwrap();
        }
        vm.refresh(store).unwrap();
    }

    #[test]
    fn inbox_opens_on_the_first_split_and_tabs_cycle() {
        let (store, mut vm) = seeded();
        assert_eq!(vm.tabs.last().unwrap().key, "split:other");
        assert_eq!(vm.tab.as_deref(), Some(vm.tabs[0].key.as_str()));
        assert!(!vm.ids.is_empty());
        let first_tab = vm.tab.clone();
        vm.handle(sc::SPLIT_NEXT, &store).unwrap();
        assert_ne!(vm.tab, first_tab);
        for _ in 1..vm.tabs.len() {
            vm.handle(sc::SPLIT_NEXT, &store).unwrap();
        }
        assert_eq!(vm.tab, first_tab, "tab wraps around");
        vm.handle(sc::SPLIT_PREV, &store).unwrap();
        assert_eq!(vm.tab.as_deref(), Some("split:other"));
    }

    #[test]
    fn j_k_move_and_enter_opens() {
        let (store, mut vm) = seeded();
        assert_eq!(vm.handle(sc::NAV_PREV, &store).unwrap(), Intent::None);
        assert_eq!(vm.cursor, 0);
        vm.handle(sc::NAV_NEXT, &store).unwrap();
        assert_eq!(vm.cursor, 1);
        let second = vm.ids[1].clone();
        assert_eq!(vm.handle(sc::NAV_OPEN, &store).unwrap(), Intent::Open(second.clone()));
        // j while reading opens the next thread.
        let third = vm.ids[2].clone();
        assert_eq!(vm.handle(sc::NAV_NEXT, &store).unwrap(), Intent::Open(third));
        vm.handle(sc::NAV_BACK, &store).unwrap();
        assert_eq!(vm.open, None);
    }

    #[test]
    fn archive_while_reading_advances_to_the_next_thread() {
        let (mut store, mut vm) = seeded();
        vm.handle(sc::NAV_OPEN, &store).unwrap();
        let first = vm.ids[0].clone();
        let second = vm.ids[1].clone();
        let intent = vm.handle(sc::THREAD_ARCHIVE, &store).unwrap();
        assert_eq!(intent, Intent::Apply { thread_ids: vec![first.clone()], kind: ActionKind::Archive });
        apply(&mut store, &mut vm, intent, "op1");
        assert!(!vm.ids.contains(&first));
        assert_eq!(vm.open.as_ref(), Some(&second));
        assert_eq!(vm.cursor, 0);
    }

    #[test]
    fn selection_extends_and_actions_cover_it() {
        let (mut store, mut vm) = seeded();
        vm.handle(sc::SELECT_EXTEND_DOWN, &store).unwrap();
        vm.handle(sc::SELECT_EXTEND_DOWN, &store).unwrap();
        assert_eq!(vm.selected.len(), 3);
        vm.handle(sc::SELECT_EXTEND_UP, &store).unwrap();
        assert_eq!(vm.selected.len(), 2);
        let targets = vm.targets();
        assert_eq!(targets, vm.ids[..2].to_vec());
        let before = vm.ids.len();
        let intent = vm.handle(sc::THREAD_TRASH, &store).unwrap();
        apply(&mut store, &mut vm, intent, "op1");
        assert_eq!(vm.ids.len(), before - 2);
        assert!(vm.selected.is_empty(), "selection drops threads that left the list");
        vm.handle(sc::SELECT_ALL, &store).unwrap();
        assert_eq!(vm.selected.len(), vm.ids.len());
        vm.handle(sc::NAV_BACK, &store).unwrap();
        assert!(vm.selected.is_empty(), "escape clears the selection");
    }

    #[test]
    fn toggles_depend_on_the_current_state() {
        let (mut store, mut vm) = seeded();
        vm.go(Folder::All, &store).unwrap();
        let unread = store.summaries(&vm.ids).unwrap().into_iter().find(|s| s.unread && !s.starred).unwrap().id;
        let p = plan(ActionKind::ToggleRead, std::slice::from_ref(&unread), &Folder::Inbox, &store).unwrap();
        assert_eq!(p[0].action, actions::set_read(true));
        store.add_overlay("op1", &p[0].thread_ids, &p[0].action).unwrap();
        let p = plan(ActionKind::ToggleRead, std::slice::from_ref(&unread), &Folder::Inbox, &store).unwrap();
        assert_eq!(p[0].action, actions::set_read(false));
        assert!(
            plan(ActionKind::MarkRead, std::slice::from_ref(&unread), &Folder::Inbox, &store).unwrap().is_empty(),
            "already read"
        );
        let p = plan(ActionKind::ToggleStar, std::slice::from_ref(&unread), &Folder::Inbox, &store).unwrap();
        assert_eq!(p[0].action, actions::set_starred(true));
    }

    #[test]
    fn gestures_split_by_account_with_each_providers_action() {
        let (store, mut vm) = seeded();
        vm.go(Folder::All, &store).unwrap();
        let rows = store.summaries(&vm.ids).unwrap();
        let gmail = rows.iter().find(|r| r.account_id == demo::GMAIL).unwrap().id.clone();
        let outlook = rows.iter().find(|r| r.account_id == demo::OUTLOOK).unwrap().id.clone();
        let p = plan(ActionKind::Archive, &[gmail, outlook], &Folder::Inbox, &store).unwrap();
        assert_eq!(p.len(), 2);
        let g = p.iter().find(|x| x.account_id == demo::GMAIL).unwrap();
        assert_eq!(g.action, actions::archive(&AccountCtx::gmail()).unwrap());
        let o = p.iter().find(|x| x.account_id == demo::OUTLOOK).unwrap();
        assert!(matches!(o.action, ThreadAction::Move { .. }), "Outlook archives by moving to its Archive folder");
    }

    #[test]
    fn go_commands_switch_lists_and_escape_returns_home() {
        let (store, mut vm) = seeded();
        vm.handle(sc::GO_STARRED, &store).unwrap();
        assert_eq!(vm.folder, Folder::Starred);
        assert!(store.summaries(&vm.ids).unwrap().iter().all(|s| s.starred));
        vm.handle(sc::NAV_BACK, &store).unwrap();
        assert_eq!(vm.folder, Folder::Inbox);
        assert_eq!(vm.handle(sc::GO_LABEL, &store).unwrap(), Intent::GoLabel);
        vm.handle(&format!("{}split:other", cmd::GO_SPLIT_PREFIX), &store).unwrap();
        assert_eq!(vm.tab.as_deref(), Some("split:other"));
        vm.handle(&format!("{}{}", cmd::GO_ACCOUNT_PREFIX, demo::OUTLOOK), &store).unwrap();
        assert!(store.summaries(&vm.ids).unwrap().iter().all(|s| s.account_id == demo::OUTLOOK));
        vm.handle(cmd::GO_ACCOUNT_PREFIX, &store).unwrap();
        assert_eq!(vm.account, None);
    }

    #[test]
    fn thread_commands_without_a_thread_do_nothing() {
        let store = Store::open_in_memory().unwrap();
        let mut vm = MailVm::default();
        vm.refresh(&store).unwrap();
        assert_eq!(vm.handle(sc::THREAD_ARCHIVE, &store).unwrap(), Intent::None);
        assert_eq!(vm.handle(sc::THREAD_REPLY, &store).unwrap(), Intent::None);
        assert_eq!(vm.handle(sc::COMPOSE_NEW, &store).unwrap(), Intent::Compose);
        assert_eq!(vm.handle("nonsense", &store).unwrap(), Intent::Unhandled);
    }

    #[test]
    fn reminders_and_search_lists() {
        let (store, mut vm) = seeded();
        vm.handle(sc::GO_REMINDERS, &store).unwrap();
        assert!(!vm.ids.is_empty());
        let some = vm.ids[0].clone();
        vm.show_search("from:ada", vec![some.clone(), "gone".into()], &store).unwrap();
        assert_eq!(vm.ids, vec![some]);
        assert_eq!(vm.folder.title(), "Search: from:ada");
    }
}

#[cfg(test)]
mod folder_tests {
    use super::*;
    use crate::demo;

    #[test]
    fn spam_and_trash_invert_inside_their_folders() {
        let mut store = Store::open_in_memory().unwrap();
        demo::seed_store(&mut store).unwrap();
        let p = plan(ActionKind::Spam, &["t-spam".into()], &Folder::Spam, &store).unwrap();
        assert_eq!(p[0].action, actions::not_spam(&AccountCtx::gmail()));
        let p = plan(ActionKind::Trash, &["t-board".into()], &Folder::Trash, &store).unwrap();
        assert_eq!(p[0].action, actions::untrash());
        let p = plan(ActionKind::Trash, &["t-board".into()], &Folder::Inbox, &store).unwrap();
        assert_eq!(p[0].action, actions::trash());
    }

    #[test]
    fn other_backends_add_hits_without_moving_the_cursor() {
        let mut store = Store::open_in_memory().unwrap();
        crate::demo::seed_store(&mut store).unwrap();
        let mut vm = MailVm::default();
        vm.show_search("deck", vec!["t-board".into()], &store).unwrap();
        vm.add_hits("other query", &["t-dawn".into()], &store).unwrap();
        assert_eq!(vm.ids, vec!["t-board"], "hits of another query are ignored");
        vm.add_hits("deck", &["t-board".into(), "t-dawn".into(), "t-missing".into()], &store).unwrap();
        assert_eq!(vm.ids, vec!["t-board", "t-dawn"], "appended once; unknown threads wait for the feed");
        assert_eq!(vm.hits.len(), 3);
        assert_eq!(vm.cursor, 0);
    }
}
