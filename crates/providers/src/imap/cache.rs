//! In-memory thread index.
//!
//! IMAP has no thread lookup (except Gmail's `X-GM-THRID`): a computed thread id (§9.5) can only be
//! resolved from headers already fetched. `list_window`, `changes_since` and `search` fetch the
//! headers of every message they report, so they record the messages here and `fetch_threads`
//! answers from this index. It is bounded and lives only in worker memory (§9.2: the worker keeps
//! no durable state), which is enough because the engine always calls `fetch_threads` right after
//! the listing that produced the ids.

use std::collections::{HashMap, VecDeque};

use crate::RemoteMessage;

/// Messages kept before the oldest threads are evicted.
pub const DEFAULT_CAPACITY: usize = 50_000;

/// Thread id → messages seen in this process.
#[derive(Debug)]
pub struct ThreadCache {
    threads: HashMap<String, Vec<RemoteMessage>>,
    order: VecDeque<String>,
    messages: usize,
    capacity: usize,
}

impl Default for ThreadCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl ThreadCache {
    /// A cache holding up to `capacity` messages.
    pub fn with_capacity(capacity: usize) -> Self {
        Self { threads: HashMap::new(), order: VecDeque::new(), messages: 0, capacity: capacity.max(1) }
    }

    /// Records a message (replacing an older copy with the same provider id).
    pub fn insert(&mut self, m: RemoteMessage) {
        let tid = m.provider_thread_id.clone();
        let entry = self.threads.entry(tid.clone()).or_insert_with(|| {
            self.order.push_back(tid.clone());
            Vec::new()
        });
        if let Some(existing) = entry.iter_mut().find(|x| x.provider_message_id == m.provider_message_id) {
            *existing = m;
        } else {
            entry.push(m);
            self.messages += 1;
        }
        while self.messages > self.capacity && self.order.len() > 1 {
            let Some(old) = self.order.pop_front() else { break };
            if old == tid {
                // Never evict the thread just written.
                self.order.push_back(old);
                continue;
            }
            if let Some(v) = self.threads.remove(&old) {
                self.messages -= v.len();
            }
        }
    }

    /// Drops messages by provider id (expunged, moved).
    pub fn remove_messages(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        for v in self.threads.values_mut() {
            let before = v.len();
            v.retain(|m| !ids.contains(&m.provider_message_id));
            self.messages -= before - v.len();
        }
    }

    /// Drops messages of `folder` recorded under another UIDVALIDITY.
    pub fn retain_folder_validity(&mut self, folder: &str, uid_validity: u32) {
        for v in self.threads.values_mut() {
            let before = v.len();
            v.retain(|m| !m.imap.iter().any(|l| l.remote_folder_id == folder && l.uid_validity != uid_validity));
            self.messages -= before - v.len();
        }
    }

    /// The messages of a thread, oldest first.
    pub fn get(&self, thread_id: &str) -> Option<Vec<RemoteMessage>> {
        let mut v = self.threads.get(thread_id)?.clone();
        if v.is_empty() {
            return None;
        }
        v.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.provider_message_id.cmp(&b.provider_message_id)));
        Some(v)
    }

    /// Number of cached messages.
    pub fn len(&self) -> usize {
        self.messages
    }

    /// No messages cached.
    pub fn is_empty(&self) -> bool {
        self.messages == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(id: &str, thread: &str, date: i64) -> RemoteMessage {
        RemoteMessage { provider_message_id: id.into(), provider_thread_id: thread.into(), date, ..Default::default() }
    }

    #[test]
    fn insert_replace_evict() {
        let mut c = ThreadCache::with_capacity(3);
        c.insert(msg("a", "t1", 2));
        c.insert(msg("b", "t1", 1));
        c.insert(msg("a", "t1", 3));
        assert_eq!(c.len(), 2);
        assert_eq!(c.get("t1").unwrap().iter().map(|m| m.date).collect::<Vec<_>>(), vec![1, 3]);
        c.insert(msg("c", "t2", 1));
        c.insert(msg("d", "t3", 1));
        // Over capacity: the oldest thread (t1) goes.
        assert!(c.get("t1").is_none());
        assert!(c.get("t2").is_some() && c.get("t3").is_some());
        let mut stale = msg("e", "t3", 1);
        stale.imap = vec![crate::ImapLocation { remote_folder_id: "INBOX".into(), uid_validity: 1, uid: 1 }];
        c.insert(stale);
        c.retain_folder_validity("INBOX", 2);
        assert_eq!(c.get("t3").unwrap().len(), 1);
        c.remove_messages(&["c".into()]);
        assert!(c.get("t2").is_none());
        assert_eq!(c.len(), 1);
    }
}
