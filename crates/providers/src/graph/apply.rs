//! `MailProvider::apply` for Graph (ARCHITECTURE §5.1 helper table, §5.4, §9.5 "Ops").
//!
//! 1. Every thread the ops touch is read fresh (one `$batch` per 20 conversations).
//! 2. Each op becomes per-message requests:
//!    - `modify`: one `PATCH` per message carrying the net `isRead`, `flag` and `categories`
//!      change. Marking unread or starring touches the newest message only; marking read or
//!      unstarring touches every message that has the state (thread semantics, D-214). A
//!      `modify` that adds a folder key is a move; removing a folder key alone is invalid.
//!    - `move`: `POST /messages/{id}/move` for each message not already there. Sent items and
//!      drafts stay where they are unless the target is Trash or Junk.
//!    - `trash`: every message to Deleted Items; `untrash`: back to Drafts, Sent Items or Inbox
//!      by what the message is; `delete_forever`: `DELETE` per message.
//!    - Label ops act on master categories (`cat:<name>`) or folders.
//! 3. All message requests go out together, 20 per `$batch`. A message that vanished meanwhile
//!    (404) is skipped; if every request of an op is 404 the op is `NotFound` (moot).
//! 4. Threads of successful ops are read again so `post` carries the provider's truth.

use std::collections::{BTreeSet, HashMap};

use reqwest::Method;
use serde_json::{Value, json};

use crate::{LabelKind, LabelTarget, OpEffect, OpId, ProviderError, ProviderOp, RemoteLabel, RemoteThreadId, SysLabel};

use super::client::{Body, Sub, enc};
use super::map::{self, category_label_id, category_name, nearest_preset};
use super::model::Message;
use super::{FolderSet, GraphProvider};

type OpResult = Result<OpEffect, ProviderError>;
/// Fresh messages by conversation, and the folder set.
type PreState = (HashMap<String, Vec<Message>>, std::sync::Arc<FolderSet>);

enum Plan {
    /// Message requests; the thread is re-read afterwards (`refetch`).
    Requests { subs: Vec<Sub>, thread: Option<String>, refetch: bool },
    /// Already decided.
    Done(OpResult),
}

fn thread_of(op: &ProviderOp) -> Option<&str> {
    match op {
        ProviderOp::Modify { thread, .. }
        | ProviderOp::Trash { thread }
        | ProviderOp::Untrash { thread }
        | ProviderOp::DeleteForever { thread }
        | ProviderOp::Move { thread, .. } => Some(&thread.provider_thread_id),
        _ => None,
    }
}

fn msg_url(id: &str) -> String {
    format!("/me/messages/{}", enc(id))
}

fn newest(msgs: &[&Message], pred: impl Fn(&Message) -> bool) -> Option<String> {
    msgs.iter()
        .filter(|m| pred(m))
        .max_by_key(|m| {
            m.received_date_time.as_deref().or(m.sent_date_time.as_deref()).and_then(map::parse_time).unwrap_or(0)
        })
        .map(|m| m.id.clone())
}

/// A move destination: the `destinationId` to send and the folder id to compare with.
struct Dest {
    destination: String,
    folder_id: Option<String>,
    /// Trash and Junk take every message, sent items and drafts included.
    takes_all: bool,
}

async fn resolve_dest(p: &GraphProvider, fs: &FolderSet, to: &LabelTarget) -> Result<Dest, ProviderError> {
    let wk = |name: &str, takes_all: bool| Dest {
        destination: name.to_owned(),
        folder_id: fs.wk(name).map(str::to_owned),
        takes_all,
    };
    Ok(match to {
        LabelTarget::Sys(SysLabel::Inbox) => wk("inbox", false),
        LabelTarget::Sys(SysLabel::Sent) => wk("sentitems", false),
        LabelTarget::Sys(SysLabel::Drafts) => wk("drafts", false),
        LabelTarget::Sys(SysLabel::Trash) => wk("deleteditems", true),
        LabelTarget::Sys(SysLabel::Spam) => wk("junkemail", true),
        LabelTarget::Sys(SysLabel::Snoozed) => {
            let id = p.snoozed_folder().await?;
            Dest { destination: id.clone(), folder_id: Some(id), takes_all: false }
        }
        LabelTarget::Label { provider_label_id, exclusive: true } if category_name(provider_label_id).is_none() => {
            Dest {
                destination: provider_label_id.clone(),
                folder_id: Some(provider_label_id.clone()),
                takes_all: false,
            }
        }
        _ => return Err(ProviderError::Invalid("graph: move target is not a folder".into())),
    })
}

fn is_folder_target(t: &LabelTarget) -> bool {
    match t {
        LabelTarget::Sys(s) => {
            matches!(
                s,
                SysLabel::Inbox
                    | SysLabel::Sent
                    | SysLabel::Drafts
                    | SysLabel::Trash
                    | SysLabel::Spam
                    | SysLabel::Snoozed
            )
        }
        LabelTarget::Label { provider_label_id, exclusive } => *exclusive && category_name(provider_label_id).is_none(),
    }
}

fn move_subs(msgs: &[&Message], fs: &FolderSet, dest: &Dest) -> Vec<Sub> {
    let sent = fs.wk("sentitems");
    let drafts = fs.wk("drafts");
    msgs.iter()
        .filter(|m| dest.folder_id.is_none() || m.parent_folder_id.as_deref() != dest.folder_id.as_deref())
        .filter(|m| {
            dest.takes_all
                || !(m.draft()
                    || (sent.is_some() && m.parent_folder_id.as_deref() == sent)
                    || (drafts.is_some() && m.parent_folder_id.as_deref() == drafts))
        })
        .map(|m| Sub::post(format!("{}/move", msg_url(&m.id)), json!({ "destinationId": dest.destination })))
        .collect()
}

fn modify_subs(msgs: &[&Message], add: &[LabelTarget], remove: &[LabelTarget]) -> Result<Vec<Sub>, ProviderError> {
    let has = |list: &[LabelTarget], s: SysLabel| list.contains(&LabelTarget::Sys(s));
    if add.iter().chain(remove).any(|t| *t == LabelTarget::Sys(SysLabel::Important)) {
        return Err(ProviderError::Unsupported("graph: important"));
    }
    let cats = |list: &[LabelTarget]| -> Vec<String> {
        list.iter()
            .filter_map(|t| match t {
                LabelTarget::Label { provider_label_id, .. } => category_name(provider_label_id).map(str::to_owned),
                LabelTarget::Sys(_) => None,
            })
            .collect()
    };
    let (cat_add, cat_remove) = (cats(add), cats(remove));

    let mut patches: HashMap<String, serde_json::Map<String, Value>> = HashMap::new();
    let mut set = |id: String, k: &str, v: Value| {
        patches.entry(id).or_default().insert(k.to_owned(), v);
    };
    if has(remove, SysLabel::Unread) {
        for m in msgs.iter().filter(|m| m.unread()) {
            set(m.id.clone(), "isRead", json!(true));
        }
    } else if has(add, SysLabel::Unread)
        && !msgs.iter().any(|m| m.unread() && !m.draft())
        && let Some(id) = newest(msgs, |m| !m.draft())
    {
        set(id, "isRead", json!(false));
    }
    if has(remove, SysLabel::Starred) {
        for m in msgs.iter().filter(|m| m.flagged()) {
            set(m.id.clone(), "flag", json!({ "flagStatus": "notFlagged" }));
        }
    } else if has(add, SysLabel::Starred)
        && !msgs.iter().any(|m| m.flagged())
        && let Some(id) = newest(msgs, |m| !m.draft()).or_else(|| newest(msgs, |_| true))
    {
        set(id, "flag", json!({ "flagStatus": "flagged" }));
    }
    if !cat_add.is_empty() || !cat_remove.is_empty() {
        for m in msgs {
            let current: Vec<String> = m.category_names().to_vec();
            let mut next: Vec<String> = current.iter().filter(|c| !cat_remove.contains(c)).cloned().collect();
            for c in &cat_add {
                if !next.contains(c) {
                    next.push(c.clone());
                }
            }
            if next != current {
                set(m.id.clone(), "categories", json!(next));
            }
        }
    }
    let mut ids: Vec<_> = patches.into_iter().collect();
    ids.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(ids.into_iter().map(|(id, body)| Sub::patch(msg_url(&id), Value::Object(body))).collect())
}

fn untrash_subs(p: &GraphProvider, msgs: &[&Message], fs: &FolderSet) -> Vec<Sub> {
    let Some(trash) = fs.wk("deleteditems") else { return Vec::new() };
    msgs.iter()
        .filter(|m| m.parent_folder_id.as_deref() == Some(trash))
        .map(|m| {
            let from_self = m
                .from
                .as_ref()
                .and_then(|r| r.email_address.as_ref())
                .and_then(|e| e.address.as_deref())
                .is_some_and(|a| p.cfg.self_addresses.iter().any(|s| s.eq_ignore_ascii_case(a.trim())));
            let dest = if m.draft() {
                "drafts"
            } else if from_self {
                "sentitems"
            } else {
                "inbox"
            };
            Sub::post(format!("{}/move", msg_url(&m.id)), json!({ "destinationId": dest }))
        })
        .collect()
}

async fn plan_thread_op(
    p: &GraphProvider,
    op: &ProviderOp,
    msgs: &[&Message],
    fs: &FolderSet,
) -> Result<(Vec<Sub>, bool), ProviderError> {
    Ok(match op {
        ProviderOp::Modify { add, remove, .. } => {
            if let Some(target) = add.iter().find(|t| is_folder_target(t)) {
                let dest = resolve_dest(p, fs, target).await?;
                let mut subs = move_subs(msgs, fs, &dest);
                let rest_add: Vec<LabelTarget> = add.iter().filter(|t| !is_folder_target(t)).cloned().collect();
                let rest_remove: Vec<LabelTarget> = remove.iter().filter(|t| !is_folder_target(t)).cloned().collect();
                subs.extend(modify_subs(msgs, &rest_add, &rest_remove)?);
                (subs, true)
            } else if remove.iter().any(is_folder_target) {
                return Err(ProviderError::Invalid("graph: removing a folder needs a move".into()));
            } else {
                (modify_subs(msgs, add, remove)?, true)
            }
        }
        ProviderOp::Move { to, .. } => {
            let dest = resolve_dest(p, fs, to).await?;
            (move_subs(msgs, fs, &dest), true)
        }
        ProviderOp::Trash { .. } => {
            let dest = resolve_dest(p, fs, &LabelTarget::Sys(SysLabel::Trash)).await?;
            (move_subs(msgs, fs, &dest), true)
        }
        ProviderOp::Untrash { .. } => (untrash_subs(p, msgs, fs), true),
        ProviderOp::DeleteForever { .. } => (msgs.iter().map(|m| Sub::delete(msg_url(&m.id))).collect(), false),
        _ => (Vec::new(), false),
    })
}

async fn label_op(p: &GraphProvider, op: &ProviderOp) -> OpResult {
    let r = match op {
        ProviderOp::LabelCreate { name, color } => {
            let mut body = json!({ "displayName": name });
            if let Some(preset) = color.as_deref().and_then(nearest_preset) {
                body["color"] = json!(preset);
            }
            let url = p.http.url("/me/outlook/masterCategories");
            let label = match p.http.send(Method::POST, &url, None, Body::Json(body)).await {
                Ok(resp) => map::category_label(&resp.json()?),
                // Already exists (Graph answers 409, some tenants 400 ErrorDuplicate…): adopt it.
                Err(ProviderError::Conflict(_)) => p
                    .categories()
                    .await?
                    .iter()
                    .find(|c| c.display_name.eq_ignore_ascii_case(name))
                    .map(map::category_label)
                    .unwrap_or_else(|| RemoteLabel {
                        provider_label_id: category_label_id(name),
                        name: name.clone(),
                        kind: LabelKind::Category,
                        role: None,
                        exclusive: false,
                        color: None,
                        hidden: false,
                    }),
                Err(e) => return Err(e),
            };
            Ok(OpEffect::Applied { post: None, created: Some(label) })
        }
        ProviderOp::LabelRename { provider_label_id, name } => {
            if category_name(provider_label_id).is_some() {
                return Err(ProviderError::Unsupported("graph: categories cannot be renamed"));
            }
            let url = p.http.url(&format!("/me/mailFolders/{}", enc(provider_label_id)));
            p.http.send(Method::PATCH, &url, None, Body::Json(json!({ "displayName": name }))).await?;
            Ok(OpEffect::Applied { post: None, created: None })
        }
        ProviderOp::LabelDelete { provider_label_id } => {
            let url = match category_name(provider_label_id) {
                Some(cat) => {
                    let found = p.categories().await?.into_iter().find(|c| c.display_name == cat);
                    match found {
                        Some(c) => p.http.url(&format!("/me/outlook/masterCategories/{}", enc(&c.id))),
                        None => return Ok(OpEffect::Moot),
                    }
                }
                None => p.http.url(&format!("/me/mailFolders/{}", enc(provider_label_id))),
            };
            match p.http.send(Method::DELETE, &url, None, Body::Empty).await {
                Ok(_) => Ok(OpEffect::Applied { post: None, created: None }),
                Err(ProviderError::NotFound) => Ok(OpEffect::Moot),
                Err(e) => Err(e),
            }
        }
        _ => Err(ProviderError::Invalid("graph: not a label op".into())),
    };
    p.invalidate_folders().await;
    r
}

pub(super) async fn apply(p: &GraphProvider, ops: &[(OpId, ProviderOp)]) -> Vec<(OpId, OpResult)> {
    // 1. Fresh state of every touched thread, and the folder set.
    let thread_ids: Vec<String> = ops
        .iter()
        .filter_map(|(_, op)| thread_of(op))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let pre: Result<PreState, ProviderError> = async {
        if thread_ids.is_empty() {
            return Ok((HashMap::new(), p.folder_set(false).await?));
        }
        let fs = p.folder_set(false).await?;
        let mut by: HashMap<String, Vec<Message>> = HashMap::new();
        for m in p.fetch_conversations(&thread_ids).await? {
            if let Some(c) = m.conversation_id.clone() {
                by.entry(c).or_default().push(m);
            }
        }
        Ok((by, fs))
    }
    .await;

    // 2. Plan.
    let mut plans = Vec::with_capacity(ops.len());
    for (_, op) in ops {
        let plan = match (op, &pre) {
            (ProviderOp::LabelCreate { .. } | ProviderOp::LabelRename { .. } | ProviderOp::LabelDelete { .. }, _) => {
                Plan::Done(label_op(p, op).await)
            }
            (ProviderOp::DeleteDraft { draft }, _) => Plan::Requests {
                subs: vec![Sub::delete(msg_url(&draft.provider_draft_id))],
                thread: None,
                refetch: false,
            },
            (_, Err(e)) => Plan::Done(Err(e.clone())),
            (_, Ok((by, fs))) => {
                let tid = thread_of(op).unwrap_or_default();
                let msgs: Vec<&Message> = by.get(tid).map(|v| v.iter().collect()).unwrap_or_default();
                if msgs.is_empty() {
                    Plan::Done(Err(ProviderError::NotFound))
                } else {
                    match plan_thread_op(p, op, &msgs, fs).await {
                        Ok((subs, refetch)) => Plan::Requests { subs, thread: Some(tid.to_owned()), refetch },
                        Err(e) => Plan::Done(Err(e)),
                    }
                }
            }
        };
        plans.push(plan);
    }

    // 3. Execute every message request together.
    let mut all = Vec::new();
    let mut owner = Vec::new();
    for (i, plan) in plans.iter().enumerate() {
        if let Plan::Requests { subs, .. } = plan {
            for s in subs {
                all.push(s.clone());
                owner.push(i);
            }
        }
    }
    let responses = if all.is_empty() { Ok(Vec::new()) } else { p.http.batch(&all).await };
    let mut errors: HashMap<usize, ProviderError> = HashMap::new();
    let mut not_found: HashMap<usize, (usize, usize)> = HashMap::new(); // op → (404s, total)
    match &responses {
        Ok(rs) => {
            for (i, r) in owner.iter().zip(rs) {
                let e = not_found.entry(*i).or_default();
                e.1 += 1;
                if r.ok() {
                    continue;
                }
                match r.error() {
                    ProviderError::NotFound => e.0 += 1,
                    err => {
                        errors.entry(*i).or_insert(err);
                    }
                }
            }
        }
        Err(err) => {
            for i in &owner {
                errors.entry(*i).or_insert_with(|| err.clone());
            }
        }
    }

    // 4. Re-read the threads of successful ops.
    let refetch: Vec<RemoteThreadId> = plans
        .iter()
        .enumerate()
        .filter_map(|(i, plan)| match plan {
            Plan::Requests { thread: Some(t), refetch: true, .. } if !errors.contains_key(&i) => Some(t.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(RemoteThreadId)
        .collect();
    let post: HashMap<String, crate::FetchedThread> = if refetch.is_empty() {
        HashMap::new()
    } else {
        let order: Vec<String> = refetch.iter().map(|t| t.0.clone()).collect();
        match p.fetch_conversations(&order).await {
            Ok(msgs) => map::threads(&order, msgs).into_iter().map(|t| (t.provider_thread_id.clone(), t)).collect(),
            Err(_) => HashMap::new(), // the op still succeeded; the next delta brings the state
        }
    };

    ops.iter()
        .zip(plans)
        .enumerate()
        .map(|(i, ((id, _), plan))| {
            let result = match plan {
                Plan::Done(r) => r,
                Plan::Requests { thread, .. } => {
                    if let Some(e) = errors.remove(&i) {
                        Err(e)
                    } else if not_found.get(&i).is_some_and(|(nf, total)| *total > 0 && nf == total) {
                        Err(ProviderError::NotFound)
                    } else {
                        Ok(OpEffect::Applied { post: thread.and_then(|t| post.get(&t).cloned()), created: None })
                    }
                }
            };
            (id.clone(), result)
        })
        .collect()
}
