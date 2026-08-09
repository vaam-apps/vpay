//! vpay API server.
//!
//! Writes rows and returns. It never calls a payment rail — that is the
//! worker's job, and it is what makes the system crash-safe.

use std::net::SocketAddr;

use anyhow::Context as _;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let registry = vpay_server::adapter_registry();
    tracing::info!(rails = ?registry, "provider adapters linked");

    let addr: SocketAddr = "0.0.0.0:8080".parse().context("invalid bind address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;

    tracing::warn!("vpay-server is a scaffold: only /healthz is implemented. See docs/STATUS.md");
    tracing::info!(%addr, "listening");

    axum::serve(listener, vpay_api::router())
        .await
        .context("server error")
}
