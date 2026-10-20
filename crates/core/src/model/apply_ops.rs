//! Visible thread state as a pure function (ARCHITECTURE §5.1).
//!
//! `labelKeys = applyOps(syncedLabelKeys, nonTerminalOpsInCreationOrder)`. Optimistic updates,
//! provider echoes, failures, cancellations and undo all reduce to "recompute". This is the Rust
//! twin of `applyOps` and `inverseAction` in `packages/mail-model/src/applyOps.ts`; both run
//! `packages/mail-model/fixtures/apply-ops.json` (`cases` in `model::shared_fixtures`, `inverses`
//! in this module's tests, D-250).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::convex::ThreadAction;

/// Keys that are mutually exclusive locations on folder providers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyContext {
    /// On Graph and IMAP: `sys:inbox`, `sys:trash`, `sys:spam`, `sys:drafts`, `sys:sent`,
    /// `sys:snoozed` and every `l:<id>` whose label is exclusive. Empty for Gmail.
    pub folder_keys: BTreeSet<String>,
}

impl ApplyContext {
    /// The context for Gmail (no folder keys).
    pub fn gmail() -> Self {
        Self::default()
    }

    /// The context for a folder provider, given the ids of its exclusive user labels.
    pub fn folders<I, S>(exclusive_label_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut folder_keys: BTreeSet<String> =
            ["sys:inbox", "sys:trash", "sys:spam", "sys:drafts", "sys:sent", "sys:snoozed"]
                .into_iter()
                .map(str::to_owned)
                .collect();
        folder_keys.extend(exclusive_label_ids.into_iter().map(|id| format!("l:{}", id.as_ref())));
        Self { folder_keys }
    }
}

/// The visible state of a thread: sorted, de-duplicated keys and whether it is deleted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visible {
    /// Label keys, sorted and de-duplicated.
    pub keys: Vec<String>,
    /// A pending `delete_forever` hides the thread.
    pub deleted: bool,
}

fn sort_dedupe<I: IntoIterator<Item = String>>(keys: I) -> Vec<String> {
    keys.into_iter().collect::<BTreeSet<_>>().into_iter().collect()
}

fn minus<'a>(keys: &'a [String], remove: impl IntoIterator<Item = &'a str> + Clone) -> Vec<String> {
    keys.iter().filter(|k| !remove.clone().into_iter().any(|r| r == k.as_str())).cloned().collect()
}

fn norm(state: &Visible, keys: Vec<String>) -> Visible {
    Visible { keys: sort_dedupe(keys), deleted: state.deleted }
}

/// Applies one action to a visible state.
pub fn apply_action(state: &Visible, action: &ThreadAction, ctx: &ApplyContext) -> Visible {
    match action {
        ThreadAction::Modify { add, remove } => {
            let mut keys = minus(&state.keys, remove.iter().map(String::as_str));
            keys.extend(add.iter().cloned());
            norm(state, keys)
        }
        ThreadAction::Trash => {
            let fixed = ["sys:inbox", "sys:spam", "sys:snoozed"];
            let mut keys = minus(&state.keys, fixed.iter().copied().chain(ctx.folder_keys.iter().map(String::as_str)));
            keys.push("sys:trash".to_owned());
            norm(state, keys)
        }
        ThreadAction::Untrash => {
            let mut keys = minus(&state.keys, ["sys:trash"]);
            keys.push("sys:inbox".to_owned());
            norm(state, keys)
        }
        ThreadAction::Move { to } => {
            let mut keys = minus(&state.keys, ctx.folder_keys.iter().map(String::as_str));
            keys.push(to.clone());
            norm(state, keys)
        }
        ThreadAction::DeleteForever => Visible { keys: Vec::new(), deleted: true },
        ThreadAction::Unsubscribe { .. } => state.clone(),
    }
}

/// Folds `actions` over the synced keys.
pub fn apply_ops<'a, I>(synced: &[String], actions: I, ctx: &ApplyContext) -> Visible
where
    I: IntoIterator<Item = &'a ThreadAction>,
{
    let start = Visible { keys: sort_dedupe(synced.iter().cloned()), deleted: false };
    actions.into_iter().fold(start, |state, action| apply_action(&state, action, ctx))
}

/// The label-key diff an action causes (`ops.effect`), used for undo (§5.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    /// Keys present after but not before.
    pub added: Vec<String>,
    /// Keys present before but not after.
    pub removed: Vec<String>,
}

impl Effect {
    /// The diff between two visible states.
    pub fn between(before: &Visible, after: &Visible) -> Self {
        let b: BTreeSet<&String> = before.keys.iter().collect();
        let a: BTreeSet<&String> = after.keys.iter().collect();
        Self {
            added: a.difference(&b).map(|s| (*s).clone()).collect(),
            removed: b.difference(&a).map(|s| (*s).clone()).collect(),
        }
    }

    /// `true` when the action changed nothing (undo yields `mode: "noop"`).
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }

    /// The inverse action for undo on Gmail-like providers: `modify{add: removed, remove: added}`.
    /// Use [`inverse_action`] for the full rule (folder providers, actions without an inverse).
    pub fn inverse(&self) -> ThreadAction {
        ThreadAction::Modify { add: self.removed.clone(), remove: self.added.clone() }
    }
}

/// Keys that describe state, not location; never a destination for a folder move.
const NON_LOCATION_KEYS: [&str; 3] = ["sys:unread", "sys:starred", "sys:important"];

/// The inverse of an applied action for undo (§5.2, D-250), or `None` when there is nothing to undo
/// (`mode: "noop"`). Twin of `inverseAction` in `packages/mail-model/src/applyOps.ts`; both run the
/// `inverses` vectors in `packages/mail-model/fixtures/apply-ops.json`.
///
/// - An empty effect, `unsubscribe` and `delete_forever` have no inverse (a permanent delete is only
///   undone by cancelling its op while it is still queued).
/// - On folder providers (`folder_provider`: Graph, IMAP) a location change (`move`, `trash`,
///   `untrash`) is undone by `move{to}` back to the first removed key (sorted) that is a location,
///   or `sys:inbox` when the action removed none.
/// - Everything else swaps the effect: `modify{add: removed, remove: added}`.
pub fn inverse_action(action: &ThreadAction, effect: &Effect, folder_provider: bool) -> Option<ThreadAction> {
    if effect.is_empty() || matches!(action, ThreadAction::Unsubscribe { .. } | ThreadAction::DeleteForever) {
        return None;
    }
    if folder_provider && matches!(action, ThreadAction::Move { .. } | ThreadAction::Trash | ThreadAction::Untrash) {
        let mut removed: Vec<&String> = effect.removed.iter().collect();
        removed.sort();
        let back = removed
            .into_iter()
            .find(|k| !NON_LOCATION_KEYS.contains(&k.as_str()))
            .cloned()
            .unwrap_or_else(|| "sys:inbox".to_owned());
        return Some(ThreadAction::Move { to: back });
    }
    Some(effect.inverse())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn modify(add: &[&str], remove: &[&str]) -> ThreadAction {
        ThreadAction::Modify { add: keys(add), remove: keys(remove) }
    }

    #[test]
    fn archive_on_gmail_removes_inbox() {
        let v = apply_ops(&keys(&["sys:unread", "sys:inbox"]), [&modify(&[], &["sys:inbox"])], &ApplyContext::gmail());
        assert_eq!(v, Visible { keys: keys(&["sys:unread"]), deleted: false });
    }

    #[test]
    fn keys_are_sorted_and_deduped() {
        let v = apply_ops(&keys(&["sys:inbox", "l:b", "l:a", "l:a"]), [], &ApplyContext::gmail());
        assert_eq!(v.keys, keys(&["l:a", "l:b", "sys:inbox"]));
    }

    #[test]
    fn trash_clears_folders_on_folder_providers() {
        let ctx = ApplyContext::folders(["arch"]);
        let v = apply_ops(&keys(&["l:arch", "sys:starred"]), [&ThreadAction::Trash], &ctx);
        assert_eq!(v.keys, keys(&["sys:starred", "sys:trash"]));
    }

    #[test]
    fn trash_on_gmail_keeps_user_labels() {
        let v =
            apply_ops(&keys(&["l:work", "sys:inbox", "sys:snoozed"]), [&ThreadAction::Trash], &ApplyContext::gmail());
        assert_eq!(v.keys, keys(&["l:work", "sys:trash"]));
    }

    #[test]
    fn move_replaces_folder() {
        let ctx = ApplyContext::folders(["arch"]);
        let v = apply_ops(&keys(&["sys:inbox", "sys:unread"]), [&ThreadAction::Move { to: "l:arch".into() }], &ctx);
        assert_eq!(v.keys, keys(&["l:arch", "sys:unread"]));
    }

    #[test]
    fn untrash_returns_to_inbox() {
        let v = apply_ops(&keys(&["sys:trash"]), [&ThreadAction::Untrash], &ApplyContext::gmail());
        assert_eq!(v.keys, keys(&["sys:inbox"]));
    }

    #[test]
    fn delete_forever_hides_and_later_actions_keep_it_deleted() {
        let v = apply_ops(
            &keys(&["sys:inbox"]),
            [&ThreadAction::DeleteForever, &modify(&["sys:starred"], &[])],
            &ApplyContext::gmail(),
        );
        assert!(v.deleted);
        assert_eq!(v.keys, keys(&["sys:starred"]));
    }

    #[test]
    fn effect_and_inverse() {
        let before = Visible { keys: keys(&["sys:inbox", "sys:unread"]), deleted: false };
        let after = apply_action(&before, &modify(&["l:x"], &["sys:inbox"]), &ApplyContext::gmail());
        let effect = Effect::between(&before, &after);
        assert_eq!(effect.added, keys(&["l:x"]));
        assert_eq!(effect.removed, keys(&["sys:inbox"]));
        let undone = apply_action(&after, &effect.inverse(), &ApplyContext::gmail());
        assert_eq!(undone, before);
    }

    #[test]
    fn folder_moves_are_undone_by_moving_back() {
        let ctx = ApplyContext::folders(["arch"]);
        let before = Visible { keys: keys(&["sys:inbox", "sys:unread"]), deleted: false };
        let action = ThreadAction::Move { to: "l:arch".into() };
        let after = apply_action(&before, &action, &ctx);
        let inverse = inverse_action(&action, &Effect::between(&before, &after), true).unwrap();
        assert_eq!(inverse, ThreadAction::Move { to: "sys:inbox".into() });
        assert_eq!(apply_action(&after, &inverse, &ctx), before);
        assert_eq!(inverse_action(&ThreadAction::DeleteForever, &Effect::between(&before, &after), true), None);
    }

    /// The `inverses` vectors of `packages/mail-model/fixtures/apply-ops.json` (D-250), shared with
    /// `packages/mail-model/test/fixtures.test.ts`.
    #[test]
    fn shared_inverse_vectors() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/mail-model/fixtures/apply-ops.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let fx: serde_json::Value = serde_json::from_str(&text).unwrap();
        let cases = fx["inverses"].as_array().expect("inverses");
        assert!(cases.len() >= 20, "inverse vectors missing");
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let provider = case["provider"].as_str().unwrap();
            let strings = |v: &serde_json::Value| -> Vec<String> {
                v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect()
            };
            let ctx = if provider == "gmail" {
                ApplyContext::gmail()
            } else {
                ApplyContext::folders(strings(&case["exclusiveLabelIds"]))
            };
            let synced = strings(&case["synced"]);
            let action: ThreadAction = serde_json::from_value(case["action"].clone()).unwrap();
            let before = apply_ops(&synced, [], &ctx);
            let after = apply_ops(&synced, [&action], &ctx);
            let effect = Effect::between(&before, &after);
            let want_effect: Effect = serde_json::from_value(case["effect"].clone()).unwrap();
            assert_eq!(effect, want_effect, "{name} effect");
            let inverse = inverse_action(&action, &effect, provider != "gmail");
            let want_inverse: Option<ThreadAction> = serde_json::from_value(case["inverse"].clone()).unwrap();
            assert_eq!(inverse, want_inverse, "{name} inverse");
            let restored = match &inverse {
                Some(inv) => apply_ops(&synced, [&action, inv], &ctx),
                None => after,
            };
            let want_restored: Visible = serde_json::from_value(case["restored"].clone()).unwrap();
            assert_eq!(restored, want_restored, "{name} restored");
        }
    }
}
