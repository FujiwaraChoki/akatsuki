//! User JWT verification (ARCHITECTURE §6.5) and service JWT minting (§6.6).
//!
//! - [`UserVerifier`] checks the Better Auth / Convex user JWT that every client sends to the
//!   worker: RS256 only, `iss == CONVEX_SITE_URL`, `aud == "convex"`, `exp`/`nbf` with 30 s
//!   leeway, non-empty `sub`. Keys come from `${CONVEX_SITE_URL}/api/auth/convex/jwks`, cached for
//!   an hour and refetched at most once a minute when an unknown `kid` appears.
//! - [`ServiceTokenMinter`] signs the worker's own ES256 tokens for Convex (`aud:
//!   "akatsuki-service"`) and apps/api (`aud: "akatsuki-api"`), 10 minutes each.
//!
//! Authorization is never taken from claims: callers resolve every id against Convex with
//! `userId = sub`.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// The audience of user JWTs.
pub const USER_AUDIENCE: &str = "convex";
/// The audience of service JWTs sent to Convex.
pub const SERVICE_AUDIENCE: &str = "akatsuki-service";
/// The audience of worker JWTs sent to apps/api `/internal/*`.
pub const API_INTERNAL_AUDIENCE: &str = "akatsuki-api";
/// Clock leeway for `exp` and `nbf`.
pub const LEEWAY_SECS: u64 = 30;
/// JWKS cache lifetime.
pub const JWKS_TTL: Duration = Duration::from_secs(3600);
/// Minimum delay between refetches triggered by an unknown `kid`.
pub const JWKS_REFETCH_MIN: Duration = Duration::from_secs(60);
/// Service token lifetime.
pub const SERVICE_TOKEN_SECS: u64 = 600;

/// Why a token was rejected. Every variant maps to `401 unauthenticated`; the variants exist for
/// tests and metrics, never for the response body.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The token is not a JWT.
    #[error("malformed token")]
    Malformed,
    /// `alg` is not RS256 (including `none`).
    #[error("algorithm not allowed")]
    AlgorithmNotAllowed,
    /// The header has no `kid`.
    #[error("missing key id")]
    MissingKid,
    /// No key with this `kid`, even after a refetch.
    #[error("unknown key id")]
    UnknownKid,
    /// Signature, issuer, audience or time checks failed.
    #[error("invalid token: {0}")]
    Invalid(String),
    /// `sub` is empty.
    #[error("empty subject")]
    EmptySubject,
    /// The JWKS could not be fetched.
    #[error("jwks unavailable: {0}")]
    JwksUnavailable(String),
    /// A signing key could not be loaded.
    #[error("bad signing key: {0}")]
    BadKey(String),
}

/// Claims of a user JWT (Better Auth Convex plugin).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserClaims {
    /// Better Auth user id.
    pub sub: String,
    /// `CONVEX_SITE_URL`.
    pub iss: String,
    /// Expiry (seconds).
    pub exp: u64,
    /// Issued at (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,
    /// Session id (always added by the plugin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Email (defined in `definePayload`; apps/api uses it for Stripe only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// A verified caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedUser {
    /// `sub`, the Convex `userId`.
    pub user_id: String,
    /// Session id, if present.
    pub session_id: Option<String>,
}

/// Where the verifier gets its keys. The production source is [`HttpJwksSource`].
pub trait JwksSource: Send + Sync {
    /// Fetches the current key set.
    fn fetch(&self) -> impl Future<Output = Result<JwkSet, AuthError>> + Send;
}

/// Fetches the JWKS over HTTPS.
#[derive(Clone, Debug)]
pub struct HttpJwksSource {
    client: reqwest::Client,
    url: String,
}

impl HttpJwksSource {
    /// `${convex_site_url}/api/auth/convex/jwks`.
    pub fn for_convex_site(client: reqwest::Client, convex_site_url: &str) -> Self {
        Self { client, url: format!("{}/api/auth/convex/jwks", convex_site_url.trim_end_matches('/')) }
    }
}

impl JwksSource for HttpJwksSource {
    async fn fetch(&self) -> Result<JwkSet, AuthError> {
        let resp = self
            .client
            .get(&self.url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| AuthError::JwksUnavailable(e.without_url().to_string()))?;
        let resp = resp.error_for_status().map_err(|e| AuthError::JwksUnavailable(e.without_url().to_string()))?;
        resp.json::<JwkSet>().await.map_err(|e| AuthError::JwksUnavailable(e.without_url().to_string()))
    }
}

#[derive(Default)]
struct Cache {
    keys: Option<JwkSet>,
    fetched_at: Option<Instant>,
}

/// Verifies user JWTs (§6.5).
pub struct UserVerifier<S: JwksSource> {
    source: S,
    issuer: String,
    cache: Arc<Mutex<Cache>>,
}

impl<S: JwksSource> std::fmt::Debug for UserVerifier<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserVerifier").field("issuer", &self.issuer).finish_non_exhaustive()
    }
}

impl<S: JwksSource> UserVerifier<S> {
    /// A verifier for tokens issued by `issuer` (`CONVEX_SITE_URL`).
    pub fn new(source: S, issuer: impl Into<String>) -> Self {
        Self { source, issuer: issuer.into(), cache: Arc::new(Mutex::new(Cache::default())) }
    }

    /// Verifies a bearer token and returns the caller.
    pub async fn verify(&self, token: &str) -> Result<VerifiedUser, AuthError> {
        let header = jsonwebtoken::decode_header(token).map_err(|_| AuthError::Malformed)?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::AlgorithmNotAllowed);
        }
        let kid = header.kid.ok_or(AuthError::MissingKid)?;
        let key = self.key_for(&kid).await?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.leeway = LEEWAY_SECS;
        validation.validate_nbf = true;
        validation.validate_exp = true;
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[USER_AUDIENCE]);
        validation.set_required_spec_claims(&["exp", "sub", "iss", "aud"]);

        let data = jsonwebtoken::decode::<UserClaims>(token, &key, &validation)
            .map_err(|e| AuthError::Invalid(e.to_string()))?;
        if data.claims.sub.trim().is_empty() {
            return Err(AuthError::EmptySubject);
        }
        Ok(VerifiedUser { user_id: data.claims.sub, session_id: data.claims.session_id })
    }

    async fn key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        let mut cache = self.cache.lock().await;
        let fresh = cache.fetched_at.is_some_and(|t| t.elapsed() < JWKS_TTL);
        if fresh {
            if let Some(jwk) = cache.keys.as_ref().and_then(|k| k.find(kid)) {
                return DecodingKey::from_jwk(jwk).map_err(|e| AuthError::Invalid(e.to_string()));
            }
            // Unknown kid: refetch at most once a minute.
            if cache.fetched_at.is_some_and(|t| t.elapsed() < JWKS_REFETCH_MIN) {
                return Err(AuthError::UnknownKid);
            }
        }
        let keys = self.source.fetch().await?;
        cache.keys = Some(keys);
        cache.fetched_at = Some(Instant::now());
        let jwk = cache.keys.as_ref().and_then(|k| k.find(kid)).ok_or(AuthError::UnknownKid)?;
        DecodingKey::from_jwk(jwk).map_err(|e| AuthError::Invalid(e.to_string()))
    }
}

/// Claims of a service JWT (§6.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceClaims {
    /// `WORKER_ISSUER` / `API_ISSUER`.
    pub iss: String,
    /// `akatsuki-service` or `akatsuki-api`.
    pub aud: String,
    /// `worker:<instanceId>` or `api`.
    pub sub: String,
    /// Issued at (seconds).
    pub iat: u64,
    /// Expiry: `iat + 600`.
    pub exp: u64,
}

/// Signs ES256 service tokens with the worker's P-256 key.
pub struct ServiceTokenMinter {
    key: EncodingKey,
    kid: String,
    issuer: String,
    subject: String,
}

impl std::fmt::Debug for ServiceTokenMinter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceTokenMinter")
            .field("kid", &self.kid)
            .field("issuer", &self.issuer)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

impl ServiceTokenMinter {
    /// Loads a PKCS#8 PEM P-256 private key (`WORKER_SERVICE_PRIVATE_KEY`).
    pub fn from_pem(
        pem: &str,
        kid: impl Into<String>,
        issuer: impl Into<String>,
        subject: impl Into<String>,
    ) -> Result<Self, AuthError> {
        let key = EncodingKey::from_ec_pem(pem.as_bytes()).map_err(|e| AuthError::BadKey(e.to_string()))?;
        Ok(Self { key, kid: kid.into(), issuer: issuer.into(), subject: subject.into() })
    }

    /// Mints a token for `audience`, issued at `now_secs`.
    pub fn mint_at(&self, audience: &str, now_secs: u64) -> Result<String, AuthError> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.kid.clone());
        let claims = ServiceClaims {
            iss: self.issuer.clone(),
            aud: audience.to_owned(),
            sub: self.subject.clone(),
            iat: now_secs,
            exp: now_secs + SERVICE_TOKEN_SECS,
        };
        jsonwebtoken::encode(&header, &claims, &self.key).map_err(|e| AuthError::BadKey(e.to_string()))
    }

    /// Mints a token for `audience`, issued now.
    pub fn mint(&self, audience: &str) -> Result<String, AuthError> {
        self.mint_at(audience, jsonwebtoken::get_current_timestamp())
    }
}

/// Extracts the token from an `Authorization: Bearer <token>` header value.
pub fn bearer_token(header_value: &str) -> Option<&str> {
    let (scheme, token) = header_value.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct StaticJwks {
        set: JwkSet,
        fetches: Arc<AtomicUsize>,
    }

    impl JwksSource for StaticJwks {
        async fn fetch(&self) -> Result<JwkSet, AuthError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            Ok(self.set.clone())
        }
    }

    struct TestKey {
        pem: String,
        jwks: JwkSet,
    }

    fn rsa_key(kid: &str) -> TestKey {
        use rsa::pkcs8::EncodePrivateKey;
        use rsa::traits::PublicKeyParts;
        let mut rng = rand08::rngs::OsRng;
        let private = rsa::RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = private.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap().to_string();
        let n = URL_SAFE_NO_PAD.encode(private.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(private.e().to_bytes_be());
        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{"kty": "RSA", "kid": kid, "alg": "RS256", "use": "sig", "n": n, "e": e}]
        }))
        .unwrap();
        TestKey { pem, jwks }
    }

    fn sign(pem: &str, kid: &str, claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_owned());
        jsonwebtoken::encode(&header, &claims, &EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap()).unwrap()
    }

    fn claims(iss: &str, aud: &str, sub: &str, exp_offset: i64) -> serde_json::Value {
        let now = jsonwebtoken::get_current_timestamp() as i64;
        serde_json::json!({"iss": iss, "aud": aud, "sub": sub, "exp": now + exp_offset, "iat": now, "sessionId": "s1"})
    }

    const ISS: &str = "https://happy-otter-123.convex.site";

    #[tokio::test]
    async fn verifies_user_tokens_and_rejects_bad_ones() {
        let key = rsa_key("k1");
        let fetches = Arc::new(AtomicUsize::new(0));
        let v = UserVerifier::new(StaticJwks { set: key.jwks.clone(), fetches: fetches.clone() }, ISS);

        let ok = sign(&key.pem, "k1", claims(ISS, "convex", "user_1", 900));
        let user = v.verify(&ok).await.unwrap();
        assert_eq!(user.user_id, "user_1");
        assert_eq!(user.session_id.as_deref(), Some("s1"));

        // Wrong issuer, wrong audience, expired beyond leeway, empty subject.
        assert!(matches!(
            v.verify(&sign(&key.pem, "k1", claims("https://evil", "convex", "u", 900))).await,
            Err(AuthError::Invalid(_))
        ));
        assert!(matches!(
            v.verify(&sign(&key.pem, "k1", claims(ISS, "other", "u", 900))).await,
            Err(AuthError::Invalid(_))
        ));
        assert!(matches!(
            v.verify(&sign(&key.pem, "k1", claims(ISS, "convex", "u", -120))).await,
            Err(AuthError::Invalid(_))
        ));
        assert!(matches!(
            v.verify(&sign(&key.pem, "k1", claims(ISS, "convex", "  ", 900))).await,
            Err(AuthError::EmptySubject)
        ));
        // Expired within leeway is accepted.
        assert!(v.verify(&sign(&key.pem, "k1", claims(ISS, "convex", "u", -10))).await.is_ok());

        // Only one fetch so far: the key set is cached.
        assert_eq!(fetches.load(Ordering::SeqCst), 1);

        // Unknown kid within the refetch window does not refetch.
        let other = sign(&key.pem, "k2", claims(ISS, "convex", "u", 900));
        assert!(matches!(v.verify(&other).await, Err(AuthError::UnknownKid)));
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rejects_none_and_hmac_algorithms() {
        let key = rsa_key("k1");
        let v = UserVerifier::new(StaticJwks { set: key.jwks, fetches: Arc::new(AtomicUsize::new(0)) }, ISS);
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","kid":"k1"}"#);
        let body = URL_SAFE_NO_PAD.encode(claims(ISS, "convex", "u", 900).to_string());
        assert!(matches!(
            v.verify(&format!("{header}.{body}.")).await,
            Err(AuthError::AlgorithmNotAllowed | AuthError::Malformed)
        ));

        let mut hs = Header::new(Algorithm::HS256);
        hs.kid = Some("k1".into());
        let token =
            jsonwebtoken::encode(&hs, &claims(ISS, "convex", "u", 900), &EncodingKey::from_secret(b"x")).unwrap();
        assert!(matches!(v.verify(&token).await, Err(AuthError::AlgorithmNotAllowed)));
        assert!(matches!(v.verify("not-a-jwt").await, Err(AuthError::Malformed)));
    }

    #[test]
    fn mints_es256_service_tokens() {
        use p256::pkcs8::{EncodePrivateKey, EncodePublicKey};
        let signing = p256::ecdsa::SigningKey::random(&mut rand08::rngs::OsRng);
        let pem = signing.to_pkcs8_pem(p256::pkcs8::LineEnding::LF).unwrap().to_string();
        let public = signing.verifying_key().to_public_key_pem(p256::pkcs8::LineEnding::LF).unwrap();

        let minter = ServiceTokenMinter::from_pem(&pem, "w1", "https://sync.akatsuki.vc", "worker:abc").unwrap();
        let token = minter.mint(SERVICE_AUDIENCE).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        assert_eq!(header.alg, Algorithm::ES256);
        assert_eq!(header.kid.as_deref(), Some("w1"));

        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&[SERVICE_AUDIENCE]);
        validation.set_issuer(&["https://sync.akatsuki.vc"]);
        let data = jsonwebtoken::decode::<ServiceClaims>(
            &token,
            &DecodingKey::from_ec_pem(public.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap();
        assert_eq!(data.claims.sub, "worker:abc");
        assert_eq!(data.claims.exp - data.claims.iat, SERVICE_TOKEN_SECS);
        assert!(format!("{minter:?}").contains("w1"));
    }

    #[test]
    fn bearer_parsing() {
        assert_eq!(bearer_token("Bearer abc.def.ghi"), Some("abc.def.ghi"));
        assert_eq!(bearer_token("bearer  x "), Some("x"));
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
    }
}
