//! Worker configuration from the environment (ARCHITECTURE §18, "Worker" row).
//!
//! Values never appear in logs or `Debug` output: only which variables are set.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// The variable names this worker reads, in contract order.
pub const VARIABLES: &[&str] = &[
    "CONVEX_URL",
    "CONVEX_SITE_URL",
    "WORKER_ISSUER",
    "WORKER_SERVICE_PRIVATE_KEY",
    "WORKER_SERVICE_KID",
    "TOKEN_ENC_KEYS",
    "STATE_KEY",
    "SPOOL_KEY",
    "LINK_SIGNING_KEY",
    "IMG_PROXY_KEY",
    "READ_RECEIPT_KEY",
    "GOOGLE_MAIL_CLIENT_ID",
    "GOOGLE_MAIL_CLIENT_SECRET",
    "MICROSOFT_MAIL_CLIENT_ID",
    "MICROSOFT_MAIL_CLIENT_SECRET",
    "GMAIL_PUBSUB_TOPIC",
    "GMAIL_PUSH_AUDIENCE",
    "GMAIL_PUSH_SERVICE_ACCOUNT",
    "SYNC_PUSH",
    "SHARD_COUNT",
    "CORS_ORIGINS",
    "PUBLIC_BASE_URL",
    "API_INTERNAL_URL",
    "BIND_ADDR",
    // Optional (§18): where the upload spool lives (default /var/lib/akatsuki/spool), D-371.
    "SPOOL_DIR",
    // Optional: extra trusted roots (PEM) for IMAP/SMTP servers with a private CA (D-381).
    "MAIL_CA_FILE",
    // Optional: `1` lets IMAP/SMTP reach loopback and private addresses (local harness, LAN self-hosting; D-398).
    "MAIL_ALLOW_PRIVATE_HOSTS",
];

/// Variables without which the worker cannot serve production traffic.
pub const REQUIRED_IN_PRODUCTION: &[&str] = &[
    "CONVEX_URL",
    "CONVEX_SITE_URL",
    "WORKER_ISSUER",
    "WORKER_SERVICE_PRIVATE_KEY",
    "WORKER_SERVICE_KID",
    "TOKEN_ENC_KEYS",
    "STATE_KEY",
    "SPOOL_KEY",
    "LINK_SIGNING_KEY",
    "IMG_PROXY_KEY",
    "READ_RECEIPT_KEY",
    "PUBLIC_BASE_URL",
];

/// Default listen address (local development). The container sets `BIND_ADDR=0.0.0.0:8787`
/// and Compose publishes it on a localhost port for host nginx.
pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";

/// A 32-byte key.
#[derive(Clone, PartialEq, Eq)]
pub struct Key32(pub [u8; 32]);

impl std::fmt::Debug for Key32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key32(<redacted>)")
    }
}

/// A configuration problem. Never contains a secret value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A variable is malformed.
    #[error("{name} is invalid: {reason}")]
    Invalid {
        /// Variable name.
        name: &'static str,
        /// What is wrong (never the value).
        reason: &'static str,
    },
}

/// Push mode (`SYNC_PUSH`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushMode {
    /// Gmail Pub/Sub and Graph webhooks (when configured), with a safety poll.
    On,
    /// Polling only (self-hosting, no public URL).
    Off,
}

/// The parsed configuration.
#[derive(Clone)]
pub struct Config {
    /// Listen address.
    pub bind_addr: SocketAddr,
    /// Convex deployment URL.
    pub convex_url: Option<String>,
    /// Convex site URL (user JWT issuer and JWKS host).
    pub convex_site_url: Option<String>,
    /// Service JWT issuer.
    pub worker_issuer: Option<String>,
    /// PKCS#8 PEM P-256 key.
    pub worker_service_private_key: Option<String>,
    /// Key id.
    pub worker_service_kid: Option<String>,
    /// `TOKEN_ENC_KEYS`, first is active.
    pub token_enc_keys: Vec<(String, Key32)>,
    /// OAuth state sealing key.
    pub state_key: Option<Key32>,
    /// Upload spool key.
    pub spool_key: Option<Key32>,
    /// `/v1/l` capability key.
    pub link_signing_key: Option<Key32>,
    /// `/v1/img` signing key.
    pub img_proxy_key: Option<Key32>,
    /// Read-receipt token key.
    pub read_receipt_key: Option<Key32>,
    /// Gmail OAuth client (mailbox access).
    pub google_mail_client: Option<(String, String)>,
    /// Microsoft OAuth client (mailbox access).
    pub microsoft_mail_client: Option<(String, String)>,
    /// Gmail Pub/Sub topic.
    pub gmail_pubsub_topic: Option<String>,
    /// Expected `aud` of Pub/Sub push JWTs.
    pub gmail_push_audience: Option<String>,
    /// Expected `email` of Pub/Sub push JWTs.
    pub gmail_push_service_account: Option<String>,
    /// Push mode.
    pub push: PushMode,
    /// Shard count (1 in v1).
    pub shard_count: u32,
    /// Exact CORS origins.
    pub cors_origins: Vec<String>,
    /// Public base URL (`https://sync.akatsuki.vc`).
    pub public_base_url: Option<String>,
    /// apps/api over the Docker network.
    pub api_internal_url: Option<String>,
    /// Upload spool directory (`SPOOL_DIR`, default `/var/lib/akatsuki/spool`).
    pub spool_dir: Option<String>,
    /// PEM file of extra trusted roots for IMAP and SMTP (`MAIL_CA_FILE`, D-381).
    pub mail_ca_file: Option<String>,
    /// `MAIL_ALLOW_PRIVATE_HOSTS=1`: IMAP/SMTP may reach loopback and private addresses, on any
    /// port (D-398). Off in the hosted worker.
    pub mail_allow_private_hosts: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field("push", &self.push)
            .field("shard_count", &self.shard_count)
            .field("cors_origins", &self.cors_origins.len())
            .field("token_enc_keys", &self.token_enc_keys.len())
            .finish_non_exhaustive()
    }
}

fn key32(name: &'static str, value: &str) -> Result<Key32, ConfigError> {
    let bytes = STANDARD.decode(value.trim()).map_err(|_| ConfigError::Invalid { name, reason: "not base64" })?;
    let arr: [u8; 32] =
        bytes.try_into().map_err(|_| ConfigError::Invalid { name, reason: "must decode to 32 bytes" })?;
    Ok(Key32(arr))
}

/// Parses `TOKEN_ENC_KEYS="k2:<base64 32B>,k1:<base64 32B>"` (first is active).
pub fn parse_token_keys(value: &str) -> Result<Vec<(String, Key32)>, ConfigError> {
    const NAME: &str = "TOKEN_ENC_KEYS";
    let mut out: Vec<(String, Key32)> = Vec::new();
    for part in value.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (id, b64) =
            part.split_once(':').ok_or(ConfigError::Invalid { name: NAME, reason: "entries must be id:base64" })?;
        let id = id.trim();
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(ConfigError::Invalid { name: NAME, reason: "key ids must be alphanumeric" });
        }
        if out.iter().any(|(k, _)| k == id) {
            return Err(ConfigError::Invalid { name: NAME, reason: "duplicate key id" });
        }
        out.push((id.to_owned(), key32(NAME, b64)?));
    }
    Ok(out)
}

impl Config {
    /// Reads the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let vars: BTreeMap<String, String> =
            VARIABLES.iter().filter_map(|n| std::env::var(n).ok().map(|v| ((*n).to_owned(), v))).collect();
        Self::from_map(&vars)
    }

    /// Builds the configuration from a map (tests, `from_env`). Empty values count as unset.
    pub fn from_map(vars: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let get = |n: &str| vars.get(n).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let key = |n: &'static str| get(n).map(|v| key32(n, &v)).transpose();
        let pair = |a: &str, b: &str| get(a).zip(get(b));

        let bind_addr = get("BIND_ADDR")
            .unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned())
            .parse()
            .map_err(|_| ConfigError::Invalid { name: "BIND_ADDR", reason: "must be host:port" })?;
        let shard_count = match get("SHARD_COUNT") {
            Some(v) => v
                .parse::<u32>()
                .ok()
                .filter(|n| *n >= 1)
                .ok_or(ConfigError::Invalid { name: "SHARD_COUNT", reason: "must be a positive integer" })?,
            None => 1,
        };
        let push = match get("SYNC_PUSH").as_deref() {
            None | Some("on") => PushMode::On,
            Some("off") => PushMode::Off,
            Some(_) => return Err(ConfigError::Invalid { name: "SYNC_PUSH", reason: "must be on or off" }),
        };
        let mail_allow_private_hosts = match get("MAIL_ALLOW_PRIVATE_HOSTS").as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            Some(_) => return Err(ConfigError::Invalid { name: "MAIL_ALLOW_PRIVATE_HOSTS", reason: "must be 0 or 1" }),
        };
        let cors_origins: Vec<String> = get("CORS_ORIGINS")
            .map(|v| {
                v.split(',').map(|o| o.trim().trim_end_matches('/').to_owned()).filter(|o| !o.is_empty()).collect()
            })
            .unwrap_or_default();
        for o in &cors_origins {
            if !(o.starts_with("https://") || o.starts_with("http://")) || o.contains('*') {
                return Err(ConfigError::Invalid {
                    name: "CORS_ORIGINS",
                    reason: "origins must be exact http(s) origins",
                });
            }
        }
        Ok(Self {
            bind_addr,
            convex_url: get("CONVEX_URL"),
            convex_site_url: get("CONVEX_SITE_URL").map(|v| v.trim_end_matches('/').to_owned()),
            worker_issuer: get("WORKER_ISSUER"),
            worker_service_private_key: get("WORKER_SERVICE_PRIVATE_KEY").map(|v| v.replace("\\n", "\n")),
            worker_service_kid: get("WORKER_SERVICE_KID"),
            token_enc_keys: get("TOKEN_ENC_KEYS").map(|v| parse_token_keys(&v)).transpose()?.unwrap_or_default(),
            state_key: key("STATE_KEY")?,
            spool_key: key("SPOOL_KEY")?,
            link_signing_key: key("LINK_SIGNING_KEY")?,
            img_proxy_key: key("IMG_PROXY_KEY")?,
            read_receipt_key: key("READ_RECEIPT_KEY")?,
            google_mail_client: pair("GOOGLE_MAIL_CLIENT_ID", "GOOGLE_MAIL_CLIENT_SECRET"),
            microsoft_mail_client: pair("MICROSOFT_MAIL_CLIENT_ID", "MICROSOFT_MAIL_CLIENT_SECRET"),
            gmail_pubsub_topic: get("GMAIL_PUBSUB_TOPIC"),
            gmail_push_audience: get("GMAIL_PUSH_AUDIENCE"),
            gmail_push_service_account: get("GMAIL_PUSH_SERVICE_ACCOUNT"),
            push,
            shard_count,
            cors_origins,
            public_base_url: get("PUBLIC_BASE_URL").map(|v| v.trim_end_matches('/').to_owned()),
            api_internal_url: get("API_INTERNAL_URL"),
            spool_dir: get("SPOOL_DIR"),
            mail_ca_file: get("MAIL_CA_FILE"),
            mail_allow_private_hosts,
        })
    }

    /// Production variables that are not set (names only).
    pub fn missing_for_production(&self) -> Vec<&'static str> {
        let set = |n: &str| -> bool {
            match n {
                "CONVEX_URL" => self.convex_url.is_some(),
                "CONVEX_SITE_URL" => self.convex_site_url.is_some(),
                "WORKER_ISSUER" => self.worker_issuer.is_some(),
                "WORKER_SERVICE_PRIVATE_KEY" => self.worker_service_private_key.is_some(),
                "WORKER_SERVICE_KID" => self.worker_service_kid.is_some(),
                "TOKEN_ENC_KEYS" => !self.token_enc_keys.is_empty(),
                "STATE_KEY" => self.state_key.is_some(),
                "SPOOL_KEY" => self.spool_key.is_some(),
                "LINK_SIGNING_KEY" => self.link_signing_key.is_some(),
                "IMG_PROXY_KEY" => self.img_proxy_key.is_some(),
                "READ_RECEIPT_KEY" => self.read_receipt_key.is_some(),
                "PUBLIC_BASE_URL" => self.public_base_url.is_some(),
                _ => true,
            }
        };
        REQUIRED_IN_PRODUCTION.iter().copied().filter(|n| !set(n)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    const K: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8="; // bytes 0..32, test only

    #[test]
    fn defaults() {
        let c = Config::from_map(&BTreeMap::new()).unwrap();
        assert_eq!(c.bind_addr.to_string(), DEFAULT_BIND_ADDR);
        assert_eq!(c.shard_count, 1);
        assert_eq!(c.push, PushMode::On);
        assert_eq!(c.missing_for_production().len(), REQUIRED_IN_PRODUCTION.len());
        assert!(!c.mail_allow_private_hosts, "IMAP/SMTP reach public addresses only by default");
        let c = Config::from_map(&map(&[("MAIL_ALLOW_PRIVATE_HOSTS", "1")])).unwrap();
        assert!(c.mail_allow_private_hosts);
        assert!(Config::from_map(&map(&[("MAIL_ALLOW_PRIVATE_HOSTS", "yes")])).is_err());
    }

    #[test]
    fn keys_parse_and_never_print() {
        let c = Config::from_map(&map(&[
            ("TOKEN_ENC_KEYS", &format!("k2:{K}, k1:{K}")),
            ("IMG_PROXY_KEY", K),
            ("SYNC_PUSH", "off"),
        ]))
        .unwrap();
        assert_eq!(c.token_enc_keys.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["k2", "k1"]);
        assert_eq!(c.img_proxy_key.as_ref().unwrap().0[31], 31);
        assert_eq!(c.push, PushMode::Off);
        let dbg = format!("{c:?} {:?}", c.img_proxy_key);
        assert!(!dbg.contains(K) && !dbg.contains("AAEC"));
    }

    #[test]
    fn rejects_bad_values_without_echoing_them() {
        for (name, value) in [
            ("TOKEN_ENC_KEYS", "nokey"),
            ("TOKEN_ENC_KEYS", "k1:short"),
            ("STATE_KEY", "c2VjcmV0LXZhbHVl"),
            ("SHARD_COUNT", "0"),
            ("SYNC_PUSH", "maybe"),
            ("BIND_ADDR", "nowhere"),
            ("CORS_ORIGINS", "*"),
        ] {
            let err = Config::from_map(&map(&[(name, value)])).unwrap_err();
            assert!(!err.to_string().contains(value), "{err}");
        }
        assert!(parse_token_keys(&format!("a:{K},a:{K}")).is_err());
    }

    #[test]
    fn cors_and_urls() {
        let c = Config::from_map(&map(&[
            ("CORS_ORIGINS", "https://app.akatsuki.vc, http://localhost:5173/"),
            ("CONVEX_SITE_URL", "https://x.convex.site/"),
        ]))
        .unwrap();
        assert_eq!(c.cors_origins, vec!["https://app.akatsuki.vc", "http://localhost:5173"]);
        assert_eq!(c.convex_site_url.as_deref(), Some("https://x.convex.site"));
    }
}
