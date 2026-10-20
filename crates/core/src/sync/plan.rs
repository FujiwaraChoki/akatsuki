//! Planning one claim batch (ARCHITECTURE §5.4): coalescing and moot detection.
//!
//! Within one claim batch the account actor may coalesce consecutive `modify` ops on the same
//! thread into one net delta. A net-zero delta (relative to the provider-confirmed state the claim
//! returned) skips the provider and completes as `moot`. Coalescing never crosses a non-`modify`
//! op on the same thread, and every op is still reported individually.

use std::collections::{BTreeMap, BTreeSet};

use crate::convex::schema::{OpPayload, ThreadAction};
use crate::convex::worker::{ClaimedOp, ClaimedThread};

/// What to do with one or more claimed ops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlannedStep {
    /// Run one op as claimed (non-modify thread ops, sends, drafts, labels, unsubscribe).
    Single(usize),
    /// One provider `modify` that stands for every op in `ops` (indices into the claim batch).
    Modify {
        /// Claimed ops covered, in claim order.
        ops: Vec<usize>,
        /// Label keys to add (only those some message lacks).
        add: Vec<String>,
        /// Label keys to remove (only those some message has).
        remove: Vec<String>,
    },
    /// Nothing to do at the provider; report every op as `moot`.
    Moot(Vec<usize>),
}

/// Composes two `(remove, add)` deltas: `(S − r1) ∪ a1` then `(… − r2) ∪ a2` equals
/// `(S − (r1 ∪ r2)) ∪ ((a1 − r2) ∪ a2)`.
fn compose(acc: &mut (BTreeSet<String>, BTreeSet<String>), remove: &[String], add: &[String]) {
    let (r, a) = acc;
    for k in remove {
        a.remove(k);
        r.insert(k.clone());
    }
    for k in add {
        a.insert(k.clone());
    }
}

/// Per-message keys of a claimed thread; falls back to the thread's synced keys when the claim
/// carried no messages.
fn message_keys(thread: Option<&ClaimedThread>) -> Vec<BTreeSet<String>> {
    match thread {
        Some(t) if !t.messages.is_empty() => {
            t.messages.iter().map(|m| m.label_keys.iter().cloned().collect()).collect()
        }
        Some(t) => vec![t.synced_label_keys.iter().cloned().collect()],
        None => vec![],
    }
}

/// Reduces a net delta to what the provider must still do: add a key only if some message lacks
/// it, remove a key only if some message has it (Gmail and Graph apply labels per message).
pub fn minimize(
    remove: &BTreeSet<String>,
    add: &BTreeSet<String>,
    messages: &[BTreeSet<String>],
) -> (Vec<String>, Vec<String>) {
    let add_needed: Vec<String> =
        add.iter().filter(|k| messages.is_empty() || messages.iter().any(|m| !m.contains(*k))).cloned().collect();
    let remove_needed: Vec<String> = remove
        .iter()
        .filter(|k| !add.contains(*k))
        .filter(|k| messages.is_empty() || messages.iter().any(|m| m.contains(*k)))
        .cloned()
        .collect();
    (add_needed, remove_needed)
}

/// Plans a claim batch. Ops come back in claim (creation) order; the plan keeps that order per
/// thread, so FIFO per thread holds.
pub fn plan(claimed: &[ClaimedOp]) -> Vec<PlannedStep> {
    // Open modify run per thread: (indices, (remove, add)).
    type Run = (Vec<usize>, (BTreeSet<String>, BTreeSet<String>));
    let mut runs: BTreeMap<String, Run> = BTreeMap::new();
    // Steps in output order; a run's slot is reserved at its first op and filled when it closes.
    let mut steps: Vec<Option<PlannedStep>> = Vec::new();
    let mut run_slot: BTreeMap<String, usize> = BTreeMap::new();

    let close = |thread: &str,
                 runs: &mut BTreeMap<String, Run>,
                 run_slot: &mut BTreeMap<String, usize>,
                 steps: &mut Vec<Option<PlannedStep>>| {
        if let (Some((ops, (r, a))), Some(slot)) = (runs.remove(thread), run_slot.remove(thread)) {
            let first = ops[0];
            let keys = message_keys(claimed[first].thread.as_ref());
            let (add, remove) = minimize(&r, &a, &keys);
            steps[slot] = Some(if add.is_empty() && remove.is_empty() {
                PlannedStep::Moot(ops)
            } else {
                PlannedStep::Modify { ops, add, remove }
            });
        }
    };

    for (i, c) in claimed.iter().enumerate() {
        let thread_key = c.thread.as_ref().map(|t| t.provider_thread_id.clone());
        match (&c.op.payload, thread_key) {
            (OpPayload::Thread { action: ThreadAction::Modify { add, remove } }, Some(t)) if !c.op.verify_only => {
                if !runs.contains_key(&t) {
                    run_slot.insert(t.clone(), steps.len());
                    steps.push(None);
                    runs.insert(t.clone(), (Vec::new(), (BTreeSet::new(), BTreeSet::new())));
                }
                if let Some((ops, acc)) = runs.get_mut(&t) {
                    ops.push(i);
                    compose(acc, remove, add);
                }
            }
            (_, Some(t)) => {
                close(&t, &mut runs, &mut run_slot, &mut steps);
                steps.push(Some(PlannedStep::Single(i)));
            }
            (_, None) => steps.push(Some(PlannedStep::Single(i))),
        }
    }
    let open: Vec<String> = runs.keys().cloned().collect();
    for t in open {
        close(&t, &mut runs, &mut run_slot, &mut steps);
    }
    steps.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convex::worker::{ClaimedMessage, ClaimedOpCore};

    fn op(id: &str, thread: &str, action: ThreadAction, msgs: &[&[&str]]) -> ClaimedOp {
        ClaimedOp {
            op: ClaimedOpCore {
                id: id.into(),
                client_op_id: format!("c-{id}"),
                payload: OpPayload::Thread { action },
                attempt: 1,
                verify_only: false,
                send_elapsed_ms: None,
            },
            thread: Some(ClaimedThread {
                provider_thread_id: thread.into(),
                synced_label_keys: vec![],
                messages: msgs
                    .iter()
                    .enumerate()
                    .map(|(n, keys)| ClaimedMessage {
                        provider_message_id: format!("{thread}-{n}"),
                        label_keys: keys.iter().map(|k| (*k).to_owned()).collect(),
                        imap_locations: None,
                    })
                    .collect(),
            }),
            labels: Default::default(),
            send: None,
            draft: None,
        }
    }

    fn modify(add: &[&str], remove: &[&str]) -> ThreadAction {
        ThreadAction::Modify {
            add: add.iter().map(|s| (*s).to_owned()).collect(),
            remove: remove.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn opposing_modifies_cancel_out_as_moot() {
        let inbox: &[&[&str]] = &[&["sys:inbox"]];
        let ops = vec![
            op("o1", "t1", modify(&[], &["sys:inbox"]), inbox),
            op("o2", "t1", modify(&["sys:inbox"], &[]), inbox),
        ];
        assert_eq!(plan(&ops), vec![PlannedStep::Moot(vec![0, 1])]);
    }

    #[test]
    fn consecutive_modifies_coalesce_into_net_delta() {
        let m: &[&[&str]] = &[&["sys:inbox", "sys:unread"], &["sys:inbox"]];
        let ops = vec![
            op("o1", "t1", modify(&[], &["sys:unread"]), m),
            op("o2", "t1", modify(&["sys:starred"], &["sys:inbox"]), m),
            op("o3", "t1", modify(&[], &["sys:starred"]), m),
        ];
        assert_eq!(
            plan(&ops),
            vec![PlannedStep::Modify {
                ops: vec![0, 1, 2],
                add: vec![],
                remove: vec!["sys:inbox".into(), "sys:unread".into()]
            }]
        );
    }

    #[test]
    fn adding_a_label_some_messages_lack_is_not_moot() {
        let m: &[&[&str]] = &[&["l:a"], &[]];
        assert_eq!(
            plan(&[op("o1", "t1", modify(&["l:a"], &[]), m)]),
            vec![PlannedStep::Modify { ops: vec![0], add: vec!["l:a".into()], remove: vec![] }]
        );
        let all: &[&[&str]] = &[&["l:a"], &["l:a"]];
        assert_eq!(plan(&[op("o1", "t1", modify(&["l:a"], &[]), all)]), vec![PlannedStep::Moot(vec![0])]);
    }

    #[test]
    fn coalescing_never_crosses_other_actions_and_keeps_thread_order() {
        let m: &[&[&str]] = &[&["sys:inbox"]];
        let ops = vec![
            op("o1", "t1", modify(&["sys:starred"], &[]), m),
            op("o2", "t2", modify(&[], &["sys:inbox"]), m),
            op("o3", "t1", ThreadAction::Trash, m),
            op("o4", "t1", modify(&["l:x"], &[]), m),
        ];
        assert_eq!(
            plan(&ops),
            vec![
                PlannedStep::Modify { ops: vec![0], add: vec!["sys:starred".into()], remove: vec![] },
                PlannedStep::Modify { ops: vec![1], add: vec![], remove: vec!["sys:inbox".into()] },
                PlannedStep::Single(2),
                PlannedStep::Modify { ops: vec![3], add: vec!["l:x".into()], remove: vec![] },
            ]
        );
    }

    #[test]
    fn minimize_drops_remove_of_readded_key() {
        let r: BTreeSet<String> = ["a".to_owned()].into();
        let a: BTreeSet<String> = ["a".to_owned()].into();
        let msgs = vec![BTreeSet::new()];
        assert_eq!(minimize(&r, &a, &msgs), (vec!["a".to_owned()], vec![]));
    }
}
