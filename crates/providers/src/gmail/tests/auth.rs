//! OAuth: authorization URL, code exchange, refresh (single-flight, skew, rotation), revocation.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use pretty_assertions::assert_eq;
use tokio::sync::Mutex;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use crate::gmail::auth::{
    AuthorizeRequest, ClientKind, GoogleTokenSource, OAuthClient, RefreshTokenStore, pkce_challenge, pkce_verifier,
};
use crate::{ProviderError, TokenSource};

fn client(server: &MockServer, kind: ClientKind) -> OAuthClient {
    let c = match kind {
        ClientKind::Web => OAuthClient::web("web-client.apps.googleusercontent.com", "web-secret"),
        ClientKind::Installed => OAuthClient::installed("desktop-client.apps.googleusercontent.com", None),
    };
    c.with_endpoints(
        "https://accounts.google.com/o/oauth2/v2/auth",
        &format!("{}/token", server.uri()),
        &format!("{}/revoke", server.uri()),
    )
}

fn token_json(access: &str, expires_in: i64, refresh: Option<&str>) -> ResponseTemplate {
    let mut v = serde_json::json!({
        "access_token": access, "expires_in": expires_in, "token_type": "Bearer",
        "scope": "openid https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/userinfo.email"
    });
    if let Some(r) = refresh {
        v["refresh_token"] = serde_json::json!(r);
    }
    ResponseTemplate::new(200).set_body_raw(v.to_string().into_bytes(), "application/json")
}

#[derive(Default)]
struct Store(Mutex<Vec<String>>);

#[async_trait]
impl RefreshTokenStore for Store {
    async fn rotated(&self, refresh_token: &str) -> Result<(), ProviderError> {
        self.0.lock().await.push(refresh_token.to_owned());
        Ok(())
    }
}

fn clock(at: Arc<AtomicI64>) -> crate::gmail::Clock {
    Arc::new(move || at.load(Ordering::SeqCst))
}

#[test]
fn pkce_matches_rfc7636() {
    assert_eq!(
        pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let v = pkce_verifier();
    assert_eq!(v.len(), 64);
    assert!(v.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_ne!(v, pkce_verifier());
}

#[tokio::test]
async fn authorize_url_carries_pkce_offline_access_and_the_scope() {
    let server = MockServer::start().await;
    let c = client(&server, ClientKind::Web);
    let url = c
        .authorize_url(&AuthorizeRequest {
            redirect_uri: "https://sync.akatsuki.vc/oauth/google/callback",
            state: "sealed.state",
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            login_hint: Some("ada@example.com"),
            scopes: &[],
        })
        .unwrap();
    let u = url::Url::parse(&url).unwrap();
    let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
    assert_eq!(u.host_str(), Some("accounts.google.com"));
    assert_eq!(q["client_id"], "web-client.apps.googleusercontent.com");
    assert_eq!(q["scope"], "openid email https://www.googleapis.com/auth/gmail.modify");
    assert_eq!(q["access_type"], "offline");
    assert_eq!(q["prompt"], "consent");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["state"], "sealed.state");
    assert_eq!(q["login_hint"], "ada@example.com");
    assert_eq!(q["response_type"], "code");
    assert!(!url.contains("web-secret"));
    assert!(!format!("{c:?}").contains("web-secret"));
}

#[tokio::test]
async fn exchange_code_sends_the_verifier_and_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=4%2F0Ab-synthetic"))
        .and(body_string_contains("code_verifier=verifier123"))
        .and(body_string_contains("client_secret=web-secret"))
        .and(body_string_contains("redirect_uri=https%3A%2F%2Fsync.akatsuki.vc%2Foauth%2Fgoogle%2Fcallback"))
        .respond_with(token_json("ya29.synthetic-access", 3599, Some("1//synthetic-refresh")))
        .expect(1)
        .mount(&server)
        .await;
    let t = client(&server, ClientKind::Web)
        .exchange_code(
            &reqwest::Client::new(),
            "4/0Ab-synthetic",
            "https://sync.akatsuki.vc/oauth/google/callback",
            "verifier123",
        )
        .await
        .unwrap();
    assert_eq!(t.refresh_token.as_deref(), Some("1//synthetic-refresh"));
    assert!(t.has_gmail_scope());
    assert_eq!(t.access_token_at(1_000).expires_at, 1_000 + 3_599_000);
    let dbg = format!("{t:?}");
    assert!(!dbg.contains("ya29") && !dbg.contains("1//"));
}

#[tokio::test]
async fn installed_clients_refresh_without_a_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(|r: &Request| {
            let b = String::from_utf8_lossy(&r.body);
            b.contains("grant_type=refresh_token")
                && b.contains("refresh_token=1%2F%2Frt")
                && b.contains("client_id=desktop-client")
                && !b.contains("client_secret")
        })
        .respond_with(token_json("ya29.a", 3600, None))
        .expect(1)
        .mount(&server)
        .await;
    let t = client(&server, ClientKind::Installed).refresh(&reqwest::Client::new(), "1//rt").await.unwrap();
    assert_eq!(t.access_token, "ya29.a");

    let no_secret = OAuthClient::web("x", "").with_endpoints("a", &format!("{}/token", server.uri()), "c");
    assert!(matches!(no_secret.refresh(&reqwest::Client::new(), "1//rt").await, Err(ProviderError::Invalid(_))));
}

#[tokio::test]
async fn token_source_caches_refreshes_at_the_skew_and_is_single_flight() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(token_json("ya29.first", 3600, None).set_delay(Duration::from_millis(100)))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(token_json("ya29.second", 3600, None))
        .expect(1)
        .mount(&server)
        .await;
    let now = Arc::new(AtomicI64::new(1_000_000));
    let ts = Arc::new(GoogleTokenSource::new(
        client(&server, ClientKind::Web),
        reqwest::Client::new(),
        "1//rt".into(),
        None,
        clock(now.clone()),
    ));
    // Ten concurrent callers, one refresh.
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let ts = ts.clone();
            tokio::spawn(async move { ts.access_token().await.unwrap().token })
        })
        .collect();
    for h in handles {
        assert_eq!(h.await.unwrap(), "ya29.first");
    }
    // 54 min later: still cached (expires in 6 min > 5 min skew).
    now.fetch_add(54 * 60 * 1000, Ordering::SeqCst);
    assert_eq!(ts.access_token().await.unwrap().token, "ya29.first");
    // 56 min: inside the skew, refresh.
    now.fetch_add(2 * 60 * 1000, Ordering::SeqCst);
    assert_eq!(ts.access_token().await.unwrap().token, "ya29.second");
}

#[tokio::test]
async fn invalidate_forces_a_refresh_and_rotation_is_persisted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=1%2F%2Fold"))
        .respond_with(token_json("ya29.one", 3600, Some("1//new")))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=1%2F%2Fnew"))
        .respond_with(token_json("ya29.two", 3600, Some("1//new")))
        .expect(1)
        .mount(&server)
        .await;
    let store = Arc::new(Store::default());
    let ts = GoogleTokenSource::new(
        client(&server, ClientKind::Web),
        reqwest::Client::new(),
        "1//old".into(),
        Some(store.clone()),
        clock(Arc::new(AtomicI64::new(0))),
    );
    assert_eq!(ts.access_token().await.unwrap().token, "ya29.one");
    ts.invalidate().await;
    assert_eq!(ts.access_token().await.unwrap().token, "ya29.two");
    assert_eq!(*store.0.lock().await, vec!["1//new".to_owned()], "only an actual change is persisted");
    assert!(!format!("{ts:?}").contains("1//"));
}

#[tokio::test]
async fn invalid_grant_requires_reauth_and_outages_are_transient() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=revoked"))
        .respond_with(ResponseTemplate::new(400).set_body_raw(
            br#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#.to_vec(),
            "application/json",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=flaky"))
        .respond_with(ResponseTemplate::new(503).insert_header("retry-after", "3"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("refresh_token=badclient"))
        .respond_with(
            ResponseTemplate::new(401).set_body_raw(br#"{"error":"invalid_client"}"#.to_vec(), "application/json"),
        )
        .mount(&server)
        .await;
    let ts = |rt: &str| {
        GoogleTokenSource::new(
            client(&server, ClientKind::Web),
            reqwest::Client::new(),
            rt.into(),
            None,
            clock(Arc::new(AtomicI64::new(0))),
        )
    };
    assert_eq!(ts("revoked").access_token().await.unwrap_err(), ProviderError::Auth { reauth_required: true });
    assert_eq!(
        ts("flaky").access_token().await.unwrap_err(),
        ProviderError::Transient("token endpoint 503".into(), Some(Duration::from_secs(3)))
    );
    assert!(matches!(ts("badclient").access_token().await, Err(ProviderError::Invalid(_))));
}

#[tokio::test]
async fn revoke_treats_an_unknown_token_as_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .and(body_string_contains("token=1%2F%2Frt"))
        .respond_with(
            ResponseTemplate::new(400).set_body_raw(br#"{"error":"invalid_token"}"#.to_vec(), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let ts = GoogleTokenSource::new(
        client(&server, ClientKind::Installed),
        reqwest::Client::new(),
        "1//rt".into(),
        None,
        clock(Arc::new(AtomicI64::new(0))),
    );
    ts.revoke().await.unwrap();
}
