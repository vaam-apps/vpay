//! Shared OS shutdown-signal handling for `vpay-server` and `vpay-worker-bin`.
//!
//! # The race this closes
//!
//! `tokio::signal::unix::signal(kind)` installs its OS-level handler
//! *synchronously, when called* — the registration happens inside the
//! function body itself (`signal_hook_registry::register`), not when the
//! returned [`Signal`](tokio::signal::unix::Signal) stream is first polled.
//! `tokio::signal::ctrl_c()`, by contrast, is an `async fn`: nothing runs
//! until its returned future is first polled — including, on Unix, the
//! `signal(SignalKind::interrupt())` call it performs internally.
//!
//! Both binaries used to construct their shutdown future as an argument to
//! `axum::serve(..).with_graceful_shutdown(..)` (or, for the worker, right
//! before entering its select loop) — i.e. *after* CLI parsing, tracing
//! init, and (for the server) binding the listener. Until that future was
//! first polled, the signal handlers it would install had never run, so
//! SIGTERM retained its default disposition: immediate termination, no
//! graceful shutdown, any in-flight request dropped. That window was
//! measured at tens of milliseconds in isolation and longer under load —
//! long enough to matter for a process a container orchestrator signals
//! immediately after spawning it.
//!
//! [`ShutdownSignals::install`] closes the window by registering the OS
//! handlers eagerly, at construction time, before any of that startup work
//! runs. Call it as the very first thing in `main`, right after CLI parsing.

use std::io;

/// Handle to the OS signal handlers this process shuts down on.
///
/// Must be created via [`ShutdownSignals::install`] as early as possible in
/// `main` — see the module docs for why construction time, not poll time,
/// is what matters here.
#[derive(Debug)]
pub struct ShutdownSignals {
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigint: tokio::signal::unix::Signal,
}

impl ShutdownSignals {
    /// Registers this process's shutdown signal handlers immediately.
    ///
    /// On Unix this installs both SIGTERM and SIGINT handlers via
    /// `tokio::signal::unix::signal`, which registers synchronously inside
    /// this call — not on first `.await` of [`Self::wait`]. `SIGINT` is
    /// handled the same way rather than via `tokio::signal::ctrl_c()`
    /// specifically so it gets the same early-installation guarantee;
    /// `ctrl_c()` is an `async fn` and would reintroduce the exact race
    /// this type exists to close.
    ///
    /// Non-Unix platforms have no `SignalKind`-based API and fall back to
    /// `tokio::signal::ctrl_c()` inside [`Self::wait`], which does still
    /// install on first poll there — this only claims to close the race on
    /// Unix, which is also the only platform SIGTERM (the signal that
    /// motivated this type) exists on.
    ///
    /// # Errors
    /// If the OS refuses to install a handler (`signal_hook_registry`
    /// failure — e.g. the process is out of the OS's signal-handler budget,
    /// or a competing registration for the same signal number failed
    /// earlier in the process). Callers should treat this as a hard startup
    /// failure rather than continuing without graceful shutdown: unlike a
    /// runtime error encountered later (handled by [`Self::wait`] itself),
    /// a failure here means the process would run its *entire* lifetime
    /// with no way to shut down cleanly, not just during a brief startup
    /// window — silently continuing would reintroduce the very bug this
    /// type exists to fix, for the whole process lifetime rather than a
    /// race window.
    pub fn install() -> io::Result<Self> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let sigterm = signal(SignalKind::terminate())?;
            let sigint = signal(SignalKind::interrupt())?;
            Ok(Self { sigterm, sigint })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {})
        }
    }

    /// Waits for SIGINT or (Unix-only) SIGTERM, whichever arrives first,
    /// logs which one fired, then returns.
    ///
    /// The handlers themselves were already installed by [`Self::install`];
    /// this only awaits notifications on them.
    pub async fn wait(&mut self) {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.sigint.recv() => tracing::info!("received SIGINT, starting graceful shutdown"),
                _ = self.sigterm.recv() => tracing::info!("received SIGTERM, starting graceful shutdown"),
            }
        }

        #[cfg(not(unix))]
        {
            // No panic in a shutdown path: log and fall back to waiting
            // forever rather than propagating a runtime error out of a
            // shutdown-signal wait.
            match tokio::signal::ctrl_c().await {
                Ok(()) => tracing::info!("received SIGINT, starting graceful shutdown"),
                Err(err) => {
                    tracing::error!(
                        %err,
                        "failed to install Ctrl+C handler; this shutdown path is now inert"
                    );
                    std::future::pending::<()>().await;
                }
            }
        }
    }
}
