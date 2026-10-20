//! Wiring: builds the services, the supervisor and the Convex connection from [`Config`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use akatsuki_convex_api::{Convex, ConvexClientTransport, WorkerApi};
use akatsuki_core::auth::{SERVICE_AUDIENCE, ServiceTokenMinter};
use akatsuki_providers::imap::config::TlsOptions;

use crate::api::Services;
use crate::config::Config;
use crate::crypto::SecretKeyring;
use crate::engine::supervisor::{Supervisor, SupervisorDeps};
use crate::engine::{EngineConfig, ProjectGate, Registry};
use crate::fetch::SafeFetcher;
use crate::metrics::Metrics;
use crate::oauth::{Endpoints, OAuthClient, OAuthKind};
use crate::providers::{AdapterFactory, AdapterImapProbe};
use crate::push_auth::{HttpGoogleKeys, PushVerifier};
use crate::spool::{DEFAULT_SPOOL_DIR, Spool};
use crate::user_convex::HttpUserConvex;

/// Service JWTs are re-minted this often (§6.6; they live 10 minutes).
pub const SERVICE_TOKEN_REFRESH: Duration = Duration::from_secs(300);

/// The shared outbound HTTP client (rustls, no proxy from the environment for provider APIs is
/// left to the deployment).
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("akatsuki-sync/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// Mailbox OAuth clients that are configured (both id and secret, and `PUBLIC_BASE_URL`).
pub fn oauth_clients(config: &Config) -> HashMap<OAuthKind, Arc<OAuthClient>> {
    let mut out = HashMap::new();
    let Some(base) = config.public_base_url.as_deref() else { return out };
    if let Some((id, secret)) = &config.google_mail_client {
        out.insert(
            OAuthKind::Google,
            Arc::new(OAuthClient::new(OAuthKind::Google, id.clone(), secret.clone(), base, Endpoints::default())),
        );
    }
    if let Some((id, secret)) = &config.microsoft_mail_client {
        out.insert(
            OAuthKind::Microsoft,
            Arc::new(OAuthClient::new(OAuthKind::Microsoft, id.clone(), secret.clone(), base, Endpoints::default())),
        );
    }
    out
}

/// A worker instance id: `w-<12 hex>`; the service JWT `sub` is `worker:<id>`.
pub fn instance_id() -> String {
    let b = crate::crypto::random_bytes::<6>().unwrap_or([0; 6]);
    format!("w-{}", hex::encode(b))
}

/// Connects to Convex under the worker's service identity and keeps the JWT fresh.
pub async fn connect_convex(
    config: &Config,
    worker_id: &str,
) -> anyhow::Result<Option<Arc<Convex<ConvexClientTransport>>>> {
    let (Some(url), Some(pem), Some(kid), Some(issuer)) = (
        config.convex_url.as_deref(),
        config.worker_service_private_key.as_deref(),
        config.worker_service_kid.as_deref(),
        config.worker_issuer.as_deref(),
    ) else {
        return Ok(None);
    };
    let minter = Arc::new(
        ServiceTokenMinter::from_pem(pem, kid, issuer, format!("worker:{worker_id}"))
            .map_err(|_| anyhow::anyhow!("WORKER_SERVICE_PRIVATE_KEY is not a P-256 PKCS#8 key"))?,
    );
    let mut attempt = 0u32;
    let transport = loop {
        match ConvexClientTransport::connect(url).await {
            Ok(t) => break t,
            Err(_) if attempt < 6 => {
                attempt += 1;
                tracing::warn!(attempt, "convex connection failed; retrying");
                tokio::time::sleep(akatsuki_core::sync::backoff(attempt, 0.5)).await;
            }
            Err(_) => anyhow::bail!("cannot connect to CONVEX_URL"),
        }
    };
    let convex = Arc::new(Convex::new(transport));
    let token = minter.mint(SERVICE_AUDIENCE).map_err(|_| anyhow::anyhow!("cannot mint the service token"))?;
    convex.set_auth(Some(token)).await;
    let refresher = convex.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SERVICE_TOKEN_REFRESH).await;
            match minter.mint(SERVICE_AUDIENCE) {
                Ok(t) => refresher.set_auth(Some(t)).await,
                Err(_) => tracing::error!("cannot mint the service token"),
            }
        }
    });
    Ok(Some(convex))
}

/// Everything `serve` runs.
pub struct Worker {
    /// HTTP services.
    pub services: Arc<Services>,
    /// The supervisor (run it with a shutdown receiver).
    pub supervisor: Supervisor,
    /// The upload spool, for the sweeper.
    pub spool: Option<Spool>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker").finish_non_exhaustive()
    }
}

/// The IMAP/SMTP trust settings: the Mozilla roots plus `MAIL_CA_FILE` when set (D-381).
///
/// # Errors
/// The file cannot be read or holds no usable certificate (the message never contains its content).
pub fn mail_tls(config: &Config) -> anyhow::Result<TlsOptions> {
    let allow_private_hosts = config.mail_allow_private_hosts;
    if allow_private_hosts {
        tracing::warn!("MAIL_ALLOW_PRIVATE_HOSTS=1: IMAP/SMTP may reach loopback and private addresses");
    }
    let Some(path) = config.mail_ca_file.as_deref() else {
        return Ok(TlsOptions { allow_private_hosts, ..TlsOptions::default() });
    };
    let pem = std::fs::read(path).map_err(|e| anyhow::anyhow!("MAIL_CA_FILE cannot be read: {}", e.kind()))?;
    let extra_roots = akatsuki_providers::imap::tls::parse_pem_roots(&pem)
        .map_err(|e| anyhow::anyhow!("MAIL_CA_FILE is invalid: {e}"))?;
    tracing::info!(roots = extra_roots.len(), "trusting extra IMAP/SMTP roots from MAIL_CA_FILE");
    Ok(TlsOptions { extra_roots, allow_private_hosts })
}

/// Builds services and the supervisor over a Convex API.
pub fn build(
    config: &Config,
    api: Arc<dyn WorkerApi>,
    worker_id: String,
    live: Arc<std::sync::atomic::AtomicBool>,
) -> Worker {
    build_with_tls(config, api, worker_id, live, TlsOptions::default())
}

/// [`build`] with IMAP/SMTP trust settings from [`mail_tls`].
pub fn build_with_tls(
    config: &Config,
    api: Arc<dyn WorkerApi>,
    worker_id: String,
    live: Arc<std::sync::atomic::AtomicBool>,
    mail_tls: TlsOptions,
) -> Worker {
    let http = http_client();
    let metrics = Arc::new(Metrics::default());
    let registry = Arc::new(Registry::default());
    let keyring = Arc::new(SecretKeyring::new(config.token_enc_keys.clone()));
    let oauth = oauth_clients(config);
    let fetcher = SafeFetcher::new();
    let spool = config
        .spool_key
        .as_ref()
        .map(|k| Spool::new(config.spool_dir.clone().unwrap_or_else(|| DEFAULT_SPOOL_DIR.to_owned()), k.0));
    let push_verifier = match (&config.gmail_push_audience, &config.gmail_push_service_account) {
        (Some(aud), Some(sa)) => {
            Some(Arc::new(PushVerifier::new(Arc::new(HttpGoogleKeys::new(http.clone())), aud.clone(), sa.clone())))
        }
        _ => None,
    };
    let user_convex = Arc::new(HttpUserConvex::new(http.clone(), config.convex_url.as_deref().unwrap_or_default()));
    let services = Arc::new(Services::new(
        api.clone(),
        registry.clone(),
        keyring.clone(),
        oauth.clone(),
        http.clone(),
        fetcher.clone(),
        spool.clone(),
        user_convex,
        Arc::new(AdapterImapProbe { mail_tls: mail_tls.clone() }),
        push_verifier,
        metrics.clone(),
    ));
    let triage = crate::triage::http_triage(config, http.clone(), &worker_id);
    let engine = Arc::new(EngineConfig {
        worker_id,
        public_base_url: config.public_base_url.clone(),
        push: config.push,
        gmail_topic: config.gmail_pubsub_topic.is_some(),
        shard_count: config.shard_count,
        triage,
    });
    let supervisor = Supervisor::new(SupervisorDeps {
        api,
        config: engine,
        factory: Arc::new(AdapterFactory::new(config.public_base_url.is_some()).with_mail_tls(mail_tls)),
        keyring,
        oauth,
        registry,
        metrics,
        gate: Arc::new(ProjectGate::default()),
        http,
        fetcher,
        gmail_topic: config.gmail_pubsub_topic.clone(),
        live,
    });
    Worker { services, supervisor, spool }
}
