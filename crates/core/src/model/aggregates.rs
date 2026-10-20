//! Thread aggregates derived from message metadata (ARCHITECTURE §4.4, step 3).

use std::collections::BTreeSet;

use crate::convex::MessageUpsert;

/// `threadKeysFromMessages`: the union of the messages' label keys, where `sys:unread` means at
/// least one message is unread. Sorted and de-duplicated.
pub fn thread_keys_from_messages(messages: &[MessageUpsert]) -> Vec<String> {
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for m in messages {
        keys.extend(m.label_keys.iter().filter(|k| k.as_str() != "sys:unread").cloned());
    }
    if messages.iter().any(|m| m.is_unread) {
        keys.insert("sys:unread".to_owned());
    }
    keys.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(keys: &[&str], unread: bool) -> MessageUpsert {
        MessageUpsert {
            label_keys: keys.iter().map(|s| (*s).to_owned()).collect(),
            is_unread: unread,
            ..MessageUpsert::default()
        }
    }

    #[test]
    fn unread_comes_from_flags() {
        let keys = thread_keys_from_messages(&[msg(&["sys:inbox", "sys:unread"], false), msg(&["l:x"], true)]);
        assert_eq!(keys, vec!["l:x", "sys:inbox", "sys:unread"]);
        let keys = thread_keys_from_messages(&[msg(&["sys:inbox"], false)]);
        assert_eq!(keys, vec!["sys:inbox"]);
    }
}
