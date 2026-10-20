//! Account configuration for the IMAP + SMTP adapter.
//!
//! The worker builds an [`ImapAccountConfig`] from `mailAccounts.imap` (hosts, ports, TLS modes)
//! and the decrypted `accountSecrets` row (`{username, password}`, §14.1). Secrets never appear in
//! `Debug` output.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use akatsuki_core::convex::TlsMode;
use akatsuki_core::sync::IMAP_IDLE_REISSUE_MS;
use akatsuki_core::wire::worker::Endpoint;
use tokio_rustls::rustls::pki_types::CertificateDer;

use crate::TokenSource;

/// How a connection is secured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte (IMAP 993, SMTP 465).
    Implicit,
    /// Plain TCP upgraded with `STARTTLS` before authentication (IMAP 143, SMTP 587).
    StartTls,
    /// No TLS at all. Refused unless the host is a loopback address: it exists for local bridges
    /// and the in-process test servers, never for a remote host.
    Plaintext,
}

impl From<TlsMode> for Security {
    fn from(m: TlsMode) -> Self {
        match m {
            TlsMode::Implicit => Security::Implicit,
            TlsMode::Starttls => Security::StartTls,
        }
    }
}

/// One IMAP or SMTP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerEndpoint {
    /// Host name (or IP literal).
    pub host: String,
    /// Port.
    pub port: u16,
    /// Transport security.
    pub security: Security,
}

impl ServerEndpoint {
    /// Builds an endpoint.
    pub fn new(host: impl Into<String>, port: u16, security: Security) -> Self {
        Self { host: host.into(), port, security }
    }

    /// From the `POST /v1/connect/imap` wire shape.
    pub fn from_wire(e: &Endpoint) -> Self {
        Self { host: e.host.trim().to_owned(), port: e.port, security: e.tls.into() }
    }
}

/// Credentials for IMAP and SMTP (the same login is used for both).
#[derive(Clone)]
pub enum Credentials {
    /// Username and password (or app-specific password).
    Password {
        /// Login name.
        username: String,
        /// Password.
        password: String,
    },
    /// SASL `XOAUTH2` with a bearer token (Gmail / Outlook IMAP with OAuth).
    OAuth2 {
        /// Login name (the mailbox address).
        username: String,
        /// Access tokens.
        tokens: Arc<dyn TokenSource>,
    },
}

impl Credentials {
    /// The login name.
    pub fn username(&self) -> &str {
        match self {
            Credentials::Password { username, .. } | Credentials::OAuth2 { username, .. } => username,
        }
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::Password { .. } => f.write_str("Credentials::Password(<redacted>)"),
            Credentials::OAuth2 { .. } => f.write_str("Credentials::OAuth2(<redacted>)"),
        }
    }
}

/// Transport settings: TLS trust and which addresses may be contacted. Production uses the
/// Mozilla roots from `webpki-roots` only and public addresses only.
#[derive(Clone, Debug, Default)]
pub struct TlsOptions {
    /// Additional trusted roots (self-hosted servers with a private CA; the test servers).
    pub extra_roots: Vec<CertificateDer<'static>>,
    /// Also connect to loopback, private (RFC 1918, ULA), link-local and CGNAT addresses. Off in
    /// the hosted worker, where those are the host's own network (DECISIONS D-398); the local
    /// end-to-end harness and self-hosted installs with a LAN mail server turn it on
    /// (`MAIL_ALLOW_PRIVATE_HOSTS=1`).
    pub allow_private_hosts: bool,
}

/// Network timeouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// TCP connect plus TLS handshake.
    pub connect: Duration,
    /// One IMAP command or SMTP reply.
    pub command: Duration,
    /// SMTP: waiting for the reply to the final `.` of `DATA` (RFC 5321 §4.5.3.2.6: 10 minutes).
    pub smtp_data_end: Duration,
    /// IMAP IDLE is re-issued after this long (§9.6: 25 minutes; RFC 2177 allows 29).
    pub idle_reissue: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(20),
            command: Duration::from_secs(90),
            smtp_data_end: Duration::from_secs(600),
            idle_reissue: Duration::from_millis(IMAP_IDLE_REISSUE_MS.unsigned_abs()),
        }
    }
}

/// Sync tuning (ARCHITECTURE §9.5, §9.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncTuning {
    /// Servers without QRESYNC: the expunge diff (and, without CONDSTORE, the flag scan) runs at
    /// most this often per folder (15 minutes).
    pub flag_scan_every: Duration,
    /// How far back the expunge diff and flag scan look (the 90-day window).
    pub scan_window: Duration,
    /// Folders other than INBOX are checked with `STATUS` this often while idling (5 minutes).
    pub status_sweep_every: Duration,
    /// Polling interval when the server has no IDLE.
    pub poll_every: Duration,
    /// New messages fetched per `changes_since` call before it returns with `more: true`.
    pub max_new_per_page: usize,
}

impl Default for SyncTuning {
    fn default() -> Self {
        Self {
            flag_scan_every: Duration::from_secs(15 * 60),
            scan_window: Duration::from_secs(90 * 24 * 3600),
            status_sweep_every: Duration::from_secs(5 * 60),
            poll_every: Duration::from_secs(120),
            max_new_per_page: 500,
        }
    }
}

/// Everything the adapter needs for one account.
#[derive(Clone, Debug)]
pub struct ImapAccountConfig {
    /// Convex `Id<"mailAccounts">`; scopes computed thread ids (§9.5). Empty while connecting.
    pub account_id: String,
    /// The mailbox address (SMTP envelope sender).
    pub email: String,
    /// IMAP server.
    pub imap: ServerEndpoint,
    /// SMTP submission server.
    pub smtp: ServerEndpoint,
    /// Login.
    pub credentials: Credentials,
    /// TLS trust.
    pub tls: TlsOptions,
    /// Timeouts.
    pub timeouts: Timeouts,
    /// Sync tuning.
    pub tuning: SyncTuning,
    /// `mailAccounts.imap.sentAutoSaved`; `None` detects it from the host ([`sent_auto_saved_for`]).
    pub sent_auto_saved: Option<bool>,
}

impl ImapAccountConfig {
    /// A config with default timeouts and tuning.
    pub fn new(
        account_id: impl Into<String>,
        email: impl Into<String>,
        imap: ServerEndpoint,
        smtp: ServerEndpoint,
        credentials: Credentials,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            email: email.into(),
            imap,
            smtp,
            credentials,
            tls: TlsOptions::default(),
            timeouts: Timeouts::default(),
            tuning: SyncTuning::default(),
            sent_auto_saved: None,
        }
    }

    /// `"user@host"`, the `providerAccountId` of an IMAP account (§3.2).
    pub fn provider_account_id(&self) -> String {
        format!("{}@{}", self.credentials.username(), self.imap.host.to_ascii_lowercase())
    }
}

/// Whether the provider files submitted mail into Sent by itself (§9.5: Gmail, iCloud, Fastmail),
/// so the adapter must not `APPEND` a second copy.
pub fn sent_auto_saved_for(imap_host: &str, x_gm_ext: bool) -> bool {
    let h = imap_host.trim().trim_end_matches('.').to_ascii_lowercase();
    let is = |d: &str| h == d || h.ends_with(&format!(".{d}"));
    x_gm_ext
        || is("gmail.com")
        || is("googlemail.com")
        || is("mail.me.com")
        || is("icloud.com")
        || is("fastmail.com")
        || is("messagingengine.com")
}

/// `true` for `localhost` and loopback IP literals.
pub fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sent_auto_saved_hosts() {
        assert!(sent_auto_saved_for("imap.gmail.com", false));
        assert!(sent_auto_saved_for("imap.mail.me.com", false));
        assert!(sent_auto_saved_for("IMAP.FASTMAIL.COM.", false));
        assert!(sent_auto_saved_for("mail.example.org", true));
        assert!(!sent_auto_saved_for("imap.mail.yahoo.com", false));
        assert!(!sent_auto_saved_for("notgmail.com.evil", false));
    }

    #[test]
    fn loopback_and_redaction() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("[::1]"));
        assert!(is_loopback_host("localhost"));
        assert!(!is_loopback_host("10.0.0.1"));
        assert!(!is_loopback_host("imap.example.com"));
        let c = Credentials::Password { username: "me".into(), password: "hunter2".into() };
        assert!(!format!("{c:?}").contains("hunter2"));
        let cfg = ImapAccountConfig::new(
            "acc",
            "me@example.com",
            ServerEndpoint::new("IMAP.example.com", 993, Security::Implicit),
            ServerEndpoint::new("smtp.example.com", 465, Security::Implicit),
            c,
        );
        assert!(!format!("{cfg:?}").contains("hunter2"));
        assert_eq!(cfg.provider_account_id(), "me@imap.example.com");
    }
}
