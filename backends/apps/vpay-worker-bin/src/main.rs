//! vpay worker: submit, poll, reconcile, deliver.

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

    tracing::warn!(
        "vpay-worker is a scaffold: the job loop is not implemented. See docs/STATUS.md"
    );

    // Exits immediately and honestly rather than idling in a loop that does
    // nothing, which would look like a running worker in `docker compose ps`.
    Ok(())
}
