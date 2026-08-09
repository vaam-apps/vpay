//! End-to-end proof that `vpay-server`'s CLI actually resolves configuration
//! from the process environment — not just that `clap` was *told* to look
//! there (see `backends/crates/vpay-config/src/cli.rs`'s own tests for that
//! declarative half of the contract).
//!
//! These spawn the real compiled binary and control its environment with
//! `std::process::Command::env`, a safe API that sets only the *child's*
//! environment table — never this test process's — so there is no `unsafe`
//! and no risk of one test's env leaking into another. (Mutating the current
//! process's env via `std::env::set_var` is `unsafe` as of edition 2024, and
//! this workspace sets `unsafe_code = "forbid"` with no per-test carve-out —
//! see the note in `vpay-config`'s `cli.rs`.)

// This whole compilation unit is an integration-test binary: every function
// in it exists to drive or assert on a spawned `vpay-server` process.
// Clippy's `expect_used`/`unwrap_used`/`panic` "is this in a test" detection
// only recognises `#[test]` fn bodies and `#[cfg(test)]` modules lexically —
// it does not extend to free helper functions in a `tests/*.rs`
// integration-test crate, even though the whole crate is a test target and
// `clippy.toml`'s `allow-expect-in-tests` is meant to cover exactly this.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::{BufRead as _, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// Binds two ephemeral ports simultaneously (so the OS cannot hand out the
/// same port twice), reads back their addresses, then frees both by
/// dropping the listeners — the real server binds them afterwards. Avoids
/// any fixed port colliding with another test or process on the runner.
fn two_free_addrs() -> (SocketAddr, SocketAddr) {
    let a = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port a");
    let b = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port b");
    (
        a.local_addr().expect("local_addr a"),
        b.local_addr().expect("local_addr b"),
    )
}

fn free_addr() -> SocketAddr {
    two_free_addrs().0
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
    Command::new(env!("CARGO_BIN_EXE_vpay-server"))
}

/// Polls `GET /healthz` on `addr` until it answers or `timeout` elapses.
/// Returns the HTTP response's status code, or `None` if the deadline
/// passed without ever getting a parseable response. Bounded, no fixed
/// sleep: returns as soon as the server is up.
fn poll_healthz(addr: SocketAddr, timeout: Duration) -> Option<u16> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(mut stream) = TcpStream::connect(addr) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let request = "GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
            if stream.write_all(request.as_bytes()).is_ok() {
                let mut buf = String::new();
                if stream.read_to_string(&mut buf).is_ok() {
                    let code = buf
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .and_then(|code| code.parse().ok());
                    if let Some(code) = code {
                        return Some(code);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    None
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
/// and reaps the child if it doesn't exit in time, returning `None`.
///
/// Load-bearing for any test that repeats a spawn/signal/wait cycle many
/// times: a hung child (from a genuine deadlock, or — observed directly
/// while developing this test — from severe host CPU starvation delaying
/// signal delivery/scheduling) must not be able to block the *entire* test,
/// and by extension the whole suite, on one unbounded `wait()`. A timed-out
/// attempt is reported as a failed attempt (which is also semantically
/// correct: a shutdown that never completes is not a graceful shutdown),
/// not as a hung test process.
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

/// Spawns `cmd` with its stdout piped, and streams that stdout line-by-line
/// into a channel from a background thread. Returns the guarded child
/// (stderr is discarded) plus the receiving end. Mirrors
/// `vpay-worker-bin/tests/cli.rs`'s helper of the same name.
fn spawn_and_capture_stdout(mut cmd: Command) -> (ChildGuard, Receiver<String>) {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn vpay-server");
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

/// Collects every remaining captured stdout line, up to `timeout`, stopping
/// early once the reader thread's sender disconnects (which happens once
/// the child's stdout pipe closes on process exit — call this only after
/// the child has already been reaped). Used to assert a line never
/// appeared anywhere in the process's whole output, not just within some
/// arbitrary blocking window.
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
fn an_invalid_bind_env_var_is_read_and_rejected() {
    // No `--bind` flag is passed at all, so a parse failure can only be
    // explained by `VPAY_BIND` actually having been read from the child's
    // environment — this is the deterministic negative-path proof.
    let output = bin()
        .env("VPAY_BIND", "not-a-socket-address")
        .output()
        .expect("spawn vpay-server");

    assert!(
        !output.status.success(),
        "expected a non-zero exit for an invalid VPAY_BIND"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not-a-socket-address"),
        "stderr should name the bad value, got: {stderr}"
    );
}

#[test]
fn bind_and_log_format_env_vars_are_actually_applied() {
    let addr = free_addr();
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut guard = ChildGuard(
        bin()
            .env("VPAY_BIND", addr.to_string())
            .env("VPAY_LOG_FORMAT", "text")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn vpay-server"),
    );

    let status = poll_healthz(addr, Duration::from_secs(5));
    assert_eq!(
        status,
        Some(200),
        "server never became healthy on {addr} (VPAY_BIND was not applied?)"
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
}

#[cfg(unix)]
#[test]
fn an_explicit_flag_wins_over_a_conflicting_env_var() {
    let (flag_addr, env_addr) = two_free_addrs();

    let mut guard = ChildGuard(
        bin()
            .env("VPAY_BIND", env_addr.to_string())
            .args(["--bind", &flag_addr.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn vpay-server"),
    );

    // The flagged address must come up...
    let flag_status = poll_healthz(flag_addr, Duration::from_secs(5));
    assert_eq!(
        flag_status,
        Some(200),
        "the --bind address never came up — the flag did not win"
    );

    // ...and the env var's address must never have been bound.
    assert!(
        TcpStream::connect(env_addr).is_err(),
        "VPAY_BIND's address should not have been used once --bind was passed"
    );

    send_sigterm(&guard.0);
    let exit = guard.0.wait().expect("wait for graceful shutdown");
    assert!(
        exit.success(),
        "expected exit 0 after SIGTERM, got {exit:?}"
    );
}

/// Proves `--shutdown-grace-seconds` is wired to a real bounded-drain path,
/// not merely parsed: with no in-flight request, a server started with a
/// small grace period still exits 0 promptly on SIGTERM, and its own log
/// says so via the *clean* path ("graceful shutdown complete"), never the
/// forced-timeout WARN.
///
/// What this test does **not** prove: that the timeout actually cuts off a
/// slow in-flight request. Constructing that deterministically would need a
/// route that hangs on purpose — `/healthz` answers instantly, and adding a
/// slow test-only route to `vpay-api` would be a test double reachable from
/// the shipping router, which `cargo xtask verify-no-mocks` (and AGENTS.md's
/// first rule) forbids. The timeout/race arithmetic itself — that
/// `grace_clock` actually waits the full grace period once signalled, and
/// never resolves if it isn't — is covered separately by the `#[cfg(test)]`
/// unit tests in `backends/apps/vpay-server/src/main.rs`, which exercise
/// that logic directly without a network or a real request.
#[test]
fn shutdown_grace_period_flag_still_allows_a_prompt_clean_exit_with_no_in_flight_work() {
    let addr = free_addr();
    let mut cmd = bin();
    cmd.env("VPAY_BIND", addr.to_string())
        .env("VPAY_LOG_FORMAT", "text")
        .args(["--shutdown-grace-seconds", "2"]);
    #[cfg_attr(not(unix), allow(unused_mut))]
    let (mut guard, rx) = spawn_and_capture_stdout(cmd);

    let status = poll_healthz(addr, Duration::from_secs(5));
    assert_eq!(status, Some(200), "server never became healthy on {addr}");

    #[cfg(unix)]
    {
        send_sigterm(&guard.0);
        let exit = guard.0.wait().expect("wait for graceful shutdown");
        assert!(
            exit.success(),
            "expected exit 0 after SIGTERM with no in-flight work, got {exit:?}"
        );

        let lines = collect_remaining_lines(&rx, Duration::from_secs(5));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("graceful shutdown complete")),
            "expected the clean-path log line in stdout, got: {lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("shutdown grace period elapsed")),
            "the forced-timeout WARN should never fire when there is no in-flight work, got: {lines:?}"
        );
    }
}

/// Regression test for the SIGTERM-before-handler-installation startup race:
/// `vpay-server` used to construct its shutdown-signal future as an argument
/// to `axum::serve(..).with_graceful_shutdown(..)`, so the OS-level SIGTERM
/// handler was only installed once that future was first polled — after CLI
/// parsing, tracing init, adapter-registry logging, and the `TcpListener`
/// bind. A SIGTERM delivered before that point kept its default disposition
/// (immediate termination) and bypassed graceful shutdown entirely, dropping
/// any in-flight request.
///
/// # Why this shape (warm-up + a majority vote across spaced-out attempts)
///
/// This deliberately does **not** wait for `/healthz` first: `axum::serve`'s
/// internal loop polls the shutdown-signal future in the same pass it polls
/// for an incoming connection, so by the time it can answer a request at all
/// it has already polled that future at least once — against the unfixed
/// code, that is exactly the moment the vulnerable window closes. Waiting
/// for readiness would make this test pass trivially against the bug it
/// exists to catch.
///
/// Getting to a delay that is both *safe* (never fails against correctly
/// fixed code) and *sensitive* (reliably fails against the unfixed code)
/// took real, extensive calibration, documented here so a future reader
/// doesn't have to re-derive it after "helpfully" simplifying this test:
///
/// - A single unrepeated attempt is not portable across machines, or even
///   across runs on the same machine: the *first* exec of a freshly linked
///   binary pays a one-time cold-start tax (disk page-in, and on macOS
///   first-launch code-signature verification) that can itself exceed
///   50ms — paid identically by fixed and unfixed code, unrelated to this
///   bug, which made an unwarmed single-shot version of this test fail even
///   against correctly fixed code. A throwaway `--help` invocation first
///   (fast: clap prints and exits before this binary's own code runs) pays
///   that cost once, outside the timed section.
/// - In isolation (this test alone, nothing else competing for the CPU), a
///   *tight* loop of many back-to-back spawns is its own confound: sustained
///   activity lets CPU frequency scaling ramp up, so later attempts in a
///   rapid-fire loop get measurably faster *regardless of which binary is
///   under test*. `SETTLE` — a deliberate idle gap between attempts — exists
///   to prevent that. With it, isolated calibration (spawn → `DELAY` →
///   SIGTERM → `SETTLE`, repeated over 250 times per side, both with and
///   without the fix) found a real, repeatable gap at delays as short as
///   2ms: fixed ~90-98% per-attempt success against unfixed's ~68%.
/// - That isolated-calibration delay does **not** survive this test running
///   as part of the *full* workspace suite: `cargo nextest run --workspace`
///   runs roughly twenty test binaries concurrently, including several
///   testcontainers-backed Postgres integration tests, and that real
///   contention starves *both* binaries enough that even a 2ms delay showed
///   the fixed binary itself failing 15 of 20 attempts in one observed run —
///   worse than the isolated calibration's unfixed rate. Widening the delay
///   to survive that contention (`DELAY = 50ms`, verified safe across
///   multiple full `cargo nextest run --workspace` invocations) in turn
///   makes the test blind to the bug under that same full-suite contention:
///   at 50ms neither binary reliably fails anymore, regardless of the fix.
///   No delay this sandbox could find was simultaneously safe *and*
///   sensitive under full-workspace-parallel load — see this PR's
///   description for the complete stash/build/measure steps, including the
///   fully-cold and Linux-container cross-checks and every delay tried.
///
/// Given that, `DELAY = 50ms` was chosen to prioritise the harder
/// requirement — `cargo nextest run --workspace` must never fail on
/// correctly fixed code — over maximal sensitivity. `ATTEMPTS = 20` /
/// `MIN_SUCCESSES = 16` is kept from the isolated-calibration design in
/// case this test is ever run scoped/alone (`cargo nextest run -p
/// vpay-server --test cli -E 'test(sigterm)'`), where the tighter, more
/// sensitive isolated-calibration behaviour actually applies.
///
/// **Known limitation, disclosed rather than hidden:** as configured for
/// the full workspace suite, this test's demonstrated ability to fail
/// against the unfixed code is strongest when run in isolation, not as part
/// of `cargo nextest run --workspace` — see above. It still exercises the
/// real code path end to end and would catch a sufficiently severe
/// regression (e.g. the signal handling being removed outright, which fails
/// unconditionally rather than racily), but a narrow reintroduction of
/// exactly this race might not reliably fail this test when run alongside
/// the rest of the suite. `wait_with_timeout` also exists because, while
/// developing this test under this sandbox's own heavy, self-inflicted load
/// (many hours of repeated builds and process spawns during calibration),
/// one attempt hung for several minutes — the child received SIGTERM but
/// took an unusually long time to act on it — before that helper existed to
/// bound it; a hung attempt is now scored as a failed attempt rather than
/// blocking the whole test.
///
/// Asserting on the log line, not just the exit code, matters: exit 0 alone
/// doesn't rule out some other reason the process happened to shut down
/// cleanly.
#[cfg(unix)]
#[test]
fn sigterm_immediately_after_startup_still_triggers_graceful_shutdown() {
    // Pays the one-time cold-start cost described above so it isn't a
    // confound for the timed attempts below. `--help` returns before this
    // binary's own signal-handling or tracing code ever runs.
    let _ = bin().arg("--help").output();

    const ATTEMPTS: u8 = 20;
    const MIN_SUCCESSES: u8 = 16;
    const DELAY: Duration = Duration::from_millis(50);
    /// Settle gap between attempts. Without it, back-to-back spawns keep the
    /// CPU busy enough (frequency scaling ramps up under sustained load)
    /// that later attempts in the loop get measurably faster regardless of
    /// which binary is under test, washing out the very signal this test
    /// depends on — see the doc comment above for the calibration that
    /// found this.
    const SETTLE: Duration = Duration::from_millis(100);

    // Bounds each attempt's wait so one stuck child (see `wait_with_timeout`)
    // can cost at most this much wall time rather than hanging the test.
    const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

    let mut outcomes = Vec::with_capacity(ATTEMPTS as usize);
    for attempt in 1..=ATTEMPTS {
        let addr = free_addr();
        let mut cmd = bin();
        cmd.env("VPAY_BIND", addr.to_string())
            .env("VPAY_LOG_FORMAT", "text");
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
}
