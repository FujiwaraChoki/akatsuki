//! `akatsuki-sync rotate-secrets` (ARCHITECTURE §14.1): re-seal every account secret under the
//! active `TOKEN_ENC_KEYS` entry. Run it after adding the new key first and before removing the
//! old one. Accounts held by a running worker are skipped (that worker re-seals on its next
//! claim), so the command can be repeated until nothing is left.

use akatsuki_convex_api::WorkerApi;
use akatsuki_core::convex::values::ConvexBytes;
use akatsuki_core::convex::worker::{
    AccountWorkerArgs, AssignedAccount, ClaimAccountArgs, ClaimAccountResult, GetSecretArgs, PutSecretArgs,
};
use akatsuki_core::sync::LEASE_MS;

use crate::crypto::SecretKeyring;
use crate::engine::supervisor::secret_kind;

/// What happened to one account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotateOutcome {
    /// Re-sealed under the active key.
    Rotated,
    /// Already sealed under the active key.
    Current,
    /// No secret row.
    Missing,
    /// Held by another worker; try again later.
    Busy,
    /// Could not be decrypted with any configured key.
    Unreadable,
    /// Convex failed.
    Failed,
}

/// Totals of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RotateReport {
    /// Re-sealed.
    pub rotated: u32,
    /// Already current.
    pub current: u32,
    /// Without secret.
    pub missing: u32,
    /// Skipped because another worker holds the lease.
    pub busy: u32,
    /// Unreadable.
    pub unreadable: u32,
    /// Convex errors.
    pub failed: u32,
}

impl RotateReport {
    fn add(&mut self, o: RotateOutcome) {
        match o {
            RotateOutcome::Rotated => self.rotated += 1,
            RotateOutcome::Current => self.current += 1,
            RotateOutcome::Missing => self.missing += 1,
            RotateOutcome::Busy => self.busy += 1,
            RotateOutcome::Unreadable => self.unreadable += 1,
            RotateOutcome::Failed => self.failed += 1,
        }
    }

    /// `true` when nothing is left under an old key.
    pub fn complete(&self) -> bool {
        self.busy == 0 && self.unreadable == 0 && self.failed == 0
    }
}

/// Re-seals one account's secret under the active key. Claims the account briefly so
/// `worker.putSecret` passes its lease check.
pub async fn rotate_account(
    api: &dyn WorkerApi,
    keyring: &SecretKeyring,
    worker_id: &str,
    account: &AssignedAccount,
) -> RotateOutcome {
    let claim = api
        .claim_account(&ClaimAccountArgs {
            account_id: account.account_id.clone(),
            worker_id: worker_id.to_owned(),
            lease_ms: LEASE_MS,
        })
        .await;
    match claim {
        Ok(ClaimAccountResult::Claimed { .. }) => {}
        Ok(ClaimAccountResult::Taken { .. }) => return RotateOutcome::Busy,
        Err(_) => return RotateOutcome::Failed,
    }
    let outcome = rotate_claimed(api, keyring, worker_id, account).await;
    let _ = api
        .release_account(&AccountWorkerArgs { account_id: account.account_id.clone(), worker_id: worker_id.to_owned() })
        .await;
    outcome
}

async fn rotate_claimed(
    api: &dyn WorkerApi,
    keyring: &SecretKeyring,
    worker_id: &str,
    account: &AssignedAccount,
) -> RotateOutcome {
    let kind = secret_kind(account.provider);
    let rec = match api.get_secret(&GetSecretArgs { account_id: account.account_id.clone(), kind }).await {
        Ok(Some(r)) => r,
        Ok(None) => return RotateOutcome::Missing,
        Err(_) => return RotateOutcome::Failed,
    };
    if !keyring.needs_rotation(&rec.key_id) {
        return RotateOutcome::Current;
    }
    let aad = account.secret_aad(kind);
    let Ok(plain) = keyring.open(&rec.key_id, &rec.nonce.0, &rec.ciphertext.0, &aad) else {
        return RotateOutcome::Unreadable;
    };
    let Ok(sealed) = keyring.seal(&aad, &plain) else { return RotateOutcome::Failed };
    match api
        .put_secret(&PutSecretArgs {
            account_id: account.account_id.clone(),
            worker_id: worker_id.to_owned(),
            kind,
            key_id: sealed.key_id,
            nonce: ConvexBytes(sealed.nonce),
            ciphertext: ConvexBytes(sealed.ciphertext),
            expected_version: rec.version,
        })
        .await
    {
        Ok(_) => RotateOutcome::Rotated,
        Err(_) => RotateOutcome::Failed,
    }
}

/// Rotates every account in `accounts`.
pub async fn rotate_all(
    api: &dyn WorkerApi,
    keyring: &SecretKeyring,
    worker_id: &str,
    accounts: &[AssignedAccount],
) -> RotateReport {
    let mut report = RotateReport::default();
    for a in accounts {
        report.add(rotate_account(api, keyring, worker_id, a).await);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Key32;
    use akatsuki_convex_api::{Convex, MockTransport};
    use akatsuki_core::convex::{AccountCaps, AccountStatus, Provider, SecretKind};
    use base64::Engine as _;
    use serde_json::json;

    fn account(id: &str) -> AssignedAccount {
        AssignedAccount {
            account_id: id.into(),
            user_id: "u1".into(),
            provider: Provider::Gmail,
            provider_account_id: "ada@example.com".into(),
            email: "ada@example.com".into(),
            status: AccountStatus::Live,
            entitled: true,
            server_triage: false,
            imap: None,
            caps: AccountCaps { multi_label: true, categories: true, server_search: true, delete_forever: false },
        }
    }

    fn claimed() -> serde_json::Value {
        json!({"ok": true, "syncState": {"_id": "s1", "_creationTime": 1.0, "accountId": "acc00001", "shard": 0,
            "consecutiveFailures": 0, "updatedAt": 0}, "folderCursors": []})
    }

    #[tokio::test]
    async fn reseals_old_ciphertext_under_the_active_key() {
        let old = SecretKeyring::new(vec![("k1".into(), Key32([1; 32]))]);
        let a = account("acc00001");
        let aad = a.secret_aad(SecretKind::Oauth);
        let sealed = old.seal(&aad, br#"{"refreshToken":"r","scope":"","obtainedAt":0}"#).unwrap();
        let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);

        let mock = Convex::new(MockTransport::new());
        let t = mock.transport();
        t.respond("worker:claimAccount", Ok(claimed()));
        t.respond("worker:claimAccount", Ok(json!({"ok": false, "owner": "w-live"})));
        t.respond(
            "worker:getSecret",
            Ok(json!({"keyId": "k1", "nonce": {"$bytes": b64(&sealed.nonce)}, "ciphertext": {"$bytes": b64(&sealed.ciphertext)}, "version": 4})),
        );
        t.respond("worker:putSecret", Ok(json!({"version": 5})));
        t.respond("worker:releaseAccount", Ok(serde_json::Value::Null));

        let rotated = SecretKeyring::new(vec![("k2".into(), Key32([2; 32])), ("k1".into(), Key32([1; 32]))]);
        let report = rotate_all(&mock, &rotated, "rotate-1", &[a.clone(), account("acc00002")]).await;
        assert_eq!(report, RotateReport { rotated: 1, busy: 1, ..Default::default() });
        assert!(!report.complete());

        let put = mock.transport().calls().into_iter().find(|c| c.function == "worker:putSecret").unwrap();
        assert_eq!(put.args["keyId"], "k2");
        assert_eq!(put.args["expectedVersion"], 4);
        // The new ciphertext opens with the new key alone and the same AAD.
        let nonce =
            base64::engine::general_purpose::STANDARD.decode(put.args["nonce"]["$bytes"].as_str().unwrap()).unwrap();
        let ct = base64::engine::general_purpose::STANDARD
            .decode(put.args["ciphertext"]["$bytes"].as_str().unwrap())
            .unwrap();
        let new_only = SecretKeyring::new(vec![("k2".into(), Key32([2; 32]))]);
        assert!(new_only.open("k2", &nonce, &ct, &aad).is_ok());
    }
}
