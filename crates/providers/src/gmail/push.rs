//! Gmail Pub/Sub push payloads (§9.6). `POST /hooks/gmail` in `apps/sync` verifies the
//! Google-signed OIDC JWT and then calls [`decode_push`]; the result goes to the account actor as
//! `ActorCmd::PushHint`. A hint whose `historyId` is not newer than the cursor is dropped
//! ([`GmailPush::is_newer_than`]).

use serde::Deserialize;

use super::convert::decode_b64;

/// The Pub/Sub push envelope.
#[derive(Debug, Deserialize)]
struct Envelope {
    message: PubSubMessage,
    #[serde(default)]
    subscription: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PubSubMessage {
    data: String,
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Notification {
    email_address: String,
    history_id: serde_json::Value,
}

/// A decoded Gmail notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GmailPush {
    /// The mailbox (lowercased), for `worker.accountsForPush({by: "gmail_email"})`.
    pub email_address: String,
    /// The mailbox's history id at notification time.
    pub history_id: u64,
    /// Pub/Sub message id (deduplication).
    pub message_id: Option<String>,
    /// Subscription name.
    pub subscription: Option<String>,
}

impl GmailPush {
    /// `true` when this push carries changes after `cursor`.
    pub fn is_newer_than(&self, cursor: u64) -> bool {
        self.history_id > cursor
    }
}

/// Decodes a push request body. Errors are short and contain no payload data.
pub fn decode_push(body: &[u8]) -> Result<GmailPush, &'static str> {
    let env: Envelope = serde_json::from_slice(body).map_err(|_| "not a Pub/Sub push envelope")?;
    let data = decode_b64(&env.message.data).map_err(|_| "data is not base64")?;
    let n: Notification = serde_json::from_slice(&data).map_err(|_| "data is not a Gmail notification")?;
    let history_id = match &n.history_id {
        serde_json::Value::Number(x) => x.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .ok_or("historyId is not a u64")?;
    Ok(GmailPush {
        email_address: n.email_address.trim().to_lowercase(),
        history_id,
        message_id: env.message.message_id,
        subscription: env.subscription,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn decodes_a_push() {
        let data = base64::engine::general_purpose::STANDARD
            .encode(br#"{"emailAddress":"Ada@Example.com","historyId":9876543210123}"#);
        let body = format!(
            r#"{{"message":{{"data":"{data}","messageId":"2070443601311540","publishTime":"2026-10-20T10:00:00Z"}},"subscription":"projects/akatsuki/subscriptions/gmail-push"}}"#
        );
        let p = decode_push(body.as_bytes()).unwrap();
        assert_eq!(p.email_address, "ada@example.com");
        assert_eq!(p.history_id, 9_876_543_210_123);
        assert_eq!(p.message_id.as_deref(), Some("2070443601311540"));
        assert!(p.is_newer_than(9_876_543_210_122));
        assert!(!p.is_newer_than(9_876_543_210_123));
        assert!(decode_push(b"{}").is_err());
        let bad = base64::engine::general_purpose::STANDARD.encode(br#"{"emailAddress":"a@b.c","historyId":"x"}"#);
        assert!(decode_push(format!(r#"{{"message":{{"data":"{bad}"}}}}"#).as_bytes()).is_err());
    }
}
