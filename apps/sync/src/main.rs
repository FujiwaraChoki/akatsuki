//! # akatsuki-sync
//!
//! The Akatsuki sync worker (ARCHITECTURE §7, §9, §10, §14): the only holder of provider
//! credentials, the only component that talks to mail providers, and the server of message bodies
//! (which it never stores).
//!
//! It serves the HTTP contract (§7), runs one account actor per mailbox under a Convex lease
//! (§9.2), shuts down gracefully (actors finish their step and release their leases), and logs
//! only content-free metadata.
//!
//! ```text
//! akatsuki-sync                 serve (default)
//! akatsuki-sync healthcheck     GET /healthz on BIND_ADDR; exit 0 when healthy (container check)
//! akatsuki-sync rotate-secrets  re-seal every account secret under the active TOKEN_ENC_KEYS key
//! akatsuki-sync --version
//! ```

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use akatsuki_core::auth::{HttpJwksSource, UserVerifier};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing_subscriber::EnvFilter;

use akatsuki_convex_api::WorkerApi;
use akatsuki_sync::config::Config;
use akatsuki_sync::http::{self, AppState, VerifyUser};
use futures::StreamExt as _;

/// How long in-flight requests and actors get to finish after SIGTERM.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_target(false);
    if std::io::stderr().is_terminal() {
        builder.with_writer(std::io::stderr).init();
    } else {
        builder.json().flatten_event(true).with_current_span(false).with_writer(std::io::stderr).init();
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let missing = config.missing_for_production();
    if !missing.is_empty() {
        tracing::warn!(missing = ?missing, "production variables not set; affected routes refuse requests");
    }
    let verifier: Option<Arc<dyn VerifyUser>> = config.convex_site_url.as_deref().map(|site| {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().unwrap_or_default();
        Arc::new(UserVerifier::new(HttpJwksSource::for_convex_site(http, site), site.to_owned())) as Arc<dyn VerifyUser>
    });
    let addr = config.bind_addr;
    let mail_tls = akatsuki_sync::app::mail_tls(&config)?;
    let worker_id = akatsuki_sync::app::instance_id();
    let convex = akatsuki_sync::app::connect_convex(&config, &worker_id).await?;
    let (tx, rx) = tokio::sync::watch::channel(false);

    let mut state = AppState::new(config.clone(), verifier);
    let mut supervisor_task = None;
    let mut sweeper = None;
    match convex {
        Some(convex) => {
            let api: Arc<dyn WorkerApi> = convex;
            let worker = akatsuki_sync::app::build_with_tls(
                &config,
                api,
                worker_id.clone(),
                state.convex_live.clone(),
                mail_tls,
            );
            state = state.with_services(worker.services.clone());
            let shutdown = rx.clone();
            supervisor_task = Some(tokio::spawn(worker.supervisor.run(shutdown)));
            if let Some(spool) = worker.spool {
                sweeper = Some(tokio::spawn(async move {
                    loop {
                        let removed = spool.sweep().await;
                        if removed > 0 {
                            tracing::info!(removed, "expired uploads removed from the spool");
                        }
                        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
                    }
                }));
            }
            tracing::info!(worker = %worker_id, "account supervisor started");
        }
        None => tracing::warn!("Convex is not configured; serving without the sync engine"),
    }

    let app = http::router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(addr = %listener.local_addr()?, version = env!("CARGO_PKG_VERSION"), "akatsuki-sync listening");

    let mut rx_server = rx.clone();
    let server = axum::serve(listener, app).with_graceful_shutdown(async move {
        let _ = rx_server.wait_for(|stop| *stop).await;
    });
    let mut task = tokio::spawn(async move { server.await });

    tokio::select! {
        res = &mut task => {
            res??;
            return Ok(());
        }
        () = shutdown_signal() => {}
    }
    tracing::info!("shutdown requested; draining");
    // Stop claiming, let every actor finish its step and release its lease (§9.2), then stop HTTP.
    let _ = tx.send(true);
    if let Some(s) = supervisor_task {
        let _ = tokio::time::timeout(SHUTDOWN_GRACE, s).await;
    }
    if let Some(s) = sweeper {
        s.abort();
    }
    match tokio::time::timeout(SHUTDOWN_GRACE, task).await {
        Ok(res) => res??,
        Err(_) => tracing::warn!("drain timed out; exiting"),
    }
    drop(rx);
    tracing::info!("stopped");
    Ok(())
}

/// `rotate-secrets`: re-seal every account secret under the active key (§14.1).
async fn rotate_secrets(config: Config) -> anyhow::Result<bool> {
    if config.token_enc_keys.is_empty() {
        anyhow::bail!("TOKEN_ENC_KEYS is not set");
    }
    let worker_id = format!("rotate-{}", akatsuki_sync::app::instance_id());
    let Some(convex) = akatsuki_sync::app::connect_convex(&config, &worker_id).await? else {
        anyhow::bail!("CONVEX_URL and the worker service key are required");
    };
    let mut accounts = Vec::new();
    for shard in 0..config.shard_count.max(1) {
        let mut sub = convex.assigned_accounts(shard).await.map_err(|_| anyhow::anyhow!("cannot list accounts"))?;
        match tokio::time::timeout(Duration::from_secs(30), sub.next()).await {
            Ok(Some(Ok(rows))) => accounts.extend(rows),
            _ => anyhow::bail!("cannot list accounts"),
        }
    }
    let keyring = akatsuki_sync::crypto::SecretKeyring::new(config.token_enc_keys.clone());
    let report = akatsuki_sync::rotate::rotate_all(convex.as_ref(), &keyring, &worker_id, &accounts).await;
    println!(
        "rotate-secrets: {} accounts, {} re-sealed, {} already current, {} without secret, {} busy, {} unreadable, {} failed",
        accounts.len(),
        report.rotated,
        report.current,
        report.missing,
        report.busy,
        report.unreadable,
        report.failed
    );
    if report.busy > 0 {
        println!(
            "rotate-secrets: busy accounts are re-sealed by the running worker on its next claim; run again later"
        );
    }
    Ok(report.complete())
}

/// `healthcheck`: a dependency-free probe for the distroless image (no shell, no curl).
async fn healthcheck(bind: SocketAddr) -> anyhow::Result<()> {
    let target = if bind.ip().is_unspecified() { SocketAddr::from(([127, 0, 0, 1], bind.port())) } else { bind };
    let mut stream = tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(target)).await??;
    stream.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut buf)).await??;
    let head = String::from_utf8_lossy(&buf);
    if head.starts_with("HTTP/1.1 200") && head.contains("\"ok\":true") { Ok(()) } else { anyhow::bail!("unhealthy") }
}

fn main() -> std::process::ExitCode {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("--version" | "-V")) {
        println!("akatsuki-sync {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("akatsuki-sync: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("akatsuki-sync: cannot start runtime: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match arg.as_deref() {
        None | Some("serve") => {
            init_tracing();
            match runtime.block_on(serve(config)) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(e) => {
                    tracing::error!(error = %e, "fatal");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Some("healthcheck") => match runtime.block_on(healthcheck(config.bind_addr)) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        },
        Some("rotate-secrets") => {
            init_tracing();
            match runtime.block_on(rotate_secrets(config)) {
                Ok(true) => std::process::ExitCode::SUCCESS,
                Ok(false) => std::process::ExitCode::from(1),
                Err(e) => {
                    eprintln!("akatsuki-sync: {e}");
                    std::process::ExitCode::from(2)
                }
            }
        }
        Some(other) => {
            eprintln!("akatsuki-sync: unknown command {other:?}; use serve, healthcheck, rotate-secrets or --version");
            std::process::ExitCode::from(2)
        }
    }
}
