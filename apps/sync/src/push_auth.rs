//! Verification of Gmail Pub/Sub push requests (ARCHITECTURE §9.6): the `Authorization: Bearer`
//! token is a Google-signed OIDC JWT with `iss` accounts.google.com, `aud == GMAIL_PUSH_AUDIENCE`,
//! `email == GMAIL_PUSH_SERVICE_ACCOUNT` and `email_verified`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// Google's OIDC signing keys.
pub const GOOGLE_CERTS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";
const KEYS_TTL: Duration = Duration::from_secs(3600);
const REFETCH_MIN: Duration = Duration::from_secs(60);

/// Where Google's keys come from.
#[async_trait]
pub trait GoogleKeys: Send + Sync {
    /// Fetches the key set.
    async fn fetch(&self) -> Result<JwkSet, ()>;
}

/// Fetches Google's keys over HTTPS.
#[derive(Clone, Debug)]
pub struct HttpGoogleKeys {
    http: reqwest::Client,
    url: String,
}

impl HttpGoogleKeys {
    /// The production source.
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, url: GOOGLE_CERTS_URL.into() }
    }
}

#[async_trait]
impl GoogleKeys for HttpGoogleKeys {
    async fn fetch(&self) -> Result<JwkSet, ()> {
        let r = self.http.get(&self.url).timeout(Duration::from_secs(10)).send().await.map_err(|_| ())?;
        r.error_for_status().map_err(|_| ())?.json().await.map_err(|_| ())
    }
}

#[derive(Debug, Deserialize)]
struct PushClaims {
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
}

/// Verifies Pub/Sub push tokens.
pub struct PushVerifier {
    keys: Arc<dyn GoogleKeys>,
    audience: String,
    service_account: String,
    cache: tokio::sync::Mutex<(Option<JwkSet>, Option<Instant>)>,
}

impl std::fmt::Debug for PushVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushVerifier").finish_non_exhaustive()
    }
}

impl PushVerifier {
    /// A verifier for `aud` and the push service account.
    pub fn new(keys: Arc<dyn GoogleKeys>, audience: String, service_account: String) -> Self {
        Self { keys, audience, service_account, cache: tokio::sync::Mutex::new((None, None)) }
    }

    async fn key(&self, kid: &str) -> Option<DecodingKey> {
        let mut c = self.cache.lock().await;
        let fresh = c.1.is_some_and(|t| t.elapsed() < KEYS_TTL);
        if fresh && let Some(jwk) = c.0.as_ref().and_then(|k| k.find(kid)) {
            return DecodingKey::from_jwk(jwk).ok();
        }
        if fresh && c.1.is_some_and(|t| t.elapsed() < REFETCH_MIN) {
            return None;
        }
        let set = self.keys.fetch().await.ok()?;
        *c = (Some(set), Some(Instant::now()));
        c.0.as_ref().and_then(|k| k.find(kid)).and_then(|j| DecodingKey::from_jwk(j).ok())
    }

    /// `true` when the token is a valid push token for this deployment.
    pub async fn verify(&self, token: &str) -> bool {
        let Ok(header) = jsonwebtoken::decode_header(token) else { return false };
        if header.alg != Algorithm::RS256 {
            return false;
        }
        let Some(kid) = header.kid else { return false };
        let Some(key) = self.key(&kid).await else { return false };
        let mut v = Validation::new(Algorithm::RS256);
        v.leeway = 30;
        v.set_issuer(&["accounts.google.com", "https://accounts.google.com"]);
        v.set_audience(&[self.audience.as_str()]);
        v.set_required_spec_claims(&["exp", "iss", "aud"]);
        let Ok(data) = jsonwebtoken::decode::<PushClaims>(token, &key, &v) else { return false };
        data.claims.email_verified && data.claims.email.as_deref() == Some(self.service_account.as_str())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use jsonwebtoken::{EncodingKey, Header};

    pub(crate) struct StaticKeys(pub JwkSet);

    #[async_trait]
    impl GoogleKeys for StaticKeys {
        async fn fetch(&self) -> Result<JwkSet, ()> {
            Ok(self.0.clone())
        }
    }

    pub(crate) fn rsa_key(kid: &str) -> (String, JwkSet) {
        use rsa::pkcs8::EncodePrivateKey;
        use rsa::traits::PublicKeyParts;
        let private = rsa::RsaPrivateKey::new(&mut rand08::rngs::OsRng, 2048).unwrap();
        let pem = private.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        let n = URL_SAFE_NO_PAD.encode(private.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(private.e().to_bytes_be());
        let set = serde_json::from_value(serde_json::json!({
            "keys": [{"kty": "RSA", "kid": kid, "alg": "RS256", "use": "sig", "n": n, "e": e}]
        }))
        .unwrap();
        (pem, set)
    }

    pub(crate) fn sign(pem: &str, kid: &str, claims: serde_json::Value) -> String {
        let mut h = Header::new(Algorithm::RS256);
        h.kid = Some(kid.into());
        jsonwebtoken::encode(&h, &claims, &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap()).unwrap()
    }

    pub(crate) fn push_claims(aud: &str, email: &str, verified: bool) -> serde_json::Value {
        let now = jsonwebtoken::get_current_timestamp();
        serde_json::json!({"iss": "https://accounts.google.com", "aud": aud, "email": email,
            "email_verified": verified, "exp": now + 300, "iat": now, "sub": "123"})
    }

    #[tokio::test]
    async fn verifies_google_push_tokens() {
        let (pem, set) = rsa_key("g1");
        let v = PushVerifier::new(
            Arc::new(StaticKeys(set)),
            "https://sync.akatsuki.vc/hooks/gmail".into(),
            "push@proj.iam.gserviceaccount.com".into(),
        );
        let good = sign(
            &pem,
            "g1",
            push_claims("https://sync.akatsuki.vc/hooks/gmail", "push@proj.iam.gserviceaccount.com", true),
        );
        assert!(v.verify(&good).await);
        for bad in [
            sign(&pem, "g1", push_claims("https://other", "push@proj.iam.gserviceaccount.com", true)),
            sign(
                &pem,
                "g1",
                push_claims("https://sync.akatsuki.vc/hooks/gmail", "evil@x.iam.gserviceaccount.com", true),
            ),
            sign(
                &pem,
                "g1",
                push_claims("https://sync.akatsuki.vc/hooks/gmail", "push@proj.iam.gserviceaccount.com", false),
            ),
            sign(
                &pem,
                "unknown",
                push_claims("https://sync.akatsuki.vc/hooks/gmail", "push@proj.iam.gserviceaccount.com", true),
            ),
            "not.a.jwt".to_owned(),
        ] {
            assert!(!v.verify(&bad).await);
        }
        let (other_pem, _) = rsa_key("g1");
        let forged = sign(
            &other_pem,
            "g1",
            push_claims("https://sync.akatsuki.vc/hooks/gmail", "push@proj.iam.gserviceaccount.com", true),
        );
        assert!(!v.verify(&forged).await);
    }
}
