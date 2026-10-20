//! Desktop sign-in (ARCHITECTURE §6.3, D-016): a per-device Better Auth session, obtained through
//! stock endpoints only.
//!
//! 1. Bind `127.0.0.1:0`; `callbackURL = http://127.0.0.1:P/cb?n=<nonce>` (32 random bytes).
//! 2. Social: `POST /api/auth/sign-in/social {provider, callbackURL, disableRedirect: true}` returns
//!    `{url}`, opened in the system browser. Email: `POST /api/auth/sign-in/magic-link`.
//! 3. Better Auth redirects to the loopback with `&ott=<token>` (crossDomain plugin).
//! 4. The loopback checks the nonce (else `400`, keep waiting), answers "You can close this tab"
//!    and exchanges the token at `POST /api/auth/cross-domain/one-time-token/verify`.
//! 5. The session token goes to the OS keychain; JWTs come from `GET /api/auth/convex/token` with
//!    `Authorization: Bearer <session>` and are refreshed 120 s before `exp`.
//! 6. Fallback: email OTP typed into the app.
//! 7. Sign-out: `POST /api/auth/sign-out` with the bearer token.
//!
//! Tokens are never logged; errors carry status codes only.

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand08::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Refresh a JWT this long before it expires (§6.3 step 6).
pub const REFRESH_BEFORE_SECS: i64 = 120;

/// Sign-in errors. Messages are safe to show.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The network failed.
    #[error("Could not reach the server.")]
    Network,
    /// The session is not (or no longer) valid.
    #[error("Your session has ended. Sign in again.")]
    Unauthenticated,
    /// The code was wrong or expired.
    #[error("That code did not work. Check it, or ask for a new one.")]
    InvalidCode,
    /// Too many attempts.
    #[error("Too many attempts. Wait a minute and try again.")]
    RateLimited,
    /// The server answered something unexpected.
    #[error("Sign-in failed (HTTP {0}).")]
    Server(u16),
    /// The response did not have the expected shape.
    #[error("The server sent an unexpected answer.")]
    Protocol,
    /// The loopback listener could not start (use the email code instead).
    #[error("Could not listen for the browser on this machine. Use an email code instead.")]
    Loopback,
    /// The provider or Better Auth reported an error on the redirect.
    #[error("The browser sign-in was not completed ({0}).")]
    Redirect(String),
    /// Nobody came back in time.
    #[error("The browser sign-in timed out.")]
    Timeout,
}

/// A Better Auth session token (kept in the keychain).
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    /// Bearer token.
    pub token: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session(..)")
    }
}

/// A Convex user JWT.
#[derive(Clone, PartialEq, Eq)]
pub struct Jwt {
    /// The token.
    pub token: String,
    /// `exp` in seconds since the epoch.
    pub exp: i64,
    /// `email` claim (display only; authorization never trusts claims).
    pub email: Option<String>,
}

impl std::fmt::Debug for Jwt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jwt").field("exp", &self.exp).finish_non_exhaustive()
    }
}

impl Jwt {
    /// Reads `exp` and `email` from the payload without verifying it (the server verifies).
    pub fn parse(token: &str) -> Result<Self, AuthError> {
        let payload = token.split('.').nth(1).ok_or(AuthError::Protocol)?;
        let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|_| AuthError::Protocol)?;
        let claims: Value = serde_json::from_slice(&bytes).map_err(|_| AuthError::Protocol)?;
        let exp = claims.get("exp").and_then(Value::as_i64).ok_or(AuthError::Protocol)?;
        let email = claims.get("email").and_then(Value::as_str).map(str::to_owned);
        Ok(Self { token: token.to_owned(), exp, email })
    }

    /// Seconds from `now_secs` until this JWT should be replaced.
    pub fn refresh_in(&self, now_secs: i64) -> Duration {
        Duration::from_secs(u64::try_from(self.exp - REFRESH_BEFORE_SECS - now_secs).unwrap_or(0))
    }
}

/// Social login providers (login only; mailbox access is separate, §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocialProvider {
    /// Google.
    Google,
    /// Microsoft.
    Microsoft,
    /// GitHub (D-054).
    Github,
}

impl SocialProvider {
    /// Better Auth provider id.
    pub fn id(self) -> &'static str {
        match self {
            SocialProvider::Google => "google",
            SocialProvider::Microsoft => "microsoft",
            SocialProvider::Github => "github",
        }
    }

    /// Button label.
    pub fn label(self) -> &'static str {
        match self {
            SocialProvider::Google => "Continue with Google",
            SocialProvider::Microsoft => "Continue with Microsoft",
            SocialProvider::Github => "Continue with GitHub",
        }
    }
}

/// Better Auth on `*.convex.site`.
#[derive(Clone, Debug)]
pub struct AuthApi {
    http: reqwest::Client,
    site_url: String,
}

#[derive(Deserialize)]
struct TokenBody {
    token: Option<String>,
}

#[derive(Deserialize)]
struct UrlBody {
    url: String,
}

#[derive(Deserialize)]
struct OttBody {
    session: Option<TokenBody>,
    token: Option<String>,
}

fn status_error(status: reqwest::StatusCode) -> AuthError {
    match status.as_u16() {
        401 | 403 => AuthError::Unauthenticated,
        429 => AuthError::RateLimited,
        s => AuthError::Server(s),
    }
}

impl AuthApi {
    /// A client for `site_url` (the Convex site URL, without `/api/auth`).
    pub fn new(http: reqwest::Client, site_url: &str) -> Self {
        Self { http, site_url: site_url.trim_end_matches('/').to_owned() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/auth{path}", self.site_url)
    }

    async fn post(&self, path: &str, body: Value, bearer: Option<&str>) -> Result<reqwest::Response, AuthError> {
        let mut req = self.http.post(self.url(path)).json(&body);
        if let Some(b) = bearer {
            req = req.bearer_auth(b);
        }
        req.send().await.map_err(|_| AuthError::Network)
    }

    /// The session token of a response: the bearer plugin's `set-auth-token` header, else the
    /// body's token.
    fn session_from(headers: &reqwest::header::HeaderMap, body_token: Option<String>) -> Result<Session, AuthError> {
        let header = headers.get("set-auth-token").and_then(|v| v.to_str().ok()).map(str::to_owned);
        header.or(body_token).filter(|t| !t.is_empty()).map(|token| Session { token }).ok_or(AuthError::Protocol)
    }

    /// Emails a sign-in code.
    pub async fn send_otp(&self, email: &str) -> Result<(), AuthError> {
        let res =
            self.post("/email-otp/send-verification-otp", json!({"email": email, "type": "sign-in"}), None).await?;
        if !res.status().is_success() {
            return Err(status_error(res.status()));
        }
        Ok(())
    }

    /// Signs in with the emailed code.
    pub async fn verify_otp(&self, email: &str, otp: &str) -> Result<Session, AuthError> {
        let res = self.post("/sign-in/email-otp", json!({"email": email, "otp": otp.trim()}), None).await?;
        let status = res.status();
        if status.as_u16() == 400 || status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(AuthError::InvalidCode);
        }
        if !status.is_success() {
            return Err(status_error(status));
        }
        let headers = res.headers().clone();
        let body: TokenBody = res.json().await.map_err(|_| AuthError::Protocol)?;
        Self::session_from(&headers, body.token)
    }

    /// Starts a social sign-in and returns the provider URL to open.
    pub async fn start_social(&self, provider: SocialProvider, callback_url: &str) -> Result<String, AuthError> {
        let res = self
            .post(
                "/sign-in/social",
                json!({"provider": provider.id(), "callbackURL": callback_url, "disableRedirect": true}),
                None,
            )
            .await?;
        if !res.status().is_success() {
            return Err(status_error(res.status()));
        }
        let body: UrlBody = res.json().await.map_err(|_| AuthError::Protocol)?;
        if !(body.url.starts_with("https://") || body.url.starts_with("http://")) {
            return Err(AuthError::Protocol);
        }
        Ok(body.url)
    }

    /// Emails a magic link that returns to the loopback.
    pub async fn send_magic_link(&self, email: &str, callback_url: &str) -> Result<(), AuthError> {
        let res = self.post("/sign-in/magic-link", json!({"email": email, "callbackURL": callback_url}), None).await?;
        if !res.status().is_success() {
            return Err(status_error(res.status()));
        }
        Ok(())
    }

    /// Exchanges the crossDomain one-time token for this device's session.
    pub async fn verify_one_time_token(&self, ott: &str) -> Result<Session, AuthError> {
        let res = self.post("/cross-domain/one-time-token/verify", json!({"token": ott}), None).await?;
        if !res.status().is_success() {
            return Err(status_error(res.status()));
        }
        let headers = res.headers().clone();
        let body: OttBody = res.json().await.map_err(|_| AuthError::Protocol)?;
        Self::session_from(&headers, body.session.and_then(|s| s.token).or(body.token))
    }

    /// Mints a Convex JWT for the session.
    pub async fn mint_jwt(&self, session: &Session) -> Result<Jwt, AuthError> {
        let res = self
            .http
            .get(self.url("/convex/token"))
            .bearer_auth(&session.token)
            .send()
            .await
            .map_err(|_| AuthError::Network)?;
        if !res.status().is_success() {
            return Err(status_error(res.status()));
        }
        let body: TokenBody = res.json().await.map_err(|_| AuthError::Protocol)?;
        Jwt::parse(&body.token.ok_or(AuthError::Unauthenticated)?)
    }

    /// Revokes the session on the server.
    pub async fn sign_out(&self, session: &Session) -> Result<(), AuthError> {
        let res = self.post("/sign-out", json!({}), Some(&session.token)).await?;
        if !res.status().is_success() && res.status().as_u16() != 401 {
            return Err(status_error(res.status()));
        }
        Ok(())
    }
}

/// The loopback listener of steps 1, 3 and 4.
pub struct Loopback {
    listener: TcpListener,
    nonce: String,
    port: u16,
}

impl std::fmt::Debug for Loopback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loopback").field("port", &self.port).finish_non_exhaustive()
    }
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Akatsuki</title>\
<style>body{font:16px system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0}</style>\
<p>You are signed in to Akatsuki. You can close this tab.</p>";
const BAD_PAGE: &str =
    "<!doctype html><meta charset=utf-8><title>Akatsuki</title><p>This sign-in link is not valid.</p>";

impl Loopback {
    /// Binds `127.0.0.1:0` with a fresh nonce.
    pub async fn bind() -> Result<Self, AuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|_| AuthError::Loopback)?;
        let port = listener.local_addr().map_err(|_| AuthError::Loopback)?.port();
        let mut bytes = [0u8; 32];
        rand08::rngs::OsRng.fill_bytes(&mut bytes);
        Ok(Self { listener, nonce: URL_SAFE_NO_PAD.encode(bytes), port })
    }

    /// `http://127.0.0.1:P/cb?n=<nonce>`.
    pub fn callback_url(&self) -> String {
        format!("http://127.0.0.1:{}/cb?n={}", self.port, self.nonce)
    }

    /// Waits for the browser to come back with the one-time token. Requests without the right
    /// nonce get a `400` and are ignored.
    pub async fn wait_for_token(self, timeout: Duration) -> Result<String, AuthError> {
        tokio::time::timeout(timeout, async {
            loop {
                let Ok((mut stream, _)) = self.listener.accept().await else { continue };
                let mut buf = vec![0u8; 8192];
                let n = match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
                    Ok(Ok(n)) => n,
                    _ => continue,
                };
                let request = String::from_utf8_lossy(&buf[..n]).into_owned();
                let verdict = check_callback(&request, &self.nonce);
                let (status, page) = match &verdict {
                    Callback::Token(_) => ("200 OK", DONE_PAGE),
                    Callback::Error(_) => ("200 OK", BAD_PAGE),
                    Callback::Ignore => ("400 Bad Request", BAD_PAGE),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\n\
                     Referrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
                match verdict {
                    Callback::Token(t) => return Ok(t),
                    Callback::Error(e) => return Err(AuthError::Redirect(e)),
                    Callback::Ignore => continue,
                }
            }
        })
        .await
        .map_err(|_| AuthError::Timeout)?
    }
}

/// What a request to the loopback carried.
#[derive(Debug, PartialEq, Eq)]
enum Callback {
    Token(String),
    Error(String),
    Ignore,
}

fn check_callback(request: &str, nonce: &str) -> Callback {
    let Some(target) = request.lines().next().and_then(|l| {
        let mut parts = l.split(' ');
        (parts.next() == Some("GET")).then(|| parts.next()).flatten()
    }) else {
        return Callback::Ignore;
    };
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else { return Callback::Ignore };
    if url.path() != "/cb" {
        return Callback::Ignore;
    }
    let get = |k: &str| url.query_pairs().find(|(name, _)| name == k).map(|(_, v)| v.into_owned());
    // Constant-time comparison of the nonce.
    let given = get("n").unwrap_or_default();
    let same =
        given.len() == nonce.len() && given.bytes().zip(nonce.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    if !same {
        return Callback::Ignore;
    }
    if let Some(e) = get("error") {
        // Only a short code-like value is shown.
        let code: String = e.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(40).collect();
        return Callback::Error(code);
    }
    match get("ott") {
        Some(t) if !t.is_empty() => Callback::Token(t),
        _ => Callback::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn jwt(exp: i64) -> String {
        let payload = URL_SAFE_NO_PAD.encode(json!({"sub": "u1", "exp": exp, "email": "ada@example.com"}).to_string());
        format!("eyJhbGciOiJSUzI1NiJ9.{payload}.sig")
    }

    async fn api() -> (MockServer, AuthApi) {
        let server = MockServer::start().await;
        let api = AuthApi::new(reqwest::Client::new(), &format!("{}/", server.uri()));
        (server, api)
    }

    #[tokio::test]
    async fn email_otp_sign_in() {
        let (server, api) = api().await;
        Mock::given(method("POST"))
            .and(path("/api/auth/email-otp/send-verification-otp"))
            .and(body_json(json!({"email": "ada@example.com", "type": "sign-in"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/auth/sign-in/email-otp"))
            .and(body_json(json!({"email": "ada@example.com", "otp": "123456"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": "sess-1", "user": {}})))
            .mount(&server)
            .await;
        api.send_otp("ada@example.com").await.unwrap();
        assert_eq!(api.verify_otp("ada@example.com", " 123456 ").await.unwrap().token, "sess-1");
    }

    #[tokio::test]
    async fn wrong_codes_and_rate_limits_are_reported() {
        let (server, api) = api().await;
        Mock::given(path("/api/auth/sign-in/email-otp")).respond_with(ResponseTemplate::new(400)).mount(&server).await;
        Mock::given(path("/api/auth/email-otp/send-verification-otp"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        assert_eq!(api.verify_otp("a@b.c", "1").await.unwrap_err(), AuthError::InvalidCode);
        assert_eq!(api.send_otp("a@b.c").await.unwrap_err(), AuthError::RateLimited);
    }

    #[tokio::test]
    async fn social_start_and_one_time_token_exchange() {
        let (server, api) = api().await;
        Mock::given(method("POST"))
            .and(path("/api/auth/sign-in/social"))
            .and(body_json(
                json!({"provider": "google", "callbackURL": "http://127.0.0.1:9/cb?n=x", "disableRedirect": true}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"url": "https://accounts.example/auth", "redirect": false})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/auth/cross-domain/one-time-token/verify"))
            .and(body_json(json!({"token": "ott-1"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("set-auth-token", "sess-signed.sig")
                    .set_body_json(json!({"session": {"token": "sess-plain", "expiresAt": "x"}, "user": {}})),
            )
            .mount(&server)
            .await;
        let url = api.start_social(SocialProvider::Google, "http://127.0.0.1:9/cb?n=x").await.unwrap();
        assert_eq!(url, "https://accounts.example/auth");
        let s = api.verify_one_time_token("ott-1").await.unwrap();
        assert_eq!(s.token, "sess-signed.sig", "the bearer header wins");
    }

    #[tokio::test]
    async fn jwt_minting_uses_the_bearer_session() {
        let (server, api) = api().await;
        let token = jwt(2_000_000_000);
        Mock::given(method("GET"))
            .and(path("/api/auth/convex/token"))
            .and(header("authorization", "Bearer sess-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": token})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/auth/convex/token"))
            .and(header("authorization", "Bearer revoked"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let j = api.mint_jwt(&Session { token: "sess-1".into() }).await.unwrap();
        assert_eq!(j.exp, 2_000_000_000);
        assert_eq!(j.email.as_deref(), Some("ada@example.com"));
        assert_eq!(j.refresh_in(2_000_000_000 - 200), Duration::from_secs(80));
        assert_eq!(j.refresh_in(2_000_000_000), Duration::ZERO);
        assert_eq!(api.mint_jwt(&Session { token: "revoked".into() }).await.unwrap_err(), AuthError::Unauthenticated);
        assert!(!format!("{j:?}").contains(&token), "debug output hides the token");
    }

    #[tokio::test]
    async fn sign_out_sends_the_bearer() {
        let (server, api) = api().await;
        Mock::given(method("POST"))
            .and(path("/api/auth/sign-out"))
            .and(header("authorization", "Bearer sess-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
            .expect(1)
            .mount(&server)
            .await;
        api.sign_out(&Session { token: "sess-1".into() }).await.unwrap();
    }

    #[tokio::test]
    async fn loopback_checks_the_nonce_then_returns_the_token() {
        let lb = Loopback::bind().await.unwrap();
        let cb = lb.callback_url();
        assert!(cb.starts_with("http://127.0.0.1:") && cb.contains("/cb?n="));
        let wait = tokio::spawn(lb.wait_for_token(Duration::from_secs(10)));
        let http = reqwest::Client::new();
        let base = cb.split("/cb").next().unwrap().to_owned();
        let bad = http.get(format!("{base}/cb?n=wrong&ott=stolen")).send().await.unwrap();
        assert_eq!(bad.status().as_u16(), 400);
        let ok = http.get(format!("{cb}&ott=ott-123")).send().await.unwrap();
        assert_eq!(ok.status().as_u16(), 200);
        assert!(ok.text().await.unwrap().contains("You can close this tab"));
        assert_eq!(wait.await.unwrap().unwrap(), "ott-123");
    }

    #[tokio::test]
    async fn loopback_reports_provider_errors_and_times_out() {
        let lb = Loopback::bind().await.unwrap();
        let cb = lb.callback_url();
        let wait = tokio::spawn(lb.wait_for_token(Duration::from_secs(10)));
        reqwest::get(format!("{cb}&error=access_denied<script>")).await.unwrap();
        assert_eq!(wait.await.unwrap().unwrap_err(), AuthError::Redirect("access_deniedscript".into()));
        let lb = Loopback::bind().await.unwrap();
        assert_eq!(lb.wait_for_token(Duration::from_millis(50)).await.unwrap_err(), AuthError::Timeout);
    }

    #[test]
    fn callback_parsing() {
        assert_eq!(check_callback("GET /cb?n=abc&ott=t HTTP/1.1\r\n", "abc"), Callback::Token("t".into()));
        assert_eq!(check_callback("GET /cb?n=abd&ott=t HTTP/1.1\r\n", "abc"), Callback::Ignore);
        assert_eq!(check_callback("POST /cb?n=abc&ott=t HTTP/1.1\r\n", "abc"), Callback::Ignore);
        assert_eq!(check_callback("GET /other?n=abc&ott=t HTTP/1.1\r\n", "abc"), Callback::Ignore);
        assert_eq!(check_callback("GET /cb?n=abc HTTP/1.1\r\n", "abc"), Callback::Ignore);
        assert!(Jwt::parse("not-a-jwt").is_err());
    }
}
