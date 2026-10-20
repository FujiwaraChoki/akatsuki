//! Server discovery for the "Connect IMAP" form (§6.7 step 5: the client sends hosts, ports and
//! TLS modes; these helpers let it pre-fill them).
//!
//! Order: the built-in table of well-known providers ([`well_known`]), then Mozilla autoconfig
//! (`autoconfig.<domain>`, `<domain>/.well-known/autoconfig`, then the Thunderbird ISPDB). The
//! fetches go through the `reqwest::Client` the caller passes, which in the worker must be the
//! SSRF-guarded client of §14.4 (guarded resolver, no redirects): the domain comes from user input.
//! The resolver only sees DNS names, so a domain that URL parsing would turn into an IP literal
//! (`127.1`, `0x7f.1`) is refused here, and a URL whose user-built host is not a DNS name is
//! never requested (D-407).

use std::time::Duration;

use akatsuki_core::convex::TlsMode;
use akatsuki_core::wire::worker::Endpoint;
use serde::Serialize;

/// Where a result came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    /// The built-in table.
    WellKnown,
    /// The domain's own autoconfig file.
    Autoconfig,
    /// The Thunderbird ISPDB.
    Ispdb,
}

/// How the login name is formed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsernameForm {
    /// The full address.
    Email,
    /// The part before `@`.
    LocalPart,
}

/// Discovered settings, in the `ImapConnect` endpoint shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    /// IMAP endpoint.
    pub imap: Endpoint,
    /// SMTP submission endpoint.
    pub smtp: Endpoint,
    /// The login name to pre-fill.
    pub username: String,
    /// Where it came from.
    pub source: DiscoverySource,
    /// The provider requires an app-specific password for IMAP.
    pub app_password_required: bool,
}

struct Known {
    domains: &'static [&'static str],
    imap: (&'static str, u16, TlsMode),
    smtp: (&'static str, u16, TlsMode),
    username: UsernameForm,
    app_password: bool,
}

const KNOWN: &[Known] = &[
    Known {
        domains: &["icloud.com", "me.com", "mac.com"],
        imap: ("imap.mail.me.com", 993, TlsMode::Implicit),
        smtp: ("smtp.mail.me.com", 587, TlsMode::Starttls),
        username: UsernameForm::Email,
        app_password: true,
    },
    Known {
        domains: &["fastmail.com", "fastmail.fm", "fastmail.net", "fastmail.org", "messagingengine.com"],
        imap: ("imap.fastmail.com", 993, TlsMode::Implicit),
        smtp: ("smtp.fastmail.com", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: true,
    },
    Known {
        domains: &["yahoo.com", "ymail.com", "rocketmail.com", "yahoo.co.uk", "yahoo.fr", "yahoo.de", "yahoo.co.jp"],
        imap: ("imap.mail.yahoo.com", 993, TlsMode::Implicit),
        smtp: ("smtp.mail.yahoo.com", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: true,
    },
    Known {
        domains: &["aol.com"],
        imap: ("imap.aol.com", 993, TlsMode::Implicit),
        smtp: ("smtp.aol.com", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: true,
    },
    Known {
        domains: &["zoho.com", "zohomail.com"],
        imap: ("imap.zoho.com", 993, TlsMode::Implicit),
        smtp: ("smtp.zoho.com", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: false,
    },
    Known {
        domains: &["zoho.eu", "zohomail.eu"],
        imap: ("imap.zoho.eu", 993, TlsMode::Implicit),
        smtp: ("smtp.zoho.eu", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: false,
    },
    Known {
        domains: &["gmail.com", "googlemail.com"],
        imap: ("imap.gmail.com", 993, TlsMode::Implicit),
        smtp: ("smtp.gmail.com", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: true,
    },
    Known {
        domains: &["outlook.com", "hotmail.com", "live.com", "msn.com"],
        imap: ("outlook.office365.com", 993, TlsMode::Implicit),
        smtp: ("smtp-mail.outlook.com", 587, TlsMode::Starttls),
        username: UsernameForm::Email,
        app_password: true,
    },
    Known {
        domains: &["gmx.com", "gmx.net", "gmx.de"],
        imap: ("imap.gmx.net", 993, TlsMode::Implicit),
        smtp: ("mail.gmx.net", 587, TlsMode::Starttls),
        username: UsernameForm::Email,
        app_password: false,
    },
    Known {
        domains: &["mailbox.org"],
        imap: ("imap.mailbox.org", 993, TlsMode::Implicit),
        smtp: ("smtp.mailbox.org", 465, TlsMode::Implicit),
        username: UsernameForm::Email,
        app_password: false,
    },
];

/// The domain of an address, lowercased, if it is a plausible public DNS name.
pub fn email_domain(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    if local.is_empty() {
        return None;
    }
    let d = domain.trim_end_matches('.').to_ascii_lowercase();
    let ok = !d.is_empty()
        && d.len() <= 253
        && d.contains('.')
        && d.parse::<std::net::IpAddr>().is_err()
        && d != "localhost"
        && !d.ends_with(".localhost")
        && !d.ends_with(".local")
        && !d.ends_with(".internal")
        && d.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && !ends_in_a_number(&d);
    ok.then_some(d)
}

/// The WHATWG URL parser treats a host whose last label is a number (`127.1`, `0x7f.1`,
/// `169.254.43518`) as an IPv4 address, so such a "domain" would become an IP literal once put
/// into a URL. No real top-level domain looks like that (D-407).
fn ends_in_a_number(domain: &str) -> bool {
    let last = domain.rsplit('.').next().unwrap_or("");
    let hex = last.strip_prefix("0x").or_else(|| last.strip_prefix("0X"));
    last.bytes().all(|b| b.is_ascii_digit()) || hex.is_some_and(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Whether a template puts the user's domain into the URL's host.
fn host_from_domain(template: &str) -> bool {
    template.split_once("://").is_some_and(|(_, rest)| rest.split(['/', '?']).next().unwrap_or("").contains("{domain}"))
}

fn local_part(email: &str) -> &str {
    email.trim().rsplit_once('@').map_or(email, |(l, _)| l)
}

/// The built-in table.
pub fn well_known(email: &str) -> Option<Discovered> {
    let d = email_domain(email)?;
    let k = KNOWN.iter().find(|k| k.domains.contains(&d.as_str()))?;
    Some(Discovered {
        imap: Endpoint { host: k.imap.0.into(), port: k.imap.1, tls: k.imap.2 },
        smtp: Endpoint { host: k.smtp.0.into(), port: k.smtp.1, tls: k.smtp.2 },
        username: match k.username {
            UsernameForm::Email => email.trim().to_owned(),
            UsernameForm::LocalPart => local_part(email).to_owned(),
        },
        source: DiscoverySource::WellKnown,
        app_password_required: k.app_password,
    })
}

fn server(node: roxmltree::Node<'_, '_>, email: &str, domain: &str) -> Option<(Endpoint, String)> {
    let text = |name: &str| node.children().find(|c| c.has_tag_name(name)).and_then(|c| c.text()).map(str::trim);
    let host = text("hostname")?.to_ascii_lowercase();
    let port: u16 = text("port")?.parse().ok()?;
    let tls = match text("socketType")?.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => TlsMode::Implicit,
        "STARTTLS" => TlsMode::Starttls,
        _ => return None, // plain is never offered
    };
    let auth_ok = node
        .children()
        .filter(|c| c.has_tag_name("authentication"))
        .filter_map(|c| c.text())
        .any(|a| matches!(a.trim(), "password-cleartext" | "plain" | "password-encrypted" | "secure"));
    let has_auth = node.children().any(|c| c.has_tag_name("authentication"));
    if has_auth && !auth_ok {
        return None; // OAuth2-only or NTLM/GSSAPI-only entries
    }
    if host.is_empty() || host.contains(['/', ' ', '%']) {
        return None;
    }
    let user = text("username").unwrap_or("%EMAILADDRESS%");
    let user = user
        .replace("%EMAILADDRESS%", email.trim())
        .replace("%EMAILLOCALPART%", local_part(email))
        .replace("%EMAILDOMAIN%", domain);
    Some((Endpoint { host, port, tls }, user))
}

/// Parses a Mozilla `clientConfig` document (config-v1.1).
pub fn parse_autoconfig(xml: &str, email: &str, source: DiscoverySource) -> Option<Discovered> {
    let domain = email_domain(email)?;
    let doc = roxmltree::Document::parse(xml).ok()?;
    let provider = doc.descendants().find(|n| n.has_tag_name("emailProvider"))?;
    let imap = provider
        .children()
        .filter(|n| n.has_tag_name("incomingServer") && n.attribute("type") == Some("imap"))
        .find_map(|n| server(n, email, &domain))?;
    let smtp = provider
        .children()
        .filter(|n| n.has_tag_name("outgoingServer") && n.attribute("type") == Some("smtp"))
        .find_map(|n| server(n, email, &domain))?;
    Some(Discovered { imap: imap.0, smtp: smtp.0, username: imap.1, source, app_password_required: false })
}

/// URL templates (`{domain}`, `{email}`), tried in order after the built-in table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoconfigSources {
    /// `(template, source)`.
    pub urls: Vec<(String, DiscoverySource)>,
    /// Per request.
    pub timeout: Duration,
    /// Largest accepted document.
    pub max_bytes: usize,
}

impl Default for AutoconfigSources {
    fn default() -> Self {
        Self {
            urls: vec![
                (
                    "https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}".into(),
                    DiscoverySource::Autoconfig,
                ),
                (
                    "https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={email}".into(),
                    DiscoverySource::Autoconfig,
                ),
                ("https://autoconfig.thunderbird.net/v1.1/{domain}".into(), DiscoverySource::Ispdb),
            ],
            timeout: Duration::from_secs(5),
            max_bytes: 256 * 1024,
        }
    }
}

async fn fetch_text(http: &reqwest::Client, url: &str, sources: &AutoconfigSources) -> Option<String> {
    let mut resp = http.get(url).timeout(sources.timeout).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    if resp.content_length().is_some_and(|n| n > sources.max_bytes as u64) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        body.extend_from_slice(&chunk);
        if body.len() > sources.max_bytes {
            return None;
        }
    }
    String::from_utf8(body).ok()
}

/// Discovers IMAP/SMTP settings for an address. `None` when nothing was found; the user then
/// types the hosts.
pub async fn discover(http: &reqwest::Client, email: &str, sources: &AutoconfigSources) -> Option<Discovered> {
    if let Some(d) = well_known(email) {
        return Some(d);
    }
    let domain = email_domain(email)?;
    let enc_email: String = url::form_urlencoded::byte_serialize(email.trim().as_bytes()).collect();
    for (template, source) in &sources.urls {
        let url = template.replace("{domain}", &domain).replace("{email}", &enc_email);
        // A host built from user input must still be a DNS name after URL parsing: an IP literal would
        // skip the guarded resolver (D-407).
        if host_from_domain(template) && reqwest::Url::parse(&url).ok().and_then(|u| u.domain().map(str::len)).is_none()
        {
            continue;
        }
        if let Some(xml) = fetch_text(http, &url, sources).await
            && let Some(d) = parse_autoconfig(&xml, email, *source)
        {
            return Some(d);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const XML: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <domain>example.org</domain>
    <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
    <incomingServer type="imap">
      <hostname>plain.example.org</hostname><port>143</port><socketType>plain</socketType>
    </incomingServer>
    <incomingServer type="imap">
      <hostname>IMAP.example.org</hostname><port>993</port><socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>587</port><socketType>STARTTLS</socketType>
      <username>%EMAILADDRESS%</username><authentication>OAuth2</authentication><authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn table_and_domains() {
        let d = well_known("Someone@iCloud.com").unwrap();
        assert_eq!(d.imap, Endpoint { host: "imap.mail.me.com".into(), port: 993, tls: TlsMode::Implicit });
        assert_eq!(d.smtp.tls, TlsMode::Starttls);
        assert!(d.app_password_required);
        assert_eq!(well_known("a@zoho.eu").unwrap().imap.host, "imap.zoho.eu");
        assert_eq!(well_known("a@fastmail.fm").unwrap().smtp.port, 465);
        assert!(well_known("a@example.org").is_none());
        assert_eq!(email_domain("a@Example.ORG."), Some("example.org".into()));
        for bad in ["a@localhost", "a@127.0.0.1", "a@foo", "@x.com", "a@x..com", "a@-x.com", "a@db.internal"] {
            assert_eq!(email_domain(bad), None, "{bad}");
        }
        // WHATWG IPv4 shorthands that URL parsing turns into internal addresses (D-407).
        for bad in ["a@127.1", "a@0x7f.1", "a@169.254.43518", "a@172.18.1", "a@10.0x1", "a@example.0X1f", "a@1.2.3.4."]
        {
            assert_eq!(email_domain(bad), None, "{bad}");
        }
        // Digits elsewhere are fine.
        assert_eq!(email_domain("a@123.example.com"), Some("123.example.com".into()));
        assert_eq!(email_domain("a@mail.0xab.org"), Some("mail.0xab.org".into()));
    }

    #[test]
    fn a_user_controlled_host_must_stay_a_dns_name() {
        assert!(host_from_domain("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}"));
        assert!(host_from_domain("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"));
        assert!(!host_from_domain("https://autoconfig.thunderbird.net/v1.1/{domain}"));
        // What the URL parser makes of a shorthand that slipped through.
        let u = reqwest::Url::parse("https://169.254.43518/.well-known/autoconfig").unwrap();
        assert!(u.domain().is_none(), "parsed as {:?}", u.host());
    }

    #[tokio::test]
    async fn numeric_hosts_are_never_fetched() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_string(XML)).mount(&server).await;
        // A template whose host is the user's domain: `{domain}` = "127.1" would be 127.0.0.1.
        let sources = AutoconfigSources {
            urls: vec![("http://{domain}/x".into(), DiscoverySource::Autoconfig)],
            ..Default::default()
        };
        let http = reqwest::Client::new();
        assert!(discover(&http, "a@127.1", &sources).await.is_none());
        assert!(server.received_requests().await.unwrap_or_default().is_empty());
    }

    #[test]
    fn parses_client_config() {
        let d = parse_autoconfig(XML, "jane@example.org", DiscoverySource::Ispdb).unwrap();
        assert_eq!(d.imap, Endpoint { host: "imap.example.org".into(), port: 993, tls: TlsMode::Implicit });
        assert_eq!(d.smtp, Endpoint { host: "smtp.example.org".into(), port: 587, tls: TlsMode::Starttls });
        assert_eq!(d.username, "jane");
        assert!(parse_autoconfig("<nope", "jane@example.org", DiscoverySource::Ispdb).is_none());
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["imap"]["tls"], "implicit");
        assert_eq!(json["source"], "ispdb");
    }

    #[tokio::test]
    async fn discovers_through_sources_in_order() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/own/example.org"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/ispdb/example.org"))
            .respond_with(ResponseTemplate::new(200).set_body_string(XML))
            .mount(&server)
            .await;
        let sources = AutoconfigSources {
            urls: vec![
                (format!("{}/own/{{domain}}", server.uri()), DiscoverySource::Autoconfig),
                (format!("{}/ispdb/{{domain}}", server.uri()), DiscoverySource::Ispdb),
            ],
            ..Default::default()
        };
        let http = reqwest::Client::new();
        let d = discover(&http, "jane@example.org", &sources).await.unwrap();
        assert_eq!(d.source, DiscoverySource::Ispdb);
        assert_eq!(d.imap.host, "imap.example.org");
        // The table wins without any request.
        assert_eq!(discover(&http, "x@yahoo.com", &sources).await.unwrap().source, DiscoverySource::WellKnown);
        // Unknown domains give nothing; invalid ones are not fetched.
        assert!(discover(&http, "x@unknown.example", &sources).await.is_none());
        assert!(discover(&http, "x@localhost", &sources).await.is_none());
        let big = AutoconfigSources { max_bytes: 10, ..sources };
        assert!(discover(&http, "jane@example.org", &big).await.is_none());
    }
}
