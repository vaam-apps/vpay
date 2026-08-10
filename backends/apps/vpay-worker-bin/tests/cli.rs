//! End-to-end proof that `vpay-worker-bin`'s CLI actually resolves
//! configuration from the process environment — not just that `clap` was
//! *told* to look there (see `backends/crates/vpay-config/src/cli.rs`'s own
//! tests for that declarative half of the contract).
//!
//! These spawn the real compiled binary and control its environment with
//! `std::process::Command::env`, a safe API that sets only the *child's*
//! environment table — never this test process's — so there is no `unsafe`
//! and no risk of one test's env leaking into another. (Mutating the current
//! process's env via `std::env::set_var` is `unsafe` as of edition 2024, and
//! this workspace sets `unsafe_code = "forbid"` with no per-test carve-out —
//! see the note in `vpay-config`'s `cli.rs`.)
//!
//! The worker has no HTTP surface to poll, so instead of `GET /healthz`
//! these read the worker's own `--log-format text` stdout for the `profile`
//! field it stamps on startup — that field can only carry the value it did
//! if the corresponding env var (or the overriding flag) was actually read.

// This whole compilation unit is an integration-test binary: every function
// in it exists to drive or assert on a spawned `vpay-worker-bin` process.
// Clippy's `expect_used`/`unwrap_used`/`panic` "is this in a test" detection
// only recognises `#[test]` fn bodies and `#[cfg(test)]` modules lexically —
// it does not extend to free helper functions in a `tests/*.rs`
// integration-test crate, even though the whole crate is a test target and
// `clippy.toml`'s `allow-expect-in-tests` is meant to cover exactly this.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::BufRead as _;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Starts a fresh Postgres 16 container and returns its connection URL. Must
/// be called from, and the returned container kept alive inside, the same
/// tokio runtime for its entire lifetime — see [`with_live_postgres`] for
/// why. Mirrors `backends/apps/vpay-server/tests/cli.rs`'s identical helper
/// (same reasoning for the pinned `16-alpine` tag: `postgres:11-alpine`,
/// this crate's default, is not cached on this machine and unreachable).
async fn start_postgres() -> (ContainerAsync<PostgresImage>, String) {
    let container = PostgresImage::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("postgres:16-alpine container starts (it is cached locally on this machine)");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    (container, url)
}

/// Runs `body` — a plain, synchronous test body that spawns and drives a
/// `vpay-worker-bin` subprocess — with a real Postgres connection URL
/// available to pass as `DATABASE_URL`. Mirrors
/// `backends/apps/vpay-server/tests/cli.rs`'s identical helper of the same
/// name; see that file's doc comment for the full reasoning (in short:
/// `vpay-worker-bin` now requires a real database at startup too, one
/// container is started per test *function* and reused across every
/// subprocess spawn within it, and the container is deliberately a local of
/// this function's own `block_on` rather than a `static`, because
/// `ContainerAsync`'s `Drop` needs an active tokio runtime context to run
/// its cleanup and a `static` would never run `Drop` at process exit at
/// all — the exact leak `.config/nextest.toml` documents rejecting
/// elsewhere in this workspace).
fn with_live_postgres(body: impl FnOnce(String)) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a tokio runtime to drive testcontainers");
    rt.block_on(async {
        let (_container, url) = start_postgres().await;
        body(url);
    });
}

/// RAII guard: kills and reaps the child on drop, so a panicking assertion
/// can never leak a process behind it.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_vpay-worker-bin"))
}

/// Spawns `cmd` with its stdout piped, and streams that stdout line-by-line
/// into a channel from a background thread. Returns the guarded child
/// (stderr is discarded) plus the receiving end.
fn spawn_and_capture_stdout(mut cmd: Command) -> (ChildGuard, Receiver<String>) {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn vpay-worker-bin");
    let stdout = child.stdout.take().expect("piped stdout");

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if tx.send(l).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    (ChildGuard(child), rx)
}

/// Blocks (up to `timeout`) until a line satisfying `predicate` arrives, or
/// returns `None` if the deadline passes first. Bounded, no fixed sleep:
/// returns as soon as the line appears.
fn wait_for_line(
    rx: &Receiver<String>,
    predicate: impl Fn(&str) -> bool,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match rx.recv_timeout(remaining) {
            Ok(line) if predicate(&line) => return Some(line),
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
}

/// Sends SIGTERM to `child` via the `kill` utility rather than a raw libc
/// FFI call, so this stays within `unsafe_code = "forbid"`.
#[cfg(unix)]
fn send_sigterm(child: &Child) {
    let pid = child.id().to_string();
    let status = Command::new("kill")
        .args(["-TERM", &pid])
        .status()
        .expect("invoke `kill`");
    assert!(status.success(), "`kill -TERM {pid}` itself failed to run");
}

/// Polls `child` for exit with `Child::try_wait` up to `timeout`, instead of
/// the blocking `Child::wait` (which has no timeout in `std`). Force-kills
/// and reaps the child if it doesn't exit in time, returning `None`. Mirrors
/// `vpay-server/tests/cli.rs`'s helper of the same name — see its doc
/// comment for why this matters for a test that repeats a spawn/signal/wait
/// cycle many times.
#[cfg(unix)]
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Collects every remaining captured stdout line, up to `timeout`, stopping
/// early once the reader thread's sender disconnects (which happens once
/// the child's stdout pipe closes on process exit — call this only after
/// the child has already been reaped). Used to assert a line never
/// appeared anywhere in the process's whole output, not just within some
/// arbitrary blocking window. Mirrors `vpay-server/tests/cli.rs`'s helper
/// of the same name.
#[cfg(unix)]
fn collect_remaining_lines(rx: &Receiver<String>, timeout: Duration) -> Vec<String> {
    let deadline = Instant::now() + timeout;
    let mut lines = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match rx.recv_timeout(remaining) {
            Ok(line) => lines.push(line),
            Err(_) => break,
        }
    }
    lines
}

#[test]
fn an_invalid_log_format_env_var_is_read_and_rejected() {
    // No `--log-format` flag is passed at all, so a parse failure can only
    // be explained by `VPAY_LOG_FORMAT` actually having been read from the
    // child's environment — this is the deterministic negative-path proof.
    let output = bin()
        .env("VPAY_LOG_FORMAT", "not-a-format")
        .output()
        .expect("spawn vpay-worker-bin");

    assert!(
        !output.status.success(),
        "expected a non-zero exit for an invalid VPAY_LOG_FORMAT"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not-a-format"),
        "stderr should name the bad value, got: {stderr}"
    );
}

#[test]
fn profile_env_var_is_read_and_stamped_into_the_startup_log() {
    with_live_postgres(|database_url| {
        let mut cmd = bin();
        cmd.env("VPAY_PROFILE", "integration-test-profile")
            .env("VPAY_LOG_FORMAT", "text")
            .env("DATABASE_URL", &database_url);
        #[cfg_attr(not(unix), allow(unused_mut))]
        let (mut guard, rx) = spawn_and_capture_stdout(cmd);

        let line = wait_for_line(
            &rx,
            |l| l.contains("integration-test-profile"),
            Duration::from_secs(5),
        );
        assert!(
            line.is_some(),
            "VPAY_PROFILE's value never appeared in the worker's startup log"
        );

        #[cfg(unix)]
        {
            send_sigterm(&guard.0);
            let exit = guard.0.wait().expect("wait for graceful shutdown");
            assert!(
                exit.success(),
                "expected exit 0 after SIGTERM, got {exit:?}"
            );
        }
    });
}

#[cfg(unix)]
#[test]
fn an_explicit_profile_flag_wins_over_a_conflicting_env_var() {
    with_live_postgres(|database_url| {
        let mut cmd = bin();
        cmd.env("VPAY_PROFILE", "env-should-lose")
            .env("VPAY_LOG_FORMAT", "text")
            .env("DATABASE_URL", &database_url)
            .args(["--profile", "flag-should-win"]);
        let (mut guard, rx) = spawn_and_capture_stdout(cmd);

        let line = wait_for_line(
            &rx,
            |l| l.contains("flag-should-win") || l.contains("env-should-lose"),
            Duration::from_secs(5),
        );
        let line = line.expect("no profile line observed in the worker's startup log");
        assert!(
            line.contains("flag-should-win"),
            "expected the --profile flag's value to win, got: {line}"
        );
        assert!(
            !line.contains("env-should-lose"),
            "VPAY_PROFILE's value should not have been used once --profile was passed, got: {line}"
        );

        send_sigterm(&guard.0);
        let exit = guard.0.wait().expect("wait for graceful shutdown");
        assert!(
            exit.success(),
            "expected exit 0 after SIGTERM, got {exit:?}"
        );
    });
}

/// Regression test for the SIGTERM-before-handler-installation startup race
/// (shared with `vpay-server`, see that crate's `tests/cli.rs` for the fuller
/// writeup and the calibration data behind this test's shape): `vpay-worker-bin`
/// used to construct its shutdown-signal future right before entering its
/// select loop, so the OS-level SIGTERM handler was only installed once that
/// future was first polled — after CLI parsing, tracing init, the startup log
/// lines, and the first heartbeat tick. A SIGTERM delivered before that point
/// kept its default disposition (immediate termination) and bypassed graceful
/// shutdown entirely.
///
/// The worker has no HTTP surface to poll for "is it up", and waiting for its
/// startup log line would suffer the same problem `vpay-server`'s equivalent
/// test avoids by not waiting for `/healthz`: by the time a log line is
/// observed, tracing has already initialised, which (pre-fix) is well past
/// where handler installation used to happen too, closing the window this
/// test exists to catch.
///
/// Like the server's version of this test, this warms up the binary once
/// first (a throwaway `--help` invocation) to pay this platform's one-time
/// cold-start cost (disk page-in, and on macOS first-launch code-signature
/// verification) outside the timed section, and uses the same
/// spawn → `DELAY` → SIGTERM → `SETTLE`, majority-vote-of-`ATTEMPTS` shape
/// for the same reason: a bare fixed sleep (even repeated with a strict
/// "every attempt must succeed" rule) could not separate this platform's
/// sub-5ms scheduling noise from the (still real) bug without an
/// unacceptable false-failure rate on correctly fixed code, and a *tight*
/// loop of back-to-back spawns lets CPU frequency scaling ramp up enough to
/// wash the signal out entirely, regardless of which binary is under test —
/// `SETTLE` exists to prevent that. See the server test's doc comment in
/// `backends/apps/vpay-server/tests/cli.rs` for the full calibration
/// writeup (measured rates, the intermediate designs that didn't hold up,
/// the fully-cold and Linux-container cross-checks); the same `DELAY`,
/// `ATTEMPTS` and `MIN_SUCCESSES` are used here since this binary's startup
/// path is close enough to the server's (same signal-handling code, no
/// adapter-registry log or `TcpListener::bind` to speak of) that a separate
/// full calibration pass wasn't expected to land meaningfully differently,
/// and spot-checks here did not contradict that.
///
/// **Known limitation, disclosed rather than hidden** (see the server
/// test's own note for the full explanation, including why `DELAY = 50ms`
/// specifically was chosen): this is a statistical, not deterministic, test
/// of a genuinely narrow race, and `cargo nextest run --workspace`'s real
/// contention from ~20 concurrently running test binaries measurably
/// widened the window for *both* fixed and unfixed code — the delay that
/// separated them cleanly in isolation does not survive running as part of
/// the full suite. `DELAY = 50ms` prioritises never failing the full suite
/// on correctly fixed code over maximal sensitivity to a reintroduced bug;
/// this test's demonstrated ability to catch the bug is strongest when run
/// scoped/alone. If this test becomes a source of CI flakiness, treat that
/// as this limitation showing up, not necessarily a reintroduced bug.
///
/// Asserting on the log line, not just the exit code, matters regardless:
/// exit 0 alone doesn't rule out some other reason the process happened to
/// shut down cleanly.
#[cfg(unix)]
#[test]
fn sigterm_immediately_after_startup_still_triggers_graceful_shutdown() {
    // Pays the one-time cold-start cost described above so it isn't a
    // confound for the timed attempts below. `--help` returns before this
    // binary's own signal-handling or tracing code ever runs (and before it
    // ever looks at `DATABASE_URL`, so no container is needed for this
    // warm-up call specifically).
    let _ = bin().arg("--help").output();

    // One container, reused for all `ATTEMPTS` spawns below — see
    // `with_live_postgres`'s doc comment, and
    // `backends/apps/vpay-server/tests/cli.rs`'s identical test for why the
    // added DB connect + idempotent migration per attempt does not affect
    // this test's sensitivity to the race (`ShutdownSignals::install()`
    // still runs first, before the DB connect).
    with_live_postgres(|database_url| {
        const ATTEMPTS: u8 = 20;
        const MIN_SUCCESSES: u8 = 16;
        const DELAY: Duration = Duration::from_millis(50);
        /// Settle gap between attempts — see the server test's doc comment for
        /// why this is load-bearing, not cosmetic: without it, back-to-back
        /// spawns keep the CPU boosted from sustained load, which washes out
        /// the very signal this test depends on.
        const SETTLE: Duration = Duration::from_millis(100);

        // Bounds each attempt's wait so one stuck child (see `wait_with_timeout`)
        // can cost at most this much wall time rather than hanging the test.
        const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

        let mut outcomes = Vec::with_capacity(ATTEMPTS as usize);
        for attempt in 1..=ATTEMPTS {
            let mut cmd = bin();
            cmd.env("VPAY_PROFILE", "sigterm-race-test")
                .env("VPAY_LOG_FORMAT", "text")
                .env("DATABASE_URL", &database_url);
            let (mut guard, rx) = spawn_and_capture_stdout(cmd);

            std::thread::sleep(DELAY);

            send_sigterm(&guard.0);
            let exit = wait_with_timeout(&mut guard.0, WAIT_TIMEOUT);
            let lines = collect_remaining_lines(&rx, Duration::from_secs(5));
            let graceful = exit.is_some_and(|exit| exit.success())
                && lines
                    .iter()
                    .any(|l| l.contains("received SIGTERM, starting graceful shutdown"));
            outcomes.push((attempt, graceful, exit, lines));
            std::thread::sleep(SETTLE);
        }

        let successes = outcomes.iter().filter(|(_, ok, _, _)| *ok).count();
        assert!(
            successes as u8 >= MIN_SUCCESSES,
            "only {successes}/{ATTEMPTS} attempts shut down gracefully after an immediate SIGTERM \
             (need at least {MIN_SUCCESSES}); this many failures indicates the handler is not \
             installed early enough, not just scheduling noise. Failing attempts:\n{}",
            outcomes
                .iter()
                .filter(|(_, ok, _, _)| !ok)
                .map(|(attempt, _, exit, lines)| {
                    let exit_desc = match exit {
                        Some(status) => format!("{status:?}"),
                        None => format!("timed out after {WAIT_TIMEOUT:?} and was force-killed"),
                    };
                    format!("  attempt {attempt}/{ATTEMPTS}: exit={exit_desc} lines={lines:?}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
    });
}
