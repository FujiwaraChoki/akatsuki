//! TCP + rustls transport shared by IMAP and SMTP.

use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore, crypto};

use super::config::TlsOptions;

/// Why a connection could not be established. Maps onto the `POST /v1/connect/imap` error codes
/// (`unreachable`, `tls_failed`) and onto [`crate::ProviderError::Transient`] during sync.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    /// DNS, refused, timeout before the TCP connection was up.
    #[error("unreachable: {0}")]
    Unreachable(String),
    /// TLS handshake or certificate failure, or the server refused STARTTLS.
    #[error("tls: {0}")]
    Tls(String),
    /// The connection broke after it was established.
    #[error("io: {0}")]
    Io(String),
}

/// A plain or TLS stream.
pub enum MailStream {
    /// Plain TCP (before STARTTLS, or loopback plaintext).
    Plain(TcpStream),
    /// TLS over TCP.
    Tls(Box<TlsStream<TcpStream>>),
}

impl MailStream {
    /// `true` once TLS is up.
    pub fn is_tls(&self) -> bool {
        matches!(self, MailStream::Tls(_))
    }
}

impl fmt::Debug for MailStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_tls() { "MailStream::Tls" } else { "MailStream::Plain" })
    }
}

impl AsyncRead for MailStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for MailStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_flush(cx),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            MailStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            MailStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// Parses PEM `CERTIFICATE` blocks into extra trusted roots ([`TlsOptions::extra_roots`]): the
/// worker's `MAIL_CA_FILE` for self-hosted mail servers with a private CA (D-381). Errors name the
/// problem, never the file content.
pub fn parse_pem_roots(pem: &[u8]) -> Result<Vec<tokio_rustls::rustls::pki_types::CertificateDer<'static>>, String> {
    use tokio_rustls::rustls::pki_types::CertificateDer;
    use tokio_rustls::rustls::pki_types::pem::PemObject as _;
    let roots: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("not a PEM certificate list: {e:?}"))?;
    if roots.is_empty() {
        return Err("no CERTIFICATE block found".into());
    }
    let mut store = RootCertStore::empty();
    for c in &roots {
        store.add(c.clone()).map_err(|e| format!("unusable certificate: {e}"))?;
    }
    Ok(roots)
}

fn client_config(opts: &TlsOptions) -> Result<Arc<ClientConfig>, TransportError> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for c in &opts.extra_roots {
        roots.add(c.clone()).map_err(|e| TransportError::Tls(format!("bad extra root: {e}")))?;
    }
    // Pick the provider explicitly: the workspace links aws-lc-rs, and an implicit default would
    // panic if a second provider ever ends up in the tree.
    let cfg = ClientConfig::builder_with_provider(Arc::new(crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| TransportError::Tls(e.to_string()))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

fn connector(opts: &TlsOptions) -> Result<TlsConnector, TransportError> {
    static DEFAULT: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    let cfg = if opts.extra_roots.is_empty() {
        match DEFAULT.get() {
            Some(c) => c.clone(),
            None => {
                let c = client_config(opts)?;
                DEFAULT.get_or_init(|| c).clone()
            }
        }
    } else {
        client_config(opts)?
    };
    Ok(TlsConnector::from(cfg))
}

/// Resolves `host` once and keeps the addresses that may be contacted: public unicast only, unless
/// `allow_private` (DECISIONS D-398). A host with no allowed address is `Unreachable`, exactly
/// like one that does not answer, so the refusal tells nothing about the network behind it.
pub async fn vetted_addrs(host: &str, port: u16, allow_private: bool) -> Result<Vec<SocketAddr>, TransportError> {
    let host_only = host.trim().trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = match host_only.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::net::lookup_host((host_only, port))
            .await
            .map_err(|e| TransportError::Unreachable(e.kind().to_string()))?
            .collect(),
    };
    let ok: Vec<SocketAddr> =
        addrs.into_iter().filter(|a| allow_private || akatsuki_core::net::ip_allowed(a.ip())).collect();
    if ok.is_empty() {
        return Err(TransportError::Unreachable("no usable address".into()));
    }
    Ok(ok)
}

/// Opens a TCP connection with `TCP_NODELAY` to the first vetted address of `host` that answers
/// ([`vetted_addrs`]; the connection uses exactly those addresses, so there is no second lookup to
/// race).
pub async fn tcp_connect(
    host: &str,
    port: u16,
    timeout: Duration,
    allow_private: bool,
) -> Result<TcpStream, TransportError> {
    let work = async {
        let mut last = TransportError::Unreachable("no usable address".into());
        for addr in vetted_addrs(host, port, allow_private).await? {
            match TcpStream::connect(addr).await {
                Ok(s) => return Ok(s),
                Err(e) => last = TransportError::Unreachable(e.kind().to_string()),
            }
        }
        Err(last)
    };
    let stream = match tokio::time::timeout(timeout, work).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(TransportError::Unreachable("connect timed out".into())),
    };
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// Runs the TLS handshake on an established TCP stream.
pub async fn upgrade(
    tcp: TcpStream,
    host: &str,
    opts: &TlsOptions,
    timeout: Duration,
) -> Result<MailStream, TransportError> {
    let host_only = host.trim().trim_start_matches('[').trim_end_matches(']');
    let name =
        ServerName::try_from(host_only.to_owned()).map_err(|_| TransportError::Tls("invalid host name".into()))?;
    let fut = connector(opts)?.connect(name, tcp);
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(s)) => Ok(MailStream::Tls(Box::new(s))),
        Ok(Err(e)) => Err(TransportError::Tls(e.to_string())),
        Err(_) => Err(TransportError::Tls("handshake timed out".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn private_and_loopback_addresses_are_refused_unless_allowed() {
        for host in [
            "127.0.0.1",
            "10.0.0.5",
            "172.17.0.1",
            "192.168.1.10",
            "169.254.169.254",
            "100.64.0.1",
            "[::1]",
            "localhost",
        ] {
            let e = vetted_addrs(host, 993, false).await.unwrap_err();
            assert!(matches!(e, TransportError::Unreachable(_)), "{host}: {e:?}");
            let e = tcp_connect(host, 993, Duration::from_secs(2), false).await.unwrap_err();
            assert!(matches!(e, TransportError::Unreachable(_)), "{host}: {e:?}");
        }
        assert_eq!(vetted_addrs("127.0.0.1", 993, true).await.unwrap(), vec!["127.0.0.1:993".parse().unwrap()]);
        assert_eq!(vetted_addrs("8.8.8.8", 993, false).await.unwrap(), vec!["8.8.8.8:993".parse().unwrap()]);
    }

    #[tokio::test]
    async fn an_allowed_loopback_listener_is_reached() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(tcp_connect("127.0.0.1", port, Duration::from_secs(2), true).await.is_ok());
        assert!(tcp_connect("127.0.0.1", port, Duration::from_secs(2), false).await.is_err());
    }

    #[test]
    fn pem_roots_parse_and_reject_junk() {
        let ck = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).expect("cert");
        let two = format!("{}\n{}", ck.cert.pem(), ck.cert.pem());
        let roots = parse_pem_roots(two.as_bytes()).expect("roots");
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].as_ref(), ck.cert.der().as_ref());
        assert!(parse_pem_roots(b"").is_err());
        assert!(parse_pem_roots(b"not pem at all").is_err());
        let err = parse_pem_roots(b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n").unwrap_err();
        assert!(!err.contains("AAAA"), "{err}");
    }
}
