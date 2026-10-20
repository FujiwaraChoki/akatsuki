//! Graph change notifications (ARCHITECTURE §9.6): what `/hooks/graph` and
//! `/hooks/graph/lifecycle` in `apps/sync` need to authenticate and route a notification batch.
//!
//! 1. A request with `?validationToken=` is answered by echoing the token as `text/plain` within
//!    10 s (already done by the route).
//! 2. Otherwise the body is a batch: [`parse_notifications`].
//! 3. Each item is authenticated by its `clientState`: [`verify_client_state`] against the
//!    `watchClientStateHash` that `worker.accountsForPush({by:"graph_subscription"})` returns for
//!    the `subscriptionId`. Items that fail are dropped.
//! 4. Change notifications only wake the account actor (a delta pass reads the actual changes);
//!    lifecycle events map to an action through [`lifecycle_action`].

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::ProviderError;

/// A Graph lifecycle event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// The access token behind the subscription expired or consent changed.
    ReauthorizationRequired,
    /// Graph removed the subscription.
    SubscriptionRemoved,
    /// Graph dropped notifications.
    Missed,
    /// An event this version does not know.
    Other(String),
}

/// What the account actor does for a lifecycle event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleAction {
    /// `POST /subscriptions/{id}/reauthorize` ([`super::GraphProvider::reauthorize`]).
    Reauthorize,
    /// Create a new subscription (`watch(None, …)`), then run a delta on all folders.
    RecreateAndResync,
    /// Run a delta on all folders.
    Resync,
    /// Nothing.
    Ignore,
}

/// One item of a notification batch.
#[derive(Clone, PartialEq, Eq)]
pub struct GraphNotification {
    /// Subscription id (routes the item to an account).
    pub subscription_id: String,
    /// The secret set at creation; compare with [`verify_client_state`].
    pub client_state: Option<String>,
    /// `created`, `updated` or `deleted` (change notifications).
    pub change_type: Option<String>,
    /// `Users/{id}/Messages/{id}` (change notifications).
    pub resource: Option<String>,
    /// The lifecycle event (lifecycle notifications).
    pub lifecycle_event: Option<LifecycleEvent>,
    /// Tenant id.
    pub tenant_id: Option<String>,
}

impl std::fmt::Debug for GraphNotification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphNotification")
            .field("subscription_id", &self.subscription_id)
            .field("change_type", &self.change_type)
            .field("lifecycle_event", &self.lifecycle_event)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawItem {
    #[serde(default)]
    subscription_id: String,
    client_state: Option<String>,
    change_type: Option<String>,
    resource: Option<String>,
    lifecycle_event: Option<String>,
    tenant_id: Option<String>,
}

#[derive(Deserialize)]
struct RawBatch {
    #[serde(default)]
    value: Vec<RawItem>,
}

/// Parses a notification or lifecycle batch. Items without a subscription id are dropped.
pub fn parse_notifications(body: &[u8]) -> Result<Vec<GraphNotification>, ProviderError> {
    let batch: RawBatch = serde_json::from_slice(body)
        .map_err(|_| ProviderError::Invalid("graph notification: malformed body".into()))?;
    Ok(batch
        .value
        .into_iter()
        .filter(|i| !i.subscription_id.is_empty())
        .map(|i| GraphNotification {
            subscription_id: i.subscription_id,
            client_state: i.client_state,
            change_type: i.change_type,
            resource: i.resource,
            lifecycle_event: i.lifecycle_event.map(|e| match e.as_str() {
                "reauthorizationRequired" => LifecycleEvent::ReauthorizationRequired,
                "subscriptionRemoved" => LifecycleEvent::SubscriptionRemoved,
                "missed" => LifecycleEvent::Missed,
                _ => LifecycleEvent::Other(e),
            }),
            tenant_id: i.tenant_id,
        })
        .collect())
}

/// The §9.6 lifecycle table.
pub fn lifecycle_action(e: &LifecycleEvent) -> LifecycleAction {
    match e {
        LifecycleEvent::ReauthorizationRequired => LifecycleAction::Reauthorize,
        LifecycleEvent::SubscriptionRemoved => LifecycleAction::RecreateAndResync,
        LifecycleEvent::Missed => LifecycleAction::Resync,
        LifecycleEvent::Other(_) => LifecycleAction::Ignore,
    }
}

/// Lowercase hex sha256 of a `clientState` (what Convex stores as `watchClientStateHash`).
pub fn client_state_hash(client_state: &str) -> String {
    Sha256::digest(client_state.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether a received `clientState` matches the stored hash (constant-time comparison).
pub fn verify_client_state(received: Option<&str>, expected_hash: &str) -> bool {
    let Some(cs) = received else { return false };
    let got = client_state_hash(cs);
    let want = expected_hash.trim().to_ascii_lowercase();
    if got.len() != want.len() {
        return false;
    }
    got.bytes().zip(want.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

/// A fresh `clientState`: 32 random bytes, base64url without padding (§9.6). The bytes come from
/// two v4 UUIDs (the OS random source through `uuid`/`getrandom`), 244 random bits in all.
pub fn new_client_state() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_change_and_lifecycle_batches() {
        let body = br##"{"value":[
          {"subscriptionId":"sub-1","subscriptionExpirationDateTime":"2026-10-23T10:00:00Z","changeType":"updated",
           "resource":"Users/u1/Messages/m1","resourceData":{"@odata.type":"#Microsoft.Graph.Message","id":"m1"},
           "clientState":"cs","tenantId":"t1"},
          {"subscriptionId":"sub-1","lifecycleEvent":"reauthorizationRequired","clientState":"cs"},
          {"subscriptionId":"sub-1","lifecycleEvent":"somethingNew"},
          {"changeType":"created"}
        ]}"##;
        let n = parse_notifications(body).unwrap();
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].change_type.as_deref(), Some("updated"));
        assert_eq!(n[0].resource.as_deref(), Some("Users/u1/Messages/m1"));
        assert_eq!(n[1].lifecycle_event, Some(LifecycleEvent::ReauthorizationRequired));
        assert_eq!(lifecycle_action(n[1].lifecycle_event.as_ref().unwrap()), LifecycleAction::Reauthorize);
        assert_eq!(lifecycle_action(&LifecycleEvent::SubscriptionRemoved), LifecycleAction::RecreateAndResync);
        assert_eq!(lifecycle_action(&LifecycleEvent::Missed), LifecycleAction::Resync);
        assert_eq!(lifecycle_action(n[2].lifecycle_event.as_ref().unwrap()), LifecycleAction::Ignore);
        assert!(!format!("{:?}", n[0]).contains("cs\""));
        assert!(parse_notifications(b"not json").is_err());
    }

    #[test]
    fn client_state_round_trip() {
        let cs = new_client_state();
        assert_eq!(cs.len(), 43);
        assert_ne!(cs, new_client_state());
        let h = client_state_hash(&cs);
        assert_eq!(h.len(), 64);
        assert!(verify_client_state(Some(&cs), &h));
        assert!(verify_client_state(Some(&cs), &h.to_uppercase()));
        assert!(!verify_client_state(Some("other"), &h));
        assert!(!verify_client_state(None, &h));
        assert!(!verify_client_state(Some(&cs), "abc"));
        assert_eq!(client_state_hash("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
