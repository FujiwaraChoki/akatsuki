//! The provider factory: turns an assigned account plus its decrypted credentials into a
//! `MailProvider` (and, for IMAP, a `PushSource`).
//!
//! The engine only ever sees `Arc<dyn MailProvider>`, so the Gmail, Graph and IMAP adapters
//! (`crates/providers::{gmail,graph,imap}`) plug in at one place: [`AdapterFactory::build`]. Tests
//! use their own factory or build actors directly over `FakeProvider` (DECISIONS D-231).

use std::sync::Arc;

use akatsuki_core::convex::Provider;
use akatsuki_core::convex::worker::AssignedAccount;
use akatsuki_core::wire::ErrorCode;
use akatsuki_core::wire::worker::ImapConnect;
use akatsuki_providers::imap::config::TlsOptions;
use akatsuki_providers::{MailProvider, PushSource, TokenSource};
use async_trait::async_trait;

use crate::config::PushMode;
use crate::oauth::ImapSecret;

/// Credentials handed to an adapter.
#[derive(Clone)]
pub enum Credentials {
    /// OAuth access tokens (Gmail, Graph).
    OAuth(Arc<dyn TokenSource>),
    /// IMAP/SMTP login.
    Imap(ImapSecret),
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credentials::OAuth(_) => f.write_str("Credentials::OAuth"),
            Credentials::Imap(_) => f.write_str("Credentials::Imap"),
        }
    }
}

/// Everything an adapter constructor may need.
#[derive(Clone, Debug)]
pub struct ProviderContext {
    /// The account.
    pub account: AssignedAccount,
    /// Credentials.
    pub credentials: Credentials,
    /// Shared HTTP client (rustls).
    pub http: reqwest::Client,
    /// `GMAIL_PUBSUB_TOPIC` (Gmail watch).
    pub gmail_pubsub_topic: Option<String>,
    /// Push mode.
    pub push: PushMode,
}

/// A built adapter.
pub struct BuiltProvider {
    /// The provider.
    pub provider: Arc<dyn MailProvider>,
    /// IMAP IDLE/NOTIFY stream, owned by the actor.
    pub push: Option<Box<dyn PushSource>>,
}

impl std::fmt::Debug for BuiltProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltProvider").field("kind", &self.provider.kind()).finish_non_exhaustive()
    }
}

/// Why no provider could be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FactoryError {
    /// The adapter for this provider is not compiled in yet.
    #[error("no adapter for {0:?}")]
    AdapterMissing(Provider),
    /// Credentials do not fit the provider.
    #[error("credentials do not match the provider")]
    WrongCredentials,
    /// The adapter refused to start.
    #[error("adapter failed to start")]
    Failed,
}

/// Builds providers.
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    /// Builds the adapter for one account.
    async fn build(&self, ctx: ProviderContext) -> Result<BuiltProvider, FactoryError>;
}

/// The production factory over the adapters in `crates/providers::{gmail,graph,imap}`.
#[derive(Debug, Clone)]
pub struct AdapterFactory {
    /// The process-wide Gmail budget shared by every Gmail adapter (§9.4).
    gmail_process: Arc<akatsuki_providers::gmail::quota::ProcessQuota>,
    /// Whether a public URL exists for Graph change notifications.
    public_url: bool,
    /// Extra trusted roots for IMAP and SMTP (`MAIL_CA_FILE`, D-381).
    mail_tls: TlsOptions,
}

impl Default for AdapterFactory {
    fn default() -> Self {
        Self::new(false)
    }
}

impl AdapterFactory {
    /// A factory; `public_url` says whether `PUBLIC_BASE_URL` is set (Graph webhooks need it).
    pub fn new(public_url: bool) -> Self {
        use akatsuki_providers::gmail::quota::{ProcessQuota, QuotaLimits};
        Self { gmail_process: ProcessQuota::new(&QuotaLimits::default()), public_url, mail_tls: TlsOptions::default() }
    }

    /// Trusts these extra roots on IMAP and SMTP connections (`MAIL_CA_FILE`).
    #[must_use]
    pub fn with_mail_tls(mut self, tls: TlsOptions) -> Self {
        self.mail_tls = tls;
        self
    }
}

/// The IMAP adapter configuration of an account, trusting the default roots only.
pub fn imap_config(
    account: &AssignedAccount,
    secret: &ImapSecret,
) -> Option<akatsuki_providers::imap::ImapAccountConfig> {
    imap_config_with_tls(account, secret, &TlsOptions::default())
}

/// The IMAP adapter configuration of an account with extra trusted roots. `None` unless the
/// account's Convex settings name exactly the endpoints sealed with the password (D-414): the
/// worker connects to the sealed endpoints, never to hosts only Convex vouches for.
pub fn imap_config_with_tls(
    account: &AssignedAccount,
    secret: &ImapSecret,
    tls: &TlsOptions,
) -> Option<akatsuki_providers::imap::ImapAccountConfig> {
    use akatsuki_providers::imap::{Credentials as ImapCredentials, ImapAccountConfig, ServerEndpoint};
    let s = account.imap.as_ref()?;
    let e = secret.bound_endpoints(s)?;
    let mut cfg = ImapAccountConfig::new(
        account.account_id.as_str(),
        account.email.clone(),
        ServerEndpoint::new(e.imap_host.clone(), e.imap_port, e.imap_tls.into()),
        ServerEndpoint::new(e.smtp_host.clone(), e.smtp_port, e.smtp_tls.into()),
        ImapCredentials::Password { username: secret.username.clone(), password: secret.password.clone() },
    );
    cfg.sent_auto_saved = Some(s.sent_auto_saved);
    cfg.tls = tls.clone();
    Some(cfg)
}

#[async_trait]
impl ProviderFactory for AdapterFactory {
    async fn build(&self, ctx: ProviderContext) -> Result<BuiltProvider, FactoryError> {
        use akatsuki_providers::gmail::quota::{AccountQuota, QuotaCosts, QuotaLimits};
        use akatsuki_providers::gmail::{GmailConfig, GmailProvider};
        use akatsuki_providers::graph::{GraphConfig, GraphProvider};
        use akatsuki_providers::imap::{ImapIdle, ImapProvider};
        let push_on = ctx.push == PushMode::On;
        match (ctx.account.provider, ctx.credentials) {
            (Provider::Gmail, Credentials::OAuth(tokens)) => {
                let cfg = GmailConfig {
                    pubsub_topic: if push_on { ctx.gmail_pubsub_topic.clone() } else { None },
                    ..GmailConfig::default()
                };
                let quota = Arc::new(AccountQuota::new(
                    QuotaCosts::default(),
                    &QuotaLimits::default(),
                    self.gmail_process.clone(),
                ));
                Ok(BuiltProvider { provider: Arc::new(GmailProvider::new(cfg, tokens, quota, ctx.http)), push: None })
            }
            (Provider::Graph, Credentials::OAuth(tokens)) => {
                let cfg = GraphConfig {
                    push: push_on && self.public_url,
                    self_addresses: vec![ctx.account.email.trim().to_lowercase()],
                    ..GraphConfig::default()
                };
                Ok(BuiltProvider { provider: Arc::new(GraphProvider::with_http(ctx.http, cfg, tokens)), push: None })
            }
            (Provider::Imap, Credentials::Imap(secret)) => {
                let cfg = imap_config_with_tls(&ctx.account, &secret, &self.mail_tls).ok_or(FactoryError::Failed)?;
                let provider = ImapProvider::connect(cfg.clone()).await.map_err(|_| FactoryError::Failed)?;
                let idle = provider.caps().imap.as_ref().is_some_and(|c| c.idle);
                let push: Option<Box<dyn PushSource>> =
                    (push_on && idle).then(|| Box::new(ImapIdle::new(Arc::new(cfg))) as Box<dyn PushSource>);
                Ok(BuiltProvider { provider: Arc::new(provider), push })
            }
            _ => Err(FactoryError::WrongCredentials),
        }
    }
}

/// What a successful IMAP/SMTP probe learned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImapProbeResult {
    /// `CAPABILITY` after login.
    pub capabilities: Vec<String>,
    /// The server stores sent mail itself (Gmail, iCloud, Fastmail).
    pub sent_auto_saved: bool,
}

/// Tests an IMAP LOGIN and an SMTP AUTH (no DATA) for `POST /v1/connect/imap` (§6.7).
#[async_trait]
pub trait ImapProbe: Send + Sync {
    /// Probes both servers; errors are `imap_auth_failed`, `smtp_auth_failed`, `tls_failed` or
    /// `unreachable`.
    async fn probe(&self, req: &ImapConnect) -> Result<ImapProbeResult, ErrorCode>;
}

/// The production probe: a live IMAP LOGIN and an SMTP AUTH without DATA through
/// `akatsuki_providers::imap::test_connection`.
#[derive(Debug, Default, Clone)]
pub struct AdapterImapProbe {
    /// Extra trusted roots (`MAIL_CA_FILE`, D-381).
    pub mail_tls: TlsOptions,
}

#[async_trait]
impl ImapProbe for AdapterImapProbe {
    async fn probe(&self, req: &ImapConnect) -> Result<ImapProbeResult, ErrorCode> {
        use akatsuki_providers::imap::{
            Credentials as ImapCredentials, ImapAccountConfig, ServerEndpoint, test_connection,
        };
        let mut cfg = ImapAccountConfig::new(
            "",
            req.email.trim().to_lowercase(),
            ServerEndpoint::from_wire(&req.imap),
            ServerEndpoint::from_wire(&req.smtp),
            ImapCredentials::Password { username: req.username.clone(), password: req.password.clone() },
        );
        cfg.tls = self.mail_tls.clone();
        match test_connection(&cfg).await {
            Ok(r) => Ok(ImapProbeResult { capabilities: r.capabilities, sent_auto_saved: r.sent_auto_saved }),
            Err(f) => Err(f.code),
        }
    }
}

/// Hosts that store sent mail themselves (§9.5): Gmail, iCloud, Fastmail.
pub fn imap_sent_auto_saved(imap_host: &str) -> bool {
    let h = imap_host.trim().trim_end_matches('.').to_ascii_lowercase();
    ["gmail.com", "googlemail.com", "mail.me.com", "icloud.com", "fastmail.com", "messagingengine.com"]
        .iter()
        .any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::ImapEndpoints;
    use akatsuki_core::convex::{AccountCaps, AccountStatus, ImapSettings, TlsMode};

    fn imap_account(smtp_host: &str) -> AssignedAccount {
        AssignedAccount {
            account_id: "acc00001".into(),
            user_id: "u1".into(),
            provider: Provider::Imap,
            provider_account_id: "me@imap.example.com".into(),
            email: "me@example.com".into(),
            status: AccountStatus::Live,
            entitled: true,
            server_triage: false,
            imap: Some(ImapSettings {
                host: "imap.example.com".into(),
                port: 993,
                tls: TlsMode::Implicit,
                smtp_host: smtp_host.into(),
                smtp_port: 465,
                smtp_tls: TlsMode::Implicit,
                capabilities: vec![],
                sent_auto_saved: false,
            }),
            caps: AccountCaps { multi_label: false, categories: false, server_search: true, delete_forever: true },
        }
    }

    #[test]
    fn imap_password_goes_only_to_the_sealed_endpoints() {
        let good = imap_account("smtp.example.com");
        let secret = ImapSecret {
            username: "me".into(),
            password: "not-real".into(),
            endpoints: Some(ImapEndpoints::of(good.imap.as_ref().unwrap())),
        };
        let cfg = imap_config(&good, &secret).expect("matching settings connect");
        assert_eq!(cfg.imap.host, "imap.example.com");
        assert_eq!(cfg.smtp.host, "smtp.example.com");
        // Case and a trailing dot are the same host.
        assert!(imap_config(&imap_account("SMTP.example.com."), &secret).is_some());

        // A Convex write that points SMTP (or IMAP, or a port, or TLS) elsewhere is refused.
        assert!(imap_config(&imap_account("mail.attacker.example"), &secret).is_none());
        let mut moved = good.clone();
        moved.imap.as_mut().unwrap().host = "imap.attacker.example".into();
        assert!(imap_config(&moved, &secret).is_none());
        let mut port = good.clone();
        port.imap.as_mut().unwrap().smtp_port = 587;
        assert!(imap_config(&port, &secret).is_none());
        let mut tls = good.clone();
        tls.imap.as_mut().unwrap().tls = TlsMode::Starttls;
        assert!(imap_config(&tls, &secret).is_none());

        // A secret sealed before D-414 has no endpoints: reconnect.
        let legacy = ImapSecret { endpoints: None, ..secret.clone() };
        assert!(imap_config(&good, &legacy).is_none());
        // Old rows still parse.
        let old: ImapSecret = serde_json::from_str(r#"{"username":"me","password":"x"}"#).unwrap();
        assert!(old.endpoints.is_none());
    }

    #[test]
    fn sent_auto_saved_hosts() {
        assert!(imap_sent_auto_saved("imap.gmail.com"));
        assert!(imap_sent_auto_saved("imap.mail.me.com"));
        assert!(imap_sent_auto_saved("imap.fastmail.com"));
        assert!(!imap_sent_auto_saved("mail.example.com"));
        assert!(!imap_sent_auto_saved("evilgmail.com"));
    }
}
