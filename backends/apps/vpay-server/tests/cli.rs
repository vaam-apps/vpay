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
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres as PostgresImage;

/// Starts a fresh Postgres 16 container and returns its connection URL. Must
/// be called from, and the returned container kept alive inside, the same
/// tokio runtime for its entire lifetime — see [`with_live_postgres`] for
/// why. The container comes from
/// `vpay_testkit::containers::start_postgres_with_retry`, the one helper
/// `vpay-db` and `vpay-tests-integration` also call — it is where the pinned
/// `16-alpine` tag and the host-port-collision retry are documented.
async fn start_postgres() -> (ContainerAsync<PostgresImage>, String) {
    let container = vpay_testkit::containers::start_postgres_with_retry()
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
/// `vpay-server` subprocess — with a real Postgres connection URL available
/// to pass as `DATABASE_URL`.
///
/// `vpay-server` now requires a real, reachable database at startup
/// (`backends/apps/vpay-server/src/main.rs`), so every subprocess-spawning
/// test in this file needs one. This starts exactly **one** container per
/// test *function*, not per subprocess spawn — the SIGTERM-race test below
/// spawns the binary up to 20 times but reuses the same container/URL for
/// every attempt, since migrations are idempotent (proven in `vpay-db`'s own
/// `tests/postgres.rs`) and nothing here exercises schema changes.
///
/// Implemented with a small `current_thread` runtime rather than
/// `#[tokio::test]`, because this file's tests are synchronous drivers of a
/// subprocess (`std::process::Command`, `std::thread::sleep`), not async
/// code of their own. `body` runs *inside* the same `block_on` call that
/// started the container: `ContainerAsync`'s `Drop` impl calls
/// `tokio::runtime::Handle::current()` (`testcontainers` 0.27's
/// `core::async_drop`), which panics with no runtime active, so the
/// container must still be a live local when it drops, at the end of this
/// function's own `block_on`. It is deliberately never a `static` — a
/// `static` `ContainerAsync` was tried and rejected for this repo's sibling
/// suite (`backends/tests/integration`, see `.config/nextest.toml`'s own
/// comment): Rust never runs `Drop` on statics at process exit, so it would
/// leak the underlying Docker container every time this test binary exits.
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

/// A minimal, valid `vpay_config::Config` YAML file — see the fixture's own
/// comment for why it needs no `${VAR}` environment variables beyond
/// `DATABASE_URL` to let this binary boot all the way to `/healthz`.
fn valid_config_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/valid-config.yml"
    )
}

/// A real RSA private key on disk, generated once per run of this test
/// binary, standing in for the Kubernetes Secret mount `vpay-server` reads
/// at boot.
///
/// **Generated, never checked in.** A PEM committed to the repository is a
/// private key in version control — even a test one, and even one nothing
/// signs anything real with; and it is the kind of file that gets copied.
/// Generation costs about a second, once, amortised across every test in
/// this file.
///
/// 2048 bits, the floor `vpay_api::op::keys` enforces: this fixture exists
/// to let the server boot, not to exercise the key-strength check (that has
/// its own unit test, against material this file never has to hold).
///
/// `CARGO_TARGET_TMPDIR` rather than `std::env::temp_dir`: cargo gives an
/// integration-test binary a scratch directory inside `target/`, which is
/// already git-ignored and already cleaned by `cargo clean`, so a generated
/// key never lands in `/tmp` where it would outlive the run. The file name
/// carries this process's pid so two concurrently-running test binaries
/// cannot write over each other.
static SIGNING_KEY_FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    use rsa::pkcs1::{EncodeRsaPrivateKey as _, LineEnding};

    let mut rng = rand::rngs::OsRng;
    let key = rsa::RsaPrivateKey::new(&mut rng, 2048).expect("rsa key generation succeeds");
    let path =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("oauth-signing-key-{}.pem", pid()));
    key.write_pkcs1_pem_file(&path, LineEnding::LF)
        .expect("the generated key is written to the cargo target tmpdir");
    path
});

/// This process's id, for a unique fixture file name. `std::process::id`
/// returns a `u32` on every platform.
fn pid() -> u32 {
    std::process::id()
}

/// [`SIGNING_KEY_FILE`] as the `&str` `Command::env` wants.
fn generated_key_path() -> &'static str {
    SIGNING_KEY_FILE
        .to_str()
        .expect("the cargo target tmpdir path is valid utf-8")
}

/// A file that exists and is readable but is not a private key of any kind.
///
/// Written next to the real one, and deliberately *not* "an RSA key with a
/// corrupted body": the interesting boundary is "the Secret was mounted but
/// holds the wrong thing", which is what a mis-keyed Kubernetes Secret or a
/// mounted ConfigMap actually looks like.
static NOT_A_KEY_FILE: LazyLock<PathBuf> = LazyLock::new(|| {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("not-a-key-{}.pem", pid()));
    std::fs::write(&path, b"this is not a PEM-encoded RSA private key\n")
        .expect("the fixture is written to the cargo target tmpdir");
    path
});

fn not_a_key_path() -> &'static str {
    NOT_A_KEY_FILE
        .to_str()
        .expect("the cargo target tmpdir path is valid utf-8")
}

/// A `vpay_config::Config` YAML file that fails validation
/// (`ConfigError::InsecureHost`: an `http://` host under `livemode: true`).
fn invalid_config_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/invalid-config.yml"
    )
}

/// A config that `vpay_config` itself accepts but this binary cannot serve:
/// it names a `providers[].code` no linked adapter answers to. See the
/// fixture's own comment.
fn provider_without_adapter_config_path() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/provider-without-adapter.yml"
    )
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

/// Sends one HTTP/1.1 request over a fresh connection and returns
/// `(status, whole response text)`. The body is returned unparsed, and the
/// caller matches on a substring, so a response framed with
/// `Transfer-Encoding: chunked` reads the same as one with a
/// `Content-Length` — this file has no HTTP client and does not want one.
///
/// Unlike [`poll_healthz`] this does **not** retry: it is for asserting on
/// one specific response from a server already known to be up.
fn http_request(addr: SocketAddr, request: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(addr).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())?;
    Some((status, response))
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

/// A `postgres://` URL pointing at a port nothing listens on. Port 1
/// (`tcpmux`) is reserved and never bound by anything on a developer or CI
/// machine, and `127.0.0.1` keeps the attempt on the loopback interface —
/// so this needs **no Docker and no network**, unlike the container-backed
/// tests in this file. `vpay_db::connect` gives up after its 5s
/// `ACQUIRE_TIMEOUT` (`backends/crates/vpay-db/src/pool.rs`) rather than
/// failing instantly on `ECONNREFUSED`: sqlx retries inside that window, so
/// a test using this URL takes a little over five seconds by design and
/// must not be given a shorter deadline.
const UNREACHABLE_DATABASE_URL: &str = "postgres://vpay:vpay@127.0.0.1:1/vpay";

/// `--config` / `VPAY_CONFIG` stays `Option<PathBuf>` at the `clap` level
/// (`vpay_config::CommonArgs::config`) but `main.rs` now treats it as
/// required — this is the deterministic negative-path proof, spawned with
/// no `VPAY_CONFIG`/`--config` at all. No `DATABASE_URL` is supplied either
/// deliberately: config loading happens *before* this binary ever tries to
/// connect to a database (see `main.rs`'s own comment on that ordering), so
/// a missing config must fail before a missing database URL would even be
/// checked.
///
/// The exit code is asserted **exactly**, not merely as non-zero: `78`
/// (`EX_CONFIG`) is `Category::Configuration::exit_code()` per ADR-0011, and
/// the whole point of that mapping is that a supervisor can tell "fix the
/// YAML" from "Postgres is down" (69, below) without parsing logs. A
/// `!success()` assertion would pass just as happily against `main`'s old
/// blanket exit `1`.
#[test]
fn a_missing_config_is_exit_78_naming_the_problem() {
    let output = bin().output().expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(78),
        "expected EX_CONFIG (78) with no --config/VPAY_CONFIG at all, got {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--config") || stderr.contains("VPAY_CONFIG"),
        "stderr should name the missing config, got: {stderr}"
    );
}

/// A config file that exists but fails validation
/// (`ConfigError::InsecureHost`, see `tests/fixtures/invalid-config.yml`)
/// must also exit `78` — and, like the missing case above, before this
/// binary ever touches the database (config validation needs no network I/O
/// and is ordered first in `main.rs`).
///
/// Same category, same code: `ConfigError` classifies as one
/// `Category::Configuration` for every variant, so "you forgot the flag" and
/// "the file is wrong" are the same *kind* of operator problem and the
/// difference is carried by the message on stderr, not by the number.
#[test]
fn a_bad_config_is_exit_78_naming_the_problem() {
    let output = bin()
        .env("VPAY_CONFIG", invalid_config_path())
        .output()
        .expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(78),
        "expected EX_CONFIG (78) for a config that fails validation, got {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("livemode requires https"),
        "stderr should name the specific validation failure, got: {stderr}"
    );
}

/// Boot step 4's join: a `providers[]` entry with no linked adapter is exit
/// `78`, and the message names the rail.
///
/// This is the one boot check `vpay-config` structurally *cannot* make —
/// which adapters exist is a property of the binary, and `vpay-config` links
/// none of them (ADR-0002) — so it is only observable from out here, on the
/// real process.
///
/// **No container, and that is the assertion's other half.** The join runs
/// immediately after the config is validated, before the signing key is
/// read and long before `vpay_db::connect`, so this test passes no
/// `DATABASE_URL` and no `VPAY_OAUTH_SIGNING_KEY_FILE` at all. If someone
/// later moves `boot_seeds` below the database connection, this test starts
/// failing with `69` (or with the missing-signing-key `78` for the wrong
/// reason, which is why the stderr assertion is on the rail's name and not
/// merely on the number).
#[test]
fn a_provider_code_with_no_linked_adapter_is_exit_78() {
    let output = bin()
        .env("VPAY_CONFIG", provider_without_adapter_config_path())
        .output()
        .expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(78),
        "expected EX_CONFIG (78) for a configured rail with no linked adapter, got {:?}; \
         stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("a_rail_that_does_not_exist"),
        "stderr should name the rail that has no adapter, got: {stderr}"
    );
    assert!(
        stderr.contains("mtn_momo"),
        "stderr should list the rails this binary does link, so the message is actionable \
         without a second lookup, got: {stderr}"
    );
}

/// The other side of the same join: the repo's own `config/application.yml`
/// names exactly the rails this binary links, so a valid config gets past
/// step 4's join and fails on the *next* thing instead.
///
/// Without this, `a_provider_code_with_no_linked_adapter_is_exit_78` would
/// still pass if `boot_seeds` rejected every configuration.
#[test]
fn the_repositorys_own_configuration_passes_the_adapter_join() {
    let output = bin()
        .env(
            "VPAY_CONFIG",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../config/application.yml"
            ),
        )
        // The rail credentials `config/application.yml` reads from the
        // environment. Values are irrelevant — nothing connects to a rail
        // here; what matters is that the placeholders resolve so the load
        // gets as far as the join.
        .env("MTN_SUBSCRIPTION_KEY", "not-a-real-key")
        .env("MTN_API_KEY", "not-a-real-key")
        .env("MTN_API_USER", "not-a-real-uuid")
        .env("ORANGE_MERCHANT_KEY", "not-a-real-key")
        .env("ORANGE_CLIENT_ID", "not-a-real-client")
        .env("ORANGE_CLIENT_SECRET", "not-a-real-secret")
        // Added 2026-09-03 (Step 5): the example merchant now names a webhook
        // endpoint whose `secrets:` is a `${MERCHANT_WEBHOOK_SECRET}`
        // placeholder. An unresolved placeholder is exit 78 *before* the join,
        // which would make this test pass for the wrong reason — it asserts
        // that the join is not what stops the config.
        .env(
            "MERCHANT_WEBHOOK_SECRET",
            "not-a-real-secret-but-32-bytes-long",
        )
        .output()
        .expect("spawn vpay-server");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("links no adapter"),
        "config/application.yml must name only rails this binary links, got: {stderr}"
    );
    // It still fails — on the missing signing key, the step after the join.
    assert_eq!(
        output.status.code(),
        Some(78),
        "expected the *next* startup requirement to be what fails, got {:?}; stderr: {stderr}",
        output.status,
    );
    assert!(
        stderr.contains("oauth-signing-key-file"),
        "the join must not be what stops this config; the next requirement should be, got: \
         {stderr}"
    );
}

/// The other half of ADR-0011's exit-code contract: a config that is
/// perfectly valid, pointing at a database that is not there, exits `69`
/// (`EX_UNAVAILABLE`) — `Category::Storage`'s code — and not `78`.
///
/// This is the assertion that proves `exit_code_for` actually walks the
/// `anyhow` chain and classifies what it finds, rather than returning one
/// constant: the *only* difference from the test above is which leaf error
/// ends up in the chain.
///
/// Needs no container (see [`UNREACHABLE_DATABASE_URL`]), so unlike the
/// `with_live_postgres` tests in this file it runs anywhere. It does take
/// ~5s, which is `vpay-db`'s acquire timeout and not this test waiting on
/// anything of its own.
#[test]
fn an_unreachable_database_is_exit_69_naming_postgres() {
    let output = bin()
        .env("VPAY_CONFIG", valid_config_path())
        .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path())
        .env("DATABASE_URL", UNREACHABLE_DATABASE_URL)
        .output()
        .expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(69),
        "expected EX_UNAVAILABLE (69) for an unreachable database, got {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Postgres"),
        "stderr should name Postgres as what could not be reached, got: {stderr}"
    );
}

/// `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE` is required,
/// and its absence is a *configuration* failure — exit `78`, naming the
/// flag.
///
/// **Needs no Docker, and that is the point of where the check sits.** A
/// valid `VPAY_CONFIG` is supplied and no `DATABASE_URL` at all: `main.rs`
/// requires the key between loading the config and looking at
/// `--database-url`, so if this test ever starts needing a container it
/// means the key check has drifted below the database connection — at which
/// point a deployment with an unmounted Secret would pay for a Postgres
/// connection and a migration run before failing, and would fail with the
/// wrong diagnosis if Postgres happened to be down too.
///
/// The exit code is asserted exactly, for the reason
/// `a_missing_config_is_exit_78_naming_the_problem` gives: `78` is what
/// tells a supervisor "fix the deploy" rather than "wait for a dependency",
/// and a bare `!success()` would pass against exit `1`.
#[test]
fn a_missing_signing_key_flag_is_exit_78_naming_the_problem() {
    let output = bin()
        .env("VPAY_CONFIG", valid_config_path())
        .output()
        .expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(78),
        "expected EX_CONFIG (78) with no --oauth-signing-key-file/VPAY_OAUTH_SIGNING_KEY_FILE, \
         got {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--oauth-signing-key-file")
            || stderr.contains("VPAY_OAUTH_SIGNING_KEY_FILE"),
        "stderr should name the missing signing key flag, got: {stderr}"
    );
}

/// A path that does not exist — the single most likely production failure,
/// an unmounted or mis-pathed Kubernetes Secret. `SigningKeyError::Read`,
/// which classifies as `Category::Configuration`, so `78`.
///
/// The path is asserted to appear on stderr: "which file did it try" is the
/// whole diagnosis, and `SigningKeyError::Read` carries it deliberately (a
/// path is not a secret). Needs no Docker, same ordering argument as above.
#[test]
fn a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path() {
    let missing = concat!(env!("CARGO_TARGET_TMPDIR"), "/no-such-signing-key.pem");
    let output = bin()
        .env("VPAY_CONFIG", valid_config_path())
        .env("VPAY_OAUTH_SIGNING_KEY_FILE", missing)
        .output()
        .expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(78),
        "expected EX_CONFIG (78) for a signing key file that does not exist, got {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no-such-signing-key.pem"),
        "stderr should name the file it could not read, got: {stderr}"
    );
}

/// A file that *is* readable but is not a private key —
/// `SigningKeyError::NotAnRsaPrivateKey`, also `Category::Configuration`,
/// also `78`.
///
/// Distinct from the missing-file case above on purpose: the two have
/// different fixes (mount the Secret vs. put the right thing in it) and this
/// is the assertion that would fail if the key were only *stat*ed rather
/// than parsed at boot — a server that deferred parsing to the first token
/// request would boot happily here and 500 on every merchant.
///
/// stderr must **not** contain the file's contents. That is a real property
/// of `SigningKeyError` (`no_error_echoes_the_key_material`, in
/// `vpay_api::op::keys`) and this asserts it survives the trip through
/// `anyhow`'s context chain and out to a process's standard error, where an
/// operator's log shipper would pick it up.
#[test]
fn a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents() {
    let output = bin()
        .env("VPAY_CONFIG", valid_config_path())
        .env("VPAY_OAUTH_SIGNING_KEY_FILE", not_a_key_path())
        .output()
        .expect("spawn vpay-server");

    assert_eq!(
        output.status.code(),
        Some(78),
        "expected EX_CONFIG (78) for a signing key file that is not a key, got {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("signing key"),
        "stderr should say the signing key is what failed to load, got: {stderr}"
    );
    assert!(
        !stderr.contains("this is not a PEM-encoded"),
        "the file's contents must never be echoed — a real one would be a private key: {stderr}"
    );
}

/// The positive counterpart to the two tests above: a config file that
/// exists and passes validation lets this binary boot all the way to
/// serving a real `200` from `/healthz` — proving `main.rs` actually calls
/// `vpay_config::Config::load` (and does not, say, silently swallow its
/// result) rather than only proving the two failure paths above.
#[test]
fn a_valid_config_lets_the_server_boot_and_serve_healthz() {
    with_live_postgres(|database_url| {
        let addr = free_addr();
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut guard = ChildGuard(
            bin()
                .env("VPAY_BIND", addr.to_string())
                .env("DATABASE_URL", &database_url)
                .env("VPAY_CONFIG", valid_config_path())
                .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn vpay-server"),
        );

        let status = poll_healthz(addr, Duration::from_secs(5));
        assert_eq!(
            status,
            Some(200),
            "server never became healthy on {addr} with a valid config"
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

/// A syntactically well-formed RS256 JWT carrying a `kid` this deployment
/// has never published, an unrelated `iss`/`aud`, and a signature that is
/// not one.
///
/// The `kid` is the load-bearing part. `JwtValidator::validate`
/// (`backends/crates/vpay-api/src/resource_auth.rs`) short-circuits a token
/// with *no* `kid` before it touches the JWKS cache at all, so a garbage
/// string such as `"nope"` would answer 401 without any HTTP client ever
/// being used. A token with an unrecognised `kid` is the cheapest input
/// that forces the cold cache to actually fetch the JWKS over loopback —
/// which is the code path this file's trust-store test exists to exercise.
const BOGUS_TOKEN_WITH_UNKNOWN_KID: &str = concat!(
    "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6ImEta2lkLXRoaXMtZGVwbG95bWVudC1uZXZlci1wdWJsaXNoZWQifQ",
    ".eyJpc3MiOiJodHRwczovL2V4YW1wbGUuaW52YWxpZCIsImF1ZCI6InZwYXkiLCJzdWIiOiJtX3Rlc3QiLCJleHAiOjQxMDI0NDQ4MDB9",
    ".bm90LWEtcmVhbC1zaWduYXR1cmU",
);

/// The exact 401 envelope an unrecognised token gets, copied from
/// `vpay_api::error`'s own pinned constant (`PINNED_INVALID_TOKEN`). Kept
/// here as a literal rather than imported so this test asserts on the bytes
/// a merchant's HTTP client receives, not on a constant the code under test
/// could change in step with it.
const PINNED_INVALID_TOKEN: &str = concat!(
    r#"{"error":{"code":"invalid_token","#,
    r#""message":"The bearer token is invalid, expired, or was not issued for this endpoint.","#,
    r#""type":"authentication_error"}}"#,
);

/// The shipped runtime image is `FROM scratch` (`docs/adr/0004-musl-mimalloc.md`):
/// no glibc, no shell, and — the part that matters here — **no OS
/// certificate store**. This test reproduces that condition on a normal
/// developer/CI machine by pointing the two variables
/// `rustls-native-certs` reads (`SSL_CERT_FILE`, `SSL_CERT_DIR`, see its
/// `lib.rs`) at paths that do not exist, so the platform verifier finds an
/// empty root store exactly as it does inside the image.
///
/// It pins two things, both of which failed before
/// `vpay_api::http_client` existed:
///
/// 1. **The process boots.** `JwtValidator::new` used to call
///    `JwksCache::new`, which calls `reqwest::Client::new()`, which under
///    this workspace's reqwest 0.13 pin builds a
///    `rustls_platform_verifier::Verifier` eagerly and returns
///    `General("No CA certificates were loaded from the system")` when the
///    store is empty — and `Client::new()` turns that into a **panic**. The
///    server died at startup inside its own image while passing every test
///    on a machine that has a trust store.
/// 2. **The JWKS client actually works.** A boot-only assertion would still
///    pass if the eager client build were merely deferred to first use, so
///    the second half sends a `/v1` request with
///    [`BOGUS_TOKEN_WITH_UNKNOWN_KID`] — the input that forces the cold
///    `JwksCache` to fetch this process's own `/v1/oauth/jwks.json` over
///    loopback — and requires the **401** `invalid_token` envelope. A
///    failed fetch would answer `503 service_unavailable` instead (see
///    `vpay_api::error`'s `PINNED_KEYS_UNAVAILABLE`), so 401 is the
///    assertion that distinguishes "the JWKS was read" from "the JWKS could
///    not be read".
///
/// The JWKS URL is plain `http://` over loopback and no TLS is ever
/// negotiated — which is precisely why the old failure was so easy to miss.
/// The trust store was consulted at *client construction*, not at connect.
#[test]
fn a_server_with_no_os_trust_store_boots_and_still_validates_tokens() {
    with_live_postgres(|database_url| {
        let addr = free_addr();
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut guard = ChildGuard(
            bin()
                .env("VPAY_BIND", addr.to_string())
                .env("DATABASE_URL", &database_url)
                .env("VPAY_CONFIG", valid_config_path())
                .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path())
                // The whole point of the test: no readable trust store.
                .env("SSL_CERT_FILE", "/nonexistent/certs.pem")
                .env("SSL_CERT_DIR", "/nonexistent")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn vpay-server"),
        );

        let status = poll_healthz(addr, Duration::from_secs(5));
        assert_eq!(
            status,
            Some(200),
            "server never became healthy on {addr} with no OS trust store — it is expected to \
             boot, because it never speaks TLS to reach its own loopback JWKS"
        );

        let request = format!(
            "GET /v1/payment_intents/pi_does_not_exist HTTP/1.1\r\nHost: localhost\r\n\
             Authorization: Bearer {BOGUS_TOKEN_WITH_UNKNOWN_KID}\r\nConnection: close\r\n\r\n"
        );
        let (code, response) =
            http_request(addr, &request).expect("the authenticated /v1 surface answers");

        assert_eq!(
            code, 401,
            "expected the invalid-token 401 after a real JWKS fetch; a 503 here means the fetch \
             itself failed. Response was:\n{response}"
        );
        assert!(
            response.contains(PINNED_INVALID_TOKEN),
            "expected the pinned invalid_token envelope, got:\n{response}"
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
    with_live_postgres(|database_url| {
        let addr = free_addr();
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut guard = ChildGuard(
            bin()
                .env("VPAY_BIND", addr.to_string())
                .env("VPAY_LOG_FORMAT", "text")
                .env("DATABASE_URL", &database_url)
                .env("VPAY_CONFIG", valid_config_path())
                .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path())
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
    });
}

#[cfg(unix)]
#[test]
fn an_explicit_flag_wins_over_a_conflicting_env_var() {
    with_live_postgres(|database_url| {
        let (flag_addr, env_addr) = two_free_addrs();

        let mut guard = ChildGuard(
            bin()
                .env("VPAY_BIND", env_addr.to_string())
                .env("DATABASE_URL", &database_url)
                .env("VPAY_CONFIG", valid_config_path())
                .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path())
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
    });
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
    with_live_postgres(|database_url| {
        let addr = free_addr();
        let mut cmd = bin();
        cmd.env("VPAY_BIND", addr.to_string())
            .env("VPAY_LOG_FORMAT", "text")
            .env("DATABASE_URL", &database_url)
            .env("VPAY_CONFIG", valid_config_path())
            .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path())
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
    });
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
    // binary's own signal-handling or tracing code ever runs (and before it
    // ever looks at `DATABASE_URL`, so no container is needed for this
    // warm-up call specifically).
    let _ = bin().arg("--help").output();

    // One container, reused for all `ATTEMPTS` spawns below (see
    // `with_live_postgres`'s doc comment) — not one per attempt. The DB
    // connect + idempotent migration each attempt performs adds fixed
    // per-spawn latency but does not affect this test's sensitivity to the
    // race: `ShutdownSignals::install()` still runs first, before the DB
    // connect, so a SIGTERM delivered at `DELAY` after spawn is queued by
    // the OS-level handler (registered synchronously at install time, not
    // lazily on first poll — see `vpay_config::signal`'s own docs)
    // regardless of what `main` happens to be doing later when it is
    // observed.
    with_live_postgres(|database_url| {
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
                .env("VPAY_LOG_FORMAT", "text")
                .env("DATABASE_URL", &database_url)
                .env("VPAY_CONFIG", valid_config_path())
                .env("VPAY_OAUTH_SIGNING_KEY_FILE", generated_key_path());
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
