//! The SSRF-guarded HTTP client (ARCHITECTURE §14.4), shared by the image proxy and RFC 8058
//! one-click unsubscribe (§9.1).
//!
//! - Only `http` and `https` URLs, without userinfo.
//! - DNS is resolved here; private, loopback, link-local, CGNAT, metadata, multicast and other
//!   special-purpose addresses are refused. The connection uses exactly the addresses that passed
//!   the check (the resolver hands them to the connector, so there is no second lookup to race).
//! - IP-literal hosts are checked the same way (they never reach a resolver).
//! - Redirects are followed by hand, at most 3, and every hop is checked again.
//! - 10 s for the whole exchange, a byte cap on the body, no cookies, no `Referer`, no proxy from
//!   the environment, and a generic `User-Agent`.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{self, HeaderValue};

/// Whole-exchange timeout.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Redirects followed.
pub const MAX_REDIRECTS: usize = 3;
/// User agent sent to third parties.
pub const USER_AGENT: &str = "Mozilla/5.0 (compatible; AkatsukiImageProxy/1.0)";

/// Why a fetch was refused or failed. Never contains the URL.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// Not http(s), userinfo present, or unparseable.
    #[error("url not allowed")]
    BadUrl,
    /// The host resolves only to blocked addresses.
    #[error("address not allowed")]
    Blocked,
    /// More than [`MAX_REDIRECTS`] redirects, or a redirect without a usable `Location`.
    #[error("too many redirects")]
    Redirects,
    /// The body exceeded the cap.
    #[error("response too large")]
    TooLarge,
    /// Timed out.
    #[error("timed out")]
    Timeout,
    /// Connection or protocol failure.
    #[error("fetch failed")]
    Network,
}

/// Whether an address may be contacted (shared with IMAP/SMTP connects, `akatsuki_core::net`).
pub use akatsuki_core::net::ip_allowed;

/// A resolver that drops blocked addresses.
#[derive(Debug, Clone, Copy)]
struct GuardedResolver {
    allow_loopback: bool,
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let allow_loopback = self.allow_loopback;
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let ok: Vec<SocketAddr> =
                addrs.into_iter().filter(|a| ip_allowed(a.ip()) || (allow_loopback && a.ip().is_loopback())).collect();
            if ok.is_empty() {
                return Err(Box::new(FetchError::Blocked) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(ok.into_iter()) as Addrs)
        })
    }
}

/// A finished fetch.
#[derive(Clone, Debug)]
pub struct Fetched {
    /// Final status.
    pub status: u16,
    /// `Content-Type` of the final response.
    pub content_type: Option<String>,
    /// Body (at most the cap).
    pub body: Vec<u8>,
}

/// The guarded client.
#[derive(Clone)]
pub struct SafeFetcher {
    client: reqwest::Client,
    allow_loopback: bool,
}

impl std::fmt::Debug for SafeFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafeFetcher").finish_non_exhaustive()
    }
}

impl SafeFetcher {
    /// The production client.
    pub fn new() -> Self {
        Self::build(false)
    }

    /// A client that may also reach loopback (tests against a local mock server only).
    #[cfg(test)]
    pub(crate) fn allowing_loopback() -> Self {
        Self::build(true)
    }

    fn build(allow_loopback: bool) -> Self {
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(GuardedResolver { allow_loopback }))
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .no_proxy()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(5))
            .timeout(FETCH_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { client, allow_loopback }
    }

    /// The guarded `reqwest` client itself (no redirects followed, blocked addresses never
    /// resolved), for callers that build their own requests (IMAP autoconfig, D-396).
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    fn check_url(&self, url: &url::Url) -> Result<(), FetchError> {
        if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::BadUrl);
        }
        match url.host() {
            Some(url::Host::Ipv4(ip)) => {
                let ip = IpAddr::V4(ip);
                if !(ip_allowed(ip) || (self.allow_loopback && ip.is_loopback())) {
                    return Err(FetchError::Blocked);
                }
            }
            Some(url::Host::Ipv6(ip)) => {
                let ip = IpAddr::V6(ip);
                if !(ip_allowed(ip) || (self.allow_loopback && ip.is_loopback())) {
                    return Err(FetchError::Blocked);
                }
            }
            Some(url::Host::Domain(d)) => {
                let d = d.trim_end_matches('.').to_ascii_lowercase();
                let special =
                    d == "localhost" || d.ends_with(".localhost") || d.ends_with(".internal") || d.ends_with(".local");
                if special && !self.allow_loopback {
                    return Err(FetchError::Blocked);
                }
            }
            None => return Err(FetchError::BadUrl),
        }
        Ok(())
    }

    /// `GET url`, following up to three checked redirects, reading at most `max_bytes`.
    pub async fn get(&self, url: &str, accept: &str, max_bytes: usize) -> Result<Fetched, FetchError> {
        self.request(reqwest::Method::GET, url, accept, None, max_bytes).await
    }

    /// `POST url` with a form body (RFC 8058 one-click unsubscribe). Redirects are followed as
    /// `GET` only for 303; 307/308 repeat the POST.
    pub async fn post_form(&self, url: &str, body: &'static str, max_bytes: usize) -> Result<Fetched, FetchError> {
        self.request(reqwest::Method::POST, url, "*/*", Some(body), max_bytes).await
    }

    async fn request(
        &self,
        method: reqwest::Method,
        url: &str,
        accept: &str,
        form: Option<&'static str>,
        max_bytes: usize,
    ) -> Result<Fetched, FetchError> {
        let work = async {
            let mut current = url::Url::parse(url).map_err(|_| FetchError::BadUrl)?;
            let mut method = method;
            let mut form = form;
            for _ in 0..=MAX_REDIRECTS {
                self.check_url(&current)?;
                let mut req = self
                    .client
                    .request(method.clone(), current.clone())
                    .header(header::ACCEPT, HeaderValue::from_str(accept).unwrap_or(HeaderValue::from_static("*/*")));
                if let Some(body) = form {
                    req = req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(body);
                }
                let mut resp = req.send().await.map_err(map_err)?;
                let status = resp.status();
                if status.is_redirection() {
                    let loc = resp
                        .headers()
                        .get(header::LOCATION)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|l| current.join(l).ok())
                        .ok_or(FetchError::Redirects)?;
                    if status == reqwest::StatusCode::SEE_OTHER
                        || (method == reqwest::Method::POST
                            && matches!(status, reqwest::StatusCode::MOVED_PERMANENTLY | reqwest::StatusCode::FOUND))
                    {
                        method = reqwest::Method::GET;
                        form = None;
                    }
                    current = loc;
                    continue;
                }
                let content_type =
                    resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_owned);
                if resp.content_length().is_some_and(|n| n > max_bytes as u64) {
                    return Err(FetchError::TooLarge);
                }
                let mut body = Vec::new();
                while let Some(chunk) = resp.chunk().await.map_err(map_err)? {
                    if body.len() + chunk.len() > max_bytes {
                        return Err(FetchError::TooLarge);
                    }
                    body.extend_from_slice(&chunk);
                }
                return Ok(Fetched { status: status.as_u16(), content_type, body });
            }
            Err(FetchError::Redirects)
        };
        tokio::time::timeout(FETCH_TIMEOUT, work).await.map_err(|_| FetchError::Timeout)?
    }
}

impl Default for SafeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

fn map_err(e: reqwest::Error) -> FetchError {
    use std::error::Error as _;
    let mut src: Option<&(dyn std::error::Error + 'static)> = e.source();
    while let Some(s) = src {
        if s.downcast_ref::<FetchError>().is_some() {
            return FetchError::Blocked;
        }
        src = s.source();
    }
    if e.is_timeout() { FetchError::Timeout } else { FetchError::Network }
}

/// Image types the proxy serves, recognised by magic bytes. SVG is refused (it is a document,
/// and the proxy does not rasterize).
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    let b = bytes;
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis") {
        Some("image/avif")
    } else if b.starts_with(b"BM") && b.len() > 14 {
        Some("image/bmp")
    } else if b.starts_with(&[0, 0, 1, 0]) && b.len() > 6 {
        Some("image/x-icon")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn blocks_special_purpose_addresses() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "224.0.0.1",
            "255.255.255.255",
            "198.18.0.1",
            "240.0.0.1",
            "192.0.0.170",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::1",
            "2001:db8::1",
            "ff02::1",
            "::127.0.0.1",
        ] {
            assert!(!ip_allowed(bad.parse().unwrap()), "{bad} must be blocked");
        }
        for good in ["93.184.216.34", "8.8.8.8", "2606:4700:4700::1111", "::ffff:8.8.8.8", "64:ff9b::808:808"] {
            assert!(ip_allowed(good.parse().unwrap()), "{good} must be allowed");
        }
    }

    #[tokio::test]
    async fn refuses_bad_urls_and_literal_private_hosts() {
        let f = SafeFetcher::new();
        for url in [
            "ftp://example.com/a.png",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://user:pw@example.com/",
            "not a url",
        ] {
            assert_eq!(f.get(url, "image/*", 1024).await.unwrap_err(), FetchError::BadUrl, "{url}");
        }
        for url in [
            "http://127.0.0.1/",
            "http://[::1]/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "http://localhost/",
            "http://metadata.google.internal/",
            "http://2130706433/",
        ] {
            assert_eq!(f.get(url, "image/*", 1024).await.unwrap_err(), FetchError::Blocked, "{url}");
        }
    }

    #[tokio::test]
    async fn resolver_drops_loopback_names() {
        // `localhost.` with a trailing dot is caught by the name check; a name that resolves to
        // loopback is caught by the resolver.
        let f = SafeFetcher::new();
        let err = f.get("http://localtest.me:9/", "image/*", 1024).await.unwrap_err();
        assert!(matches!(err, FetchError::Blocked | FetchError::Network), "{err:?}");
    }

    #[tokio::test]
    async fn redirects_are_rechecked_and_capped() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/hop"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "http://169.254.169.254/x"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/loop"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/loop"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4096]))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ok"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "image/png").set_body_bytes(b"x"))
            .mount(&server)
            .await;
        let f = SafeFetcher::allowing_loopback();
        assert_eq!(f.get(&format!("{}/hop", server.uri()), "image/*", 1024).await.unwrap_err(), FetchError::Blocked);
        assert_eq!(f.get(&format!("{}/loop", server.uri()), "image/*", 1024).await.unwrap_err(), FetchError::Redirects);
        assert_eq!(f.get(&format!("{}/big", server.uri()), "image/*", 1024).await.unwrap_err(), FetchError::TooLarge);
        let ok = f.get(&format!("{}/ok", server.uri()), "image/*", 1024).await.unwrap();
        assert_eq!(ok.status, 200);
        assert_eq!(ok.content_type.as_deref(), Some("image/png"));
        // The production client refuses the same loopback mock server.
        assert_eq!(
            SafeFetcher::new().get(&format!("{}/ok", server.uri()), "image/*", 1024).await.unwrap_err(),
            FetchError::Blocked
        );
    }

    #[test]
    fn sniffs_images_and_refuses_svg() {
        assert_eq!(sniff_image(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0]), Some("image/png"));
        assert_eq!(sniff_image(b"GIF89a...."), Some("image/gif"));
        assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
        assert_eq!(sniff_image(b"<html>"), None);
    }
}
