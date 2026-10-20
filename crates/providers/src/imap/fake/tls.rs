//! A throwaway self-signed certificate for `127.0.0.1` and the server-side stream type.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio_rustls::rustls::{ServerConfig, crypto};
use tokio_rustls::server::TlsStream;

/// The test certificate and an acceptor for it.
#[derive(Clone)]
pub struct TestTls {
    /// Trust this root in the client (`TlsOptions::extra_roots`).
    pub cert: CertificateDer<'static>,
    pub acceptor: TlsAcceptor,
}

impl TestTls {
    pub fn new() -> Self {
        let ck =
            rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned(), "localhost".to_owned()]).expect("cert");
        let cert = ck.cert.der().clone();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
        let cfg = ServerConfig::builder_with_provider(Arc::new(crypto::aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)
            .expect("server config");
        Self { cert, acceptor: TlsAcceptor::from(Arc::new(cfg)) }
    }
}

/// Plain or TLS server stream.
pub enum ServerIo {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
    /// Placeholder while upgrading.
    Closed,
}

impl ServerIo {
    pub fn is_tls(&self) -> bool {
        matches!(self, ServerIo::Tls(_))
    }
}

impl AsyncRead for ServerIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_read(cx, buf),
            ServerIo::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            ServerIo::Closed => Poll::Ready(Ok(())),
        }
    }
}

impl AsyncWrite for ServerIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_write(cx, buf),
            ServerIo::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            ServerIo::Closed => Poll::Ready(Err(io::ErrorKind::NotConnected.into())),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_flush(cx),
            ServerIo::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
            ServerIo::Closed => Poll::Ready(Ok(())),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerIo::Plain(s) => Pin::new(s).poll_shutdown(cx),
            ServerIo::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            ServerIo::Closed => Poll::Ready(Ok(())),
        }
    }
}
