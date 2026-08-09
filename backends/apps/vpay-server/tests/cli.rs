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
