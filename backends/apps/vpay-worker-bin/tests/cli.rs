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
    let mut cmd = bin();
    cmd.env("VPAY_PROFILE", "integration-test-profile")
        .env("VPAY_LOG_FORMAT", "text");
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
}

#[cfg(unix)]
#[test]
fn an_explicit_profile_flag_wins_over_a_conflicting_env_var() {
    let mut cmd = bin();
    cmd.env("VPAY_PROFILE", "env-should-lose")
        .env("VPAY_LOG_FORMAT", "text")
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
}
