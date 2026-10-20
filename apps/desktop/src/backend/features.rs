//! Settings sync, snippets, labels, op-failure Retry, send later and the compose store.

use std::collections::HashSet;
use std::sync::Arc;

use futures::StreamExt;

use akatsuki_core::convex::ThreadAction;
use akatsuki_core::convex::client::MeView;
use akatsuki_core::convex::client::{
    AckFailureArgs, DraftDiscardArgs, LabelCreateArgs, LabelRemoveArgs, LabelRenameArgs, OpRetryArgs, PrefsPatch,
    SendsCancelArgs, SendsListArgs, SendsRescheduleArgs, SendsResolveArgs, SnippetIdArgs, SnippetSaveArgs,
    UnknownResolution, UpdatePrefsArgs,
};
use akatsuki_core::convex::rows::OpFailure;
use akatsuki_core::time::now_ms;

use super::{Backend, Event, Remote, safe};
use crate::vm::compose::Draft;
use crate::vm::mail::PlannedApply;
use crate::vm::prefs::Synced;

/// The sentence of a failure toast (§5.7): "Couldn't update 'Subject' in me@…: <reason>".
pub fn failure_text(f: &OpFailure) -> String {
    let verb = match f.payload_kind.as_str() {
        "thread" => "update",
        "label_create" => "create the label",
        "label_rename" => "rename the label",
        "label_delete" => "delete the label",
        "send" => "send",
        "delete_draft" => "discard the draft",
        _ => "finish an action",
    };
    let subject = f
        .subject
        .as_deref()
        .map(|s| {
            let s = s.trim();
            let s = if s.is_empty() { "(no subject)" } else { s };
            let clipped: String = s.chars().take(60).collect();
            format!(" '{}{}'", clipped, if s.chars().count() > 60 { "…" } else { "" })
        })
        .unwrap_or_default();
    let reason = f.error.message.trim_end_matches('.');
    format!("Couldn't {verb}{subject} in {}: {reason}", f.account_email)
}

impl Backend {
    // ───────────── me and settings sync ─────────────

    /// The last `me.get`.
    pub fn me_view(&self) -> Option<MeView> {
        self.me.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(super) async fn refresh_me(self: &Arc<Self>, r: &Remote) {
        if let Ok(Some(me)) = r.api.me().await {
            *self.me.write().unwrap_or_else(|e| e.into_inner()) = Some(me.clone());
            self.emit(Event::Me(Box::new(me)));
        }
    }

    /// Sends the device's changed synced settings (`me.updatePrefs`); `Event::PrefsSynced`
    /// reports what the server stored.
    pub fn sync_prefs(self: &Arc<Self>, local: Synced, patch: PrefsPatch) {
        let Some(r) = self.remote() else { return };
        if patch.is_empty() {
            return;
        }
        let me = self.clone();
        self.rt.spawn(async move {
            if r.api.update_prefs(&UpdatePrefsArgs { patch }).await.is_ok() {
                me.emit(Event::PrefsSynced(local));
            }
        });
    }

    // ───────────── snippets ─────────────

    pub(super) async fn refresh_snippets_now(self: &Arc<Self>, r: &Remote) {
        if let Ok(list) = r.api.list_snippets().await {
            *self.snippets.write().unwrap_or_else(|e| e.into_inner()) = list.clone();
            self.emit(Event::Snippets(list));
        }
    }

    /// Re-reads the snippets.
    pub fn refresh_snippets(self: &Arc<Self>) {
        let Some(r) = self.remote() else { return };
        let me = self.clone();
        self.rt.spawn(async move { me.refresh_snippets_now(&r).await });
    }

    /// Creates or updates a snippet. The body is the editor's plain text, stored as simple HTML.
    pub fn save_snippet(
        self: &Arc<Self>,
        snippet_id: Option<String>,
        name: String,
        shortcut: Option<String>,
        subject: Option<String>,
        body: String,
    ) {
        let Some(r) = self.remote() else { return };
        let args = SnippetSaveArgs {
            snippet_id: snippet_id.map(|s| s.as_str().into()),
            name: name.trim().to_owned(),
            shortcut: shortcut.map(|s| s.trim().trim_start_matches(';').to_lowercase()).filter(|s| !s.is_empty()),
            subject: subject.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
            variables: crate::vm::snippets::variables_in(&body),
            body_html: crate::vm::snippets::text_to_html(&body),
        };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.save_snippet(&args).await {
                Ok(_) => {
                    me.toast("Snippet saved", false);
                    me.refresh_snippets_now(&r).await;
                }
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Deletes a snippet.
    pub fn remove_snippet(self: &Arc<Self>, snippet_id: &str) {
        let Some(r) = self.remote() else { return };
        let args = SnippetIdArgs { snippet_id: snippet_id.into() };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.remove_snippet(&args).await {
                Ok(()) => {
                    me.toast("Snippet deleted", false);
                    me.refresh_snippets_now(&r).await;
                }
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Counts a use (orders the picker, `snippets.markUsed`).
    pub fn mark_snippet_used(self: &Arc<Self>, snippet_id: &str) {
        let Some(r) = self.remote() else { return };
        let args = SnippetIdArgs { snippet_id: snippet_id.into() };
        if let Some(s) =
            self.snippets.write().unwrap_or_else(|e| e.into_inner()).iter_mut().find(|s| s.id == args.snippet_id)
        {
            s.use_count += 1.0;
            s.last_used_at = Some(now_ms() as f64);
        }
        self.rt.spawn(async move {
            let _ = r.api.mark_snippet_used(&args).await;
        });
    }

    // ───────────── labels ─────────────

    /// Creates a label on an account (`labels.create`) and, when `apply_to` names threads, adds it
    /// to them; the thread op waits for the label op on the server (§5.4).
    pub fn create_label(self: &Arc<Self>, account_id: String, name: String, apply_to: Vec<String>) {
        let Some(r) = self.remote() else { return };
        let name = name.trim().to_owned();
        if name.is_empty() {
            return self.toast("Type a name for the label", false);
        }
        let args = LabelCreateArgs {
            client_op_id: uuid::Uuid::now_v7().to_string(),
            account_id: account_id.as_str().into(),
            name: name.clone(),
            color: None,
        };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.create_label(&args).await {
                Ok(created) if apply_to.is_empty() => {
                    let _ = created;
                    me.toast(format!("Created {name}"), false);
                }
                Ok(created) => me.apply(
                    vec![PlannedApply {
                        account_id,
                        thread_ids: apply_to,
                        action: ThreadAction::Modify { add: vec![created.label_key], remove: vec![] },
                    }],
                    Some(&format!("Labelled {name}")),
                ),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Renames a user label (`labels.rename`); the feed brings the new name.
    pub fn rename_label(self: &Arc<Self>, label_id: &str, name: String) {
        let Some(r) = self.remote() else { return };
        let name = name.trim().to_owned();
        if name.is_empty() {
            return self.toast("Type a name for the label", false);
        }
        let args = LabelRenameArgs {
            client_op_id: uuid::Uuid::now_v7().to_string(),
            label_id: label_id.into(),
            name: name.clone(),
        };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.rename_label(&args).await {
                Ok(_) => me.toast(format!("Renamed to {name}"), false),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Deletes a user label (`labels.remove`): conversations keep their mail and lose the label.
    pub fn remove_label(self: &Arc<Self>, label_id: &str, name: &str) {
        let Some(r) = self.remote() else { return };
        let args = LabelRemoveArgs { client_op_id: uuid::Uuid::now_v7().to_string(), label_id: label_id.into() };
        let me = self.clone();
        let name = name.to_owned();
        self.rt.spawn(async move {
            match r.api.remove_label(&args).await {
                Ok(_) => me.toast(format!("Deleted {name}"), false),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    // ───────────── op failures (§5.7) ─────────────

    /// Failures shown and not yet retried or dismissed, newest first.
    pub fn failures(&self) -> Vec<OpFailure> {
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(super) async fn watch_failures(self: Arc<Self>, r: Remote) {
        let Ok(mut sub) = r.api.op_failures().await else { return };
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(Ok(list)) = sub.next().await {
            let fresh: Vec<OpFailure> = list.into_iter().filter(|f| seen.insert(f.id.as_str().to_owned())).collect();
            if fresh.is_empty() {
                continue;
            }
            let all = {
                let mut cur = self.failures.lock().unwrap_or_else(|e| e.into_inner());
                for f in fresh {
                    cur.insert(0, f);
                }
                cur.truncate(20);
                cur.clone()
            };
            self.emit(Event::Failures(all));
        }
    }

    fn take_failure(&self, op_id: &str) -> Option<OpFailure> {
        let mut cur = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        let at = cur.iter().position(|f| f.id.as_str() == op_id)?;
        Some(cur.remove(at))
    }

    /// Retry (`ops.retry`): the same action again as a new op. Sends are excluded (§4.2).
    pub fn retry_failure(self: &Arc<Self>, op_id: &str) {
        let Some(r) = self.remote() else { return };
        let Some(f) = self.take_failure(op_id) else { return self.toast("Nothing to retry", false) };
        self.emit(Event::Failures(self.failures()));
        if f.payload_kind == "send" {
            return self.toast("Sends cannot be retried: open Show scheduled messages", false);
        }
        let args = OpRetryArgs { op_id: f.id.clone(), client_op_id: uuid::Uuid::now_v7().to_string() };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.retry_op(&args).await {
                Ok(_) => me.toast("Trying again", false),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Dismiss (`ops.ackFailure`).
    pub fn dismiss_failure(self: &Arc<Self>, op_id: &str) {
        let Some(r) = self.remote() else { return };
        let Some(f) = self.take_failure(op_id) else { return };
        self.emit(Event::Failures(self.failures()));
        self.rt.spawn(async move {
            let _ = r.api.ack_failures(&AckFailureArgs { op_ids: vec![f.id] }).await;
        });
    }

    // ───────────── scheduled sends ─────────────

    /// Reads `sends.list` (scheduled, due, sending, failed and unknown sends).
    pub fn list_scheduled(self: &Arc<Self>) {
        let Some(r) = self.remote() else { return };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.list_sends(&SendsListArgs::default()).await {
                Ok(list) => me.emit(Event::Scheduled(list)),
                Err(e) => me.toast(safe(&e), false),
            }
        });
    }

    /// Changes the time of a scheduled send.
    pub fn reschedule(self: &Arc<Self>, client_send_id: String, send_at: i64) {
        let Some(r) = self.remote() else { return };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.reschedule_send(&SendsRescheduleArgs { client_send_id, send_at }).await {
                Ok(()) => me.toast("Rescheduled", false),
                Err(e) => me.toast(safe(&e), false),
            }
            me.list_scheduled();
        });
    }

    /// Cancels a scheduled send; the draft stays in Drafts.
    pub fn cancel_scheduled(self: &Arc<Self>, client_send_id: String) {
        let Some(r) = self.remote() else { return };
        let me = self.clone();
        self.rt.spawn(async move {
            match r.api.sends_cancel(&SendsCancelArgs { client_send_id }).await {
                Ok(c) if c.result == akatsuki_core::convex::client::CancelOutcome::Cancelled => {
                    me.toast("Cancelled. The draft is in Drafts.", false)
                }
                Ok(_) => me.toast("Too late: the message was already sent.", false),
                Err(e) => me.toast(safe(&e), false),
            }
            me.list_scheduled();
        });
    }

    /// Resolves a send whose outcome is unknown (§5.6): it was sent, or drop it.
    pub fn resolve_unknown(self: &Arc<Self>, client_send_id: String, was_sent: bool) {
        let Some(r) = self.remote() else { return };
        let resolution = if was_sent { UnknownResolution::WasSent } else { UnknownResolution::Discard };
        let me = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = r.api.resolve_unknown_send(&SendsResolveArgs { client_send_id, resolution }).await {
                me.toast(safe(&e), false);
            }
            me.list_scheduled();
        });
    }

    // ───────────── the compose store (§10.1 step 4) ─────────────

    /// Writes the draft to this device's compose store (every edit). Blank drafts are not kept.
    pub fn save_local_draft(&self, draft: &Draft) {
        let mut store = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if draft.is_blank() && draft.base_version == 0 {
            let _ = store.delete_local_draft(&draft.client_draft_id);
            return;
        }
        let mut d = draft.clone();
        d.updated_at = now_ms();
        if let Ok(doc) = serde_json::to_value(&d) {
            let _ = store.put_local_draft(&d.client_draft_id, &doc);
        }
    }

    /// Deletes the device copy (sent or discarded).
    pub fn delete_local_draft(&self, client_draft_id: &str) {
        let _ = self.store.lock().unwrap_or_else(|e| e.into_inner()).delete_local_draft(client_draft_id);
    }

    /// Drafts kept on this device, newest first (in-flight uploads marked to attach again).
    pub fn local_drafts(&self) -> Vec<Draft> {
        let rows = self.store.lock().unwrap_or_else(|e| e.into_inner()).local_drafts().unwrap_or_default();
        rows.into_iter().filter_map(|r| serde_json::from_value::<Draft>(r.doc).ok()).map(Draft::restored).collect()
    }

    /// Discards a draft: the device copy goes at once, and a provider draft the autosave created
    /// is deleted through `drafts.discard` (D-281: an empty composer never leaves one behind).
    pub fn discard_draft(self: &Arc<Self>, draft: &Draft) {
        self.delete_local_draft(&draft.client_draft_id);
        if draft.base_version == 0 {
            return;
        }
        let Some(r) = self.remote() else { return };
        let args = DraftDiscardArgs {
            client_op_id: uuid::Uuid::now_v7().to_string(),
            client_draft_id: draft.client_draft_id.clone(),
        };
        let me = self.clone();
        self.rt.spawn(async move {
            if let Err(e) = r.api.discard_draft(&args).await
                && e.code() != Some(akatsuki_core::ErrorCode::NotFound)
            {
                me.toast(safe(&e), false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn failure_toasts_name_the_action_subject_and_account() {
        let f: OpFailure = serde_json::from_value(json!({
            "_id": "o1", "threadId": "t1", "subject": "Board deck", "accountEmail": "me@x.example",
            "payloadKind": "thread", "error": {"code": "not_found", "message": "The message no longer exists.", "retryable": false},
            "state": "failed", "at": 1
        }))
        .unwrap();
        assert_eq!(failure_text(&f), "Couldn't update 'Board deck' in me@x.example: The message no longer exists");
        let mut g = f.clone();
        g.subject = None;
        g.payload_kind = "label_rename".into();
        assert!(failure_text(&g).starts_with("Couldn't rename the label in me@x.example"));
        g.subject = Some("x".repeat(80));
        g.payload_kind = "thread".into();
        assert!(failure_text(&g).contains(&format!("'{}…'", "x".repeat(60))));
    }
}
