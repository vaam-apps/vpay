//! A real signal, to a real process, mid-payment.
//!
//! Every other crash-safety test in this repository *writes the state a crash
//! leaves*. `docs/flows/crash-safety.md` says so in its own words — "they are
//! exercised by writing the state a crash leaves, not by killing a process
//! […] Nothing in this repository kills a process mid-confirm" — and calls
//! the resulting claim "a weaker claim than 'a `SIGKILL` at each point
//! resolves cleanly'". This file is the stronger claim, for three of the
//! moments that matter:
//!
//! | Signalled | While | Resolved by |
//! |---|---|---|
//! | `vpay-server worker` | its status query is in flight | the next worker's boot reap, then the ladder |
//! | `vpay-server` | its `requesttopay` submit is in flight (kill point 2) | the worker's recovery table |
//! | `vpay-server worker` (`SIGTERM`) | its webhook delivery is in flight | **the drain itself** — the job is finished, not re-run |
//!
//! The third row is a different property from the first two and is here
//! rather than in a suite of its own because it is the same question asked of
//! the other signal: `SIGKILL` asks "what does a process that cannot clean up
//! leave behind?", `SIGTERM` asks "does the cleanup a process *is* given the
//! chance to do actually happen?". Issue #85 is the request; before it, the
//! only SIGTERM in any suite went through [`stop_worker_cleanly`], whose own
//! assertion string says "a worker with nothing in flight", and the
//! outstanding-work half was a hand measurement recorded in
//! `docs/flows/crash-safety.md` that nothing re-ran.
//!
//! Nothing here is simulated except the passage of time (see
//! [`age_the_dead_workers_lease`] and [`age_the_crashed_charge`], the two
//! clocks this file cannot wait for). The processes are the shipping binaries, built from this
//! working tree by [`shipping_binary`] and spawned as ordinary OS processes;
//! the database is a real Postgres in a container; the rail is the shared
//! WireMock tree under `backends/tests/conformance/wiremock/mtn`; the signal
//! is `Child::kill()`, i.e. `SIGKILL`, which runs no destructor, flushes no
//! buffer, closes no socket politely and gives the process no opportunity to
//! hand its lease back. The exit status is asserted to be *signalled with 9*
//! rather than merely unsuccessful, so a process that chose to exit(1) on its
//! own could not stand in for one that was killed.
//!
//! The SIGTERM case adds one more real thing and no more simulation: a second
//! WireMock container standing in for the **merchant's receiver**, reached
//! over HTTP at the URL `merchant_clients[].webhooks[].url` names, exactly as
//! `tests/webhooks.rs` reaches it (ADR-0006). Its exit status is asserted to
//! be a plain `0` with **no** signal, because `vpay-server worker` catches
//! SIGTERM: a process that had died *of* the signal would be the regression,
//! not the behaviour.
//!
//! # How a crash is made observable at all
//!
//! A crash is only interesting if it lands *during* an operation, and the
//! operations here are one HTTP request to a rail. So the rail is made slow
//! at exactly one point, by a mapping keyed on a documentation MSISDN
//! (`requesttopay-kill9.json`, 30 s `fixedDelayMilliseconds`) — the same
//! technique, and the same justification, as `mtn-e2e-poll`'s: a confirm
//! cannot choose its `provider_reference_id`, so the only thing a test can
//! steer from outside is the payer's MSISDN, which the merchant supplies in
//! the request body. **No stored state is rewritten to steer a rail here.**
//!
//! Thirty seconds is longer than `vpay_provider::DEFAULT_REQUEST_TIMEOUT`
//! (20 s), so the delayed request can never be answered successfully. A kill
//! that arrived late therefore cannot settle the charge behind the
//! assertions' back; it fails the test instead.
//!
//! # How a SIGTERM is made to land mid-delivery
//!
//! The same way, and the receiver rather than the rail is what is made slow:
//! `slow-ack.json` holds its `200` for [`RECEIVER_ACK_DELAY`]. Two `const`
//! assertions below pin that number *between* the two shipping budgets it has
//! to sit between, so the outcome is chosen by arithmetic the compiler checks
//! rather than by a stopwatch. And because a margin is not a proof, the case
//! reads the victim's own transcript afterwards and fails unless
//! [`DRAIN_BEGAN`] precedes [`WEBHOOK_DELIVERED`] in it — a signal that
//! arrived after the receiver had already answered makes the run say so
//! instead of passing while proving nothing.
//!
//! # What "exactly once" is asserted against
//!
//! Four independent records, because the interesting failure mode of a
//! crash-and-recover is *double* work rather than none:
//!
//! * `charges` — one row per intent (a unique index enforces it, and this
//!   asserts the index is what a recovery meets);
//! * `events` — one `payment_intent.succeeded`, which is one webhook to the
//!   merchant;
//! * `payment_intents.amount_received` — the amount, once;
//! * the rail's own request journal — the number of requests the ladder
//!   implies and no more, in particular **no second submit**, which is the
//!   double-charge `docs/flows/crash-safety.md`'s retry rule exists to
//!   prevent.
//!
//! For the SIGTERM case a fifth record joins them, and it is the one a
//! merchant actually experiences: **the receiver's own request journal**, which
//! must hold exactly one POST. A drain that dropped the in-flight send would
//! leave two — the one the receiver had already accepted, and the one a
//! worker re-sent from the still-`pending` row.
//!
//! # No test doubles
//!
//! The two binaries under test are the ones `backends/Dockerfile` builds and
//! `deploy/helm/vpay` runs. Nothing in this file links a fake rail, a fake
//! clock or a seam that exists only in tests: the only thing it reaches into
//! is the `jobs` table's own lease timestamp, and that is the test controlling
//! *the queue's clock*, exactly as `support::make_every_job_runnable` and
//! `worker_recovery.rs::strand_the_poll_job` already do.

// This suite is Unix-only, and the whole file rather than a scattering of
// `#[cfg(unix)]` blocks: its subject *is* a POSIX signal. On a non-Unix host
// it compiles to an empty test binary rather than to a differently-shaped
// test that would claim to prove the same thing.
#![cfg(unix)]
// This whole compilation unit is an integration-test binary; clippy's
// "is this a test" detection does not extend to free helper functions in a
// `tests/*.rs` crate even though `clippy.toml` exempts tests. Same header,
// for the same reason, as `vpay-server/tests/cli.rs`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::BufRead as _;
use std::net::{SocketAddr, TcpListener};
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use serde_json::{Value, json};
use sqlx::PgPool;
use testcontainers::{ContainerAsync, GenericImage};
use testcontainers_modules::postgres::Postgres as PostgresImage;
use uuid::Uuid;
use vpay_config::{Config, CurrencyEntry, Deployment, HostEntry, ProviderHost};
use vpay_db::Repositories;
use vpay_sdk::{
    ConfirmPaymentIntentParams, CreatePaymentIntentParams, Credentials, IntentStatus,
    PaymentMethodType, RequestOptions,
};

mod support;

use support::{ensure_crypto_provider_installed, generate_key, merchant_client, migrated_postgres};

const CLIENT_A: &str = "acme-cameroon";
const MERCHANT_A: &str = "acme-cameroon-tenant";
const RAIL: &str = "mtn_momo";
const CURRENCY: &str = "xaf";
const AMOUNT: i64 = 5000;

/// The documentation MSISDN that arms `mtn-kill9-slow-status`: the confirm is
/// answered normally, and the **status query** that follows takes 30 s.
const SLOW_STATUS_MSISDN: &str = "237600000ce9";

/// The documentation MSISDN whose **submit** takes 30 s to answer. What
/// stages `docs/flows/crash-safety.md`'s kill point 2 against the shipping
/// server.
const SLOW_SUBMIT_MSISDN: &str = "237600000cf9";

/// The documentation MSISDN the SIGTERM scenario confirms with, and the only
/// one in this file that arms **nothing**.
///
/// That is its whole job. The rail must behave completely ordinarily there —
/// a 202 from `requesttopay.json`'s general mapping, a `SUCCESSFUL` from
/// `requesttopay-status.json`'s catch-all — because the thing made slow in
/// that scenario is the *merchant's receiver*, and a rail that also dawdled
/// would put a second unbounded wait inside the same test. It continues the
/// `0ce9`/`0cf9` convention of the two above, with `c15` naming the signal
/// (SIGTERM is 15) rather than a rail behaviour; no mapping mentions it, and
/// `an_ordinary_msisdn_arms_no_mapping_in_the_shared_rail_tree` is what keeps
/// that true if one ever does.
const SIGTERM_MSISDN: &str = "237600000c15";

/// The endpoint id and secret the SIGTERM scenario's receiver is registered
/// under in `merchant_clients[].webhooks[]`.
///
/// The secret is not decoration: the delivered bytes are handed to
/// `vpay_sdk::webhooks::verify` under it, so "one signed POST" is a claim
/// about a signature a merchant's own SDK accepts rather than about a header
/// being present.
const RECEIVER_ENDPOINT_ID: &str = "kill9-receiver";
/// See [`RECEIVER_ENDPOINT_ID`].
const RECEIVER_SECRET: &str = "whsec_kill9_sigterm";

/// The receiver path `slow-ack.json` answers slowly on.
///
/// A path of its own rather than `/webhooks`: `tests/webhooks.rs` asserts
/// exact ladder timings against the same mounted mapping tree, and a delay on
/// the path it uses would slow all of it.
const RECEIVER_PATH: &str = "/slow-ack";

/// How long `slow-ack.json` holds its `200`, and therefore how long the
/// delivery is observably in flight.
///
/// Transcribed from that mapping, and the two assertions below are why it is
/// a constant here rather than a number there: this is the *interval within
/// which the signal must land*, and it has to be checkable against the
/// shipping budgets on either side of it.
const RECEIVER_ACK_DELAY: Duration = Duration::from_secs(6);

/// `--shutdown-grace-seconds` for every worker this file spawns.
///
/// Named once, here, and read by [`spawn_worker`]: the SIGTERM scenario's
/// whole outcome is a comparison against this value, so a copy of it in the
/// environment block would be a copy that could drift out from under the
/// assertion below.
const SHUTDOWN_GRACE_SECONDS: u64 = 20;

/// **The delivery must be answerable before the delivery client gives up.**
///
/// `handle_deliver` builds its client with
/// `vpay_worker::webhooks::WEBHOOK_REQUEST_TIMEOUT`; a receiver slower than
/// that produces a transport failure, one rung of the delivery ladder and a
/// *second* POST, which is the opposite of what the scenario asserts. Checked
/// at compile time so that lowering the shipping budget fails the build
/// instead of making the test flaky.
const _: () = assert!(
    RECEIVER_ACK_DELAY.as_secs() < vpay_worker::webhooks::WEBHOOK_REQUEST_TIMEOUT.as_secs(),
    "the receiver must answer before the delivery client's own deadline"
);

/// **And the drain must be able to wait for it.**
///
/// If the acknowledgement arrived after the grace period the drain would
/// abort the task, hand the lease back and the process would exit `1`
/// (`Drain::TimedOut`) — a real behaviour, but not this one, and the case
/// asserts a `0`. Checked at compile time for the reason above.
const _: () = assert!(
    RECEIVER_ACK_DELAY.as_secs() < SHUTDOWN_GRACE_SECONDS,
    "the drain must outlast the receiver, or the exit code under test changes"
);

/// The `Vpay-Signature` tolerance the SDKs default to
/// (`docs/flows/webhooks.md`), used as-is: widening it would stop proving
/// that the `t` on the delivered header is a *current* timestamp.
const SIGNATURE_TOLERANCE: Duration = Duration::from_secs(300);

/// The `financialTransactionId` the `mtn-kill9-slow-status` scenario returns,
/// distinct from every other mapping's so an assertion on
/// `charges.provider_txn_id` cannot pass by reading another stub's answer.
const KILL9_TXN_ID: &str = "kill9-1234567893";

/// The `financialTransactionId` of `requesttopay-status.json`'s catch-all
/// `SUCCESSFUL`, which is what answers the recovery poll in the
/// killed-server case (that charge's reference has no mapping of its own).
const CATCH_ALL_TXN_ID: &str = "1234567890";

/// `vpay_db::provider_requests::STATUS_CODE_NOT_CARRIED_BY_THE_PORT`, the `0`
/// sentinel migration `0020` introduced for "the rail answered and the port
/// does not carry its status line" — `vpay_provider::Submitted` exposes
/// `ref_extra`/`redirect_url` and nothing about MTN's actual `202`, by design
/// (ADR-0002: the core must not branch on a transport detail). Transcribed
/// rather than imported, the same way `worker_recovery.rs`'s
/// `ANSWERED_SENTINEL` is, so this file says out loud which value a
/// *successful* submit attempt is recorded with.
const ANSWERED_SENTINEL: i32 = 0;

/// How long a spawned binary is given to reach its "I am running" log line.
///
/// It has to connect to Postgres, run every migration and reconcile
/// configuration first, so this is startup plus container-scheduling margin,
/// not a guess at how long a boot "should" take.
const BOOT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a settlement is given once a worker is running.
///
/// The recovery poll is answered immediately by the stub (the delay applies
/// only to the *first* status query, which is the one that was killed), so
/// this is a ceiling on a wait that normally ends in a second or two.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the in-flight rail request is given to appear.
///
/// Bounded well under the stub's own 30 s delay: if the request has not
/// reached the rail by now, the kill would not be landing mid-flight and the
/// test must say so rather than kill something and assert about it anyway.
const IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the in-flight **webhook delivery** is given to appear.
///
/// Much longer than [`IN_FLIGHT_TIMEOUT`] because it is a sum of shipping
/// delays rather than one: the confirm's poll job runs at
/// `vpay_worker::poll_delay(0)` (10 s), the settled event waits up to one
/// `FAN_OUT_IDLE` (5 s) for the outbox drain that turns it into a delivery,
/// and the delivery job is then claimed by whichever worker asks next. Fifty
/// seconds is roughly triple that, and it bounds the wait rather than
/// estimating it — the *precision* the scenario needs is in
/// [`RECEIVER_ACK_DELAY`], not here.
const DELIVERY_IN_FLIGHT_TIMEOUT: Duration = Duration::from_secs(50);

/// How long the signalled worker is given to be gone.
///
/// Its own drain budget plus margin: a worker still alive after this has
/// neither drained nor timed out, which is a hang rather than either outcome
/// the case distinguishes.
const SIGTERM_EXIT_TIMEOUT: Duration = Duration::from_secs(SHUTDOWN_GRACE_SECONDS + 30);

/// How long the reader threads are given to hand over the last lines an
/// exited process wrote.
///
/// The channel disconnects when both threads see EOF, so this is a ceiling on
/// a wait that ends as soon as the pipes close, not a sleep.
const LOG_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// `vpay_config::signal` — the process saying it heard the signal at all.
const SIGTERM_HEARD: &str = "received SIGTERM, starting graceful shutdown";
/// `vpay_worker::run_loop` — the drain starting, i.e. the grace clock
/// starting. **The decisive line**: everything the drain is credited with
/// must be logged after it.
const DRAIN_BEGAN: &str = "shutdown signalled; draining in-flight jobs";
/// `vpay_worker::webhooks::record_receiver_answer`, logged only when the
/// compare-and-swap that settles the delivery actually changed the row.
const WEBHOOK_DELIVERED: &str = "webhook delivered";
/// `vpay_server::worker` on the `Drain::Clean` arm — the branch that exits
/// `0`.
const SHUTDOWN_COMPLETE: &str = "graceful shutdown complete, exiting";
/// `vpay_worker::run_loop` on the other arm, whose absence is asserted:
/// `Drain::TimedOut` exits `1` and hands leases back.
const DRAIN_TIMED_OUT: &str = "the shutdown grace period elapsed before in-flight jobs finished";

fn mappings_dir(rail: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../conformance/wiremock")
        .join(rail)
}

/// The merchant-receiver mapping tree, which is **not** under
/// `conformance/wiremock`: a receiver is not a rail.
///
/// The same directory `compose.e2e.yml` mounts into `wiremock-webhook` and
/// the same one `tests/webhooks.rs` uses, so the receiver this file's SIGTERM
/// case delivers to is the receiver a developer's stack answers with.
fn receiver_mappings_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../webhook-receiver/wiremock")
}

// ------------------------------------------------------------- the binaries

/// Builds and returns the path of a shipping binary, from *this* working
/// tree.
///
/// # Why this is not `env!("CARGO_BIN_EXE_…")`
///
/// Cargo sets `CARGO_BIN_EXE_<name>` only for integration tests **of the
/// package that declares the binary** — which is why
/// `vpay-server/tests/cli.rs` can use it and this suite cannot. This
/// package cannot depend on the app either: it has a library target, but
/// nothing in it is the `main` this suite has to kill, and artifact
/// dependencies are nightly-only.
///
/// Since 2026-09-07 (issue #77) there is one shipping binary rather than
/// two: `vpay-server`, with the worker behind a `worker` subcommand. This
/// helper still takes a package name, because what it does is "build a
/// binary out of this tree and hand me its path", and which *mode* the
/// caller then runs is the caller's argv — see [`spawn_worker`].
///
/// # Why it runs `cargo build` rather than just computing a path
///
/// The path (`<target>/<profile>/vpay-server`, derived from this test
/// binary's own location) may hold a binary built from *older sources*, or
/// none at all — `cargo nextest run -p vpay-tests-integration` has no reason
/// to build another package's binary. A stale binary is the worse of the two
/// failures: the suite would go green having proved something about code that
/// is no longer in the tree. So the build is part of the test, into the same
/// target directory, with the same profile, and a failure is a panic carrying
/// cargo's own stderr.
///
/// `--offline` because `just ci` is expected to run without a network, and
/// everything this needs is already resolved: the run that compiled *this*
/// test binary resolved the same workspace lockfile.
fn shipping_binary(package: &str) -> PathBuf {
    let test_exe = std::env::current_exe().expect("the running test binary has a path");
    // `<target>/<profile>/deps/<name>-<hash>` — up two levels is the profile
    // directory, which is where cargo puts binaries, and up three is the
    // target directory cargo must be told to reuse.
    let profile_dir = test_exe
        .parent()
        .and_then(Path::parent)
        .expect("the test binary lives in <target>/<profile>/deps")
        .to_path_buf();
    let target_dir = profile_dir
        .parent()
        .expect("the profile directory lives in <target>")
        .to_path_buf();
    let profile_name = profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .expect("the profile directory has a name");
    // The `debug` *directory* is the `dev` *profile*; every other profile
    // directory is named after the profile itself.
    let cargo_profile = if profile_name == "debug" {
        "dev"
    } else {
        profile_name
    };

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .args([
            "build",
            "--quiet",
            "--offline",
            "--profile",
            cargo_profile,
            "-p",
            package,
            "--bin",
            package,
            "--manifest-path",
        ])
        .arg(&manifest)
        .env("CARGO_TARGET_DIR", &target_dir)
        .output()
        .expect("invoking cargo to build the binary under test");
    assert!(
        output.status.success(),
        "could not build {package} (the process this test is about):\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let path = profile_dir.join(package);
    assert!(
        path.is_file(),
        "cargo reported success but {} is not there; the profile directory was derived from \
         this test binary's own path ({})",
        path.display(),
        test_exe.display()
    );
    path
}

/// A spawned shipping binary, its captured output, and a `Drop` that never
/// leaks it.
///
/// Both streams are merged into one channel because the assertions are about
/// *what the process said*, not about which fd it said it on, and because a
/// boot failure prints on stderr while a running worker logs on stdout — a
/// suite that captured only one would report "no such line" for a process
/// that had explained itself perfectly well on the other.
struct Proc {
    name: &'static str,
    child: Child,
    lines: Receiver<String>,
    /// Every line taken off the channel so far, kept so a failure message can
    /// show the whole log rather than only the part that arrived after the
    /// assertion started waiting.
    seen: Vec<String>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Proc {
    fn spawn(name: &'static str, mut cmd: Command) -> Self {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("spawning {name}: {error}"));
        let (tx, rx) = mpsc::channel();
        for stream in [
            Box::new(child.stdout.take().expect("piped stdout")) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().expect("piped stderr")),
        ] {
            let tx = tx.clone();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(stream).lines() {
                    match line {
                        Ok(line) => {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });
        }
        Self {
            name,
            child,
            lines: rx,
            seen: Vec::new(),
        }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Blocks until a captured line contains `needle`, or `within` elapses.
    ///
    /// Synchronous on purpose even though the callers are `async`: this
    /// waits on a process, not on the runtime, and every wait here is
    /// bounded.
    fn wait_for_line(&mut self, needle: &str, within: Duration) -> Option<String> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(found) = self.seen.iter().find(|line| line.contains(needle)) {
                return Some(found.clone());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.lines.recv_timeout(remaining) {
                Ok(line) => self.seen.push(line),
                Err(_) => return None,
            }
        }
    }

    /// Everything the process has said, for a failure message.
    fn log(&mut self) -> String {
        self.absorb();
        format!("--- {} output ---\n{}", self.name, self.seen.join("\n"))
    }

    /// Moves whatever has already arrived onto [`Self::seen`], without
    /// waiting.
    ///
    /// For a process that is still running: it says what has been said *so
    /// far*, which is what an assertion about something a process must
    /// **not** have done needs.
    fn absorb(&mut self) {
        while let Ok(line) = self.lines.try_recv() {
            self.seen.push(line);
        }
    }

    /// Blocks until both reader threads have hung up, i.e. everything the
    /// process ever wrote is on [`Self::seen`].
    ///
    /// [`Self::absorb`] is racy for a process that has just exited: the
    /// reader threads may not have pushed its last lines yet, and the last
    /// lines of a draining worker are exactly the ones under assertion. Both
    /// senders are dropped when the pipes close, so this ends at EOF rather
    /// than at `within` in every non-pathological case; `within` only bounds
    /// a reader that is somehow still alive.
    fn absorb_until_closed(&mut self, within: Duration) {
        let deadline = Instant::now() + within;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            match self.lines.recv_timeout(remaining) {
                Ok(line) => self.seen.push(line),
                // Disconnected: both readers are done, which is what is
                // being waited for. Timeout: `within` is up.
                Err(_) => return,
            }
        }
    }

    /// Where `needle` first appears in what has been absorbed, or `None`.
    ///
    /// An **index** rather than a bool because the SIGTERM case's decisive
    /// assertion is about the *order* of two lines one process wrote: both
    /// go to stdout (`vpay-server` logs to stdout for every long-running
    /// mode — `logs_to_stderr`), one thread reads that pipe, so the order
    /// here is the order the process wrote them in.
    fn said(&self, needle: &str) -> Option<usize> {
        self.seen.iter().position(|line| line.contains(needle))
    }

    /// `SIGKILL`, and the exit status it produced.
    ///
    /// `Child::kill` is `kill(2)` with `SIGKILL`: uncatchable, unblockable,
    /// no unwinding, no `Drop`, no flush. That is the whole point — a
    /// `SIGTERM` here would exercise the graceful drain, which is a
    /// different (and already tested) thing.
    fn kill9(&mut self) -> ExitStatus {
        self.child.kill().expect("SIGKILL the process under test");
        self.child.wait().expect("reap the killed process")
    }

    /// `SIGTERM`, sent with the `kill` utility rather than a `libc` FFI call
    /// so this stays inside the workspace's `unsafe_code = "forbid"`. Same
    /// approach as `vpay-worker-bin/tests/cli.rs`.
    fn sigterm(&self) {
        let pid = self.pid().to_string();
        let status = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .expect("invoke `kill`");
        assert!(status.success(), "`kill -TERM {pid}` itself failed to run");
    }

    /// Polls for exit up to `within`; force-kills and returns `None` if the
    /// process outlasts it (`std::process::Child` has no timed wait).
    fn wait_within(&mut self, within: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + within;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

// ------------------------------------------------------------- the fixtures

/// A directory this test owns, holding the YAML configuration (and, for the
/// server case, the signing key) the spawned processes read.
///
/// A real process reads its configuration from a file — that is ADR-0003 and
/// there is no in-memory `Config` to hand it — so a file has to exist. It is
/// removed on drop; a leaked one holds a stub rail's URL and a generated
/// test key, nothing else.
struct Workspace(PathBuf);

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Workspace {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("vpay-kill9-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("creating this test's own directory");
        Self(dir)
    }

    /// Writes `config` as the YAML a shipping binary loads.
    ///
    /// Serialized from the *same* `vpay_config::Config` value the in-process
    /// harness uses rather than written out by hand: the two processes must
    /// agree on the rail host, and two spellings of one configuration is
    /// exactly how they would stop agreeing. `Config` round-trips through
    /// this serializer inside `Config::load_with_env` already.
    fn write_config(&self, config: &Config) -> PathBuf {
        let path = self.0.join("application.yml");
        let yaml = serde_yaml_ng::to_string(config).expect("the configuration serializes");
        std::fs::write(&path, yaml).expect("writing the configuration");
        path
    }

    fn write_signing_key(&self, pem: &str) -> PathBuf {
        let path = self.0.join("oauth-signing-key.pem");
        std::fs::write(&path, pem).expect("writing the signing key");
        path
    }
}

/// One rail, on XAF, `livemode: false` — the shape `config/application.yml`
/// has, and the same one `worker_e2e.rs` builds.
fn config_with(base_url: &str, mtn_url: &str, jwks_a: Value) -> Config {
    Config {
        deployment: Deployment {
            name: "worker-kill9".to_owned(),
            livemode: false,
            public_base_url: base_url.to_owned(),
        },
        providers: vec![ProviderHost {
            code: RAIL.to_owned(),
            enabled: true,
            host: HostEntry {
                url: mtn_url.to_owned(),
                label: "mtn-wiremock".to_owned(),
            },
            settings: BTreeMap::from([
                ("target_environment".to_owned(), "sandbox".to_owned()),
                (
                    "api_user".to_owned(),
                    "11111111-2222-3333-4444-555555555555".to_owned(),
                ),
            ]),
            callback_url: None,
            currency: CURRENCY.to_ascii_uppercase(),
            credentials: BTreeMap::from([
                (
                    "subscription_key".to_owned(),
                    "stub-subscription-key".to_owned(),
                ),
                ("api_key".to_owned(), "stub-api-key".to_owned()),
            ]),
        }],
        currencies: vec![CurrencyEntry {
            code: CURRENCY.to_ascii_uppercase(),
            exponent: 0,
        }],
        merchant_clients: vec![merchant_client(CLIENT_A, MERCHANT_A, jwks_a)],
        // No webhook endpoint is configured above, so the two `SIGKILL`
        // cases deliver none and the default
        // (`allow_private_targets: false`) is what they run under.
        // `config_with_receiver` is the one caller that overrides both, and
        // it says why. Named rather than defaulted away with `..`: this
        // file's whole subject is what the shipping binaries do, and a config
        // literal that hid a field would let one drift.
        webhooks: vpay_config::WebhookPolicy::default(),
        checkout: vpay_config::CheckoutConfig::default(),
        dashboard_client: None,
        staff_auth: vpay_config::StaffAuth::default(),
    }
}

/// [`config_with`] plus a merchant webhook endpoint pointing at the receiver
/// container, which is the only difference the SIGTERM scenario needs.
///
/// # Why `allow_private_targets` has to be `true` here
///
/// The receiver is a container this host reaches at `127.0.0.1:<mapped>`, and
/// `vpay_worker::ssrf` classifies loopback as private. It is the same value,
/// for the same reason, that `compose.e2e.yml`'s sandbox profile sets for
/// `wiremock-webhook`, and it is a **configuration** value rather than a code
/// path (ADR-0003): the guard still resolves the host and still classifies
/// every address it answers with, and the delivery client is still pinned to
/// them. Only the verdict on a private address changes.
///
/// `deployment.livemode` stays `false`, and `Config::validate_all` refuses
/// the two together — so this cannot be the shape of a production
/// deployment even by accident.
fn config_with_receiver(
    base_url: &str,
    mtn_url: &str,
    jwks_a: Value,
    endpoint_url: &str,
) -> Config {
    let mut config = config_with(base_url, mtn_url, jwks_a.clone());
    config.merchant_clients = vec![support::merchant_client_with(
        CLIENT_A,
        MERCHANT_A,
        jwks_a,
        &[vpay_api::SCOPE_PAYMENTS_WRITE],
        vec![support::webhook_endpoint(
            RECEIVER_ENDPOINT_ID,
            endpoint_url,
            &[RECEIVER_SECRET],
        )],
        Vec::new(),
    )];
    config.webhooks = vpay_config::WebhookPolicy {
        allow_private_targets: true,
    };
    config
}

fn create_params() -> CreatePaymentIntentParams {
    CreatePaymentIntentParams {
        amount: AMOUNT,
        currency: CURRENCY.to_owned(),
        payment_method_types: vec![PaymentMethodType::MtnMomo],
        metadata: BTreeMap::new(),
        description: None,
        customer: None,
    }
}

fn sdk_client(base_url: &str, pem: &str) -> vpay_sdk::Client {
    vpay_sdk::Client::builder(base_url)
        .credentials(Credentials::rsa_pem(CLIENT_A, pem).expect("the generated PEM parses"))
        .build()
        .expect("the SDK client builds from a base URL and a credential")
}

/// Binds an ephemeral port, reads it back, then frees it — the spawned server
/// binds it a moment later. The same helper, and the same reasoning about
/// fixed ports, as `vpay-server/tests/cli.rs`'s.
fn free_addr() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("local_addr")
}

/// Postgres and the MTN stub, started together.
///
/// Both handles on the database come back: `Arc<dyn Repositories>` is what the
/// shipping code takes (`support::serve`, and everything Step 7 moved behind
/// the repository traits), and the raw `PgPool` is what the assertions below
/// read the *schema* with — a kill test asserts on columns no repository
/// method exposes (`jobs.locked_by`, `provider_requests.error_kind`), which is
/// the same split `support::migrated_postgres` hands every suite here.
async fn containers() -> anyhow::Result<(
    ContainerAsync<PostgresImage>,
    Arc<dyn Repositories>,
    PgPool,
    String,
    ContainerAsync<GenericImage>,
    String,
)> {
    ensure_crypto_provider_installed();
    let (postgres, repositories, pool) = migrated_postgres().await?;
    let host = postgres.get_host().await.context("postgres host")?;
    let port = postgres
        .get_host_port_ipv4(5432)
        .await
        .context("postgres port")?;
    // The spawned processes connect over TCP to the same container this
    // test's own pool uses, so every assertion below reads the rows those
    // processes wrote — not a copy.
    let database_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let mtn = vpay_testkit::containers::start_wiremock(&mappings_dir("mtn"))
        .await
        .context("the MTN stub container starts")?;
    let mtn_url = format!(
        "http://127.0.0.1:{}",
        mtn.get_host_port_ipv4(8080)
            .await
            .context("the MTN stub's mapped port")?
    );

    Ok((postgres, repositories, pool, database_url, mtn, mtn_url))
}

// ------------------------------------------------------------ rail journal

/// How many requests the rail has *received* matching `criteria`.
///
/// WireMock journals a request when it matches it, before any
/// `fixedDelayMilliseconds` is served — verified against
/// `wiremock/wiremock:3.9.2` before this suite was written, and it is what
/// makes an in-flight request observable from outside the process that made
/// it. That matters for the assertions after the kill: the killed process
/// cannot tell anyone what it did, so the rail's own journal is the
/// independent witness.
async fn journal_count(mtn_url: &str, criteria: Value) -> anyhow::Result<u64> {
    let body: Value = reqwest::Client::new()
        .post(format!("{mtn_url}/__admin/requests/count"))
        .json(&criteria)
        .send()
        .await
        .context("asking the rail's admin API for a request count")?
        .json()
        .await
        .context("reading the request count")?;
    body.get("count")
        .and_then(Value::as_u64)
        .with_context(|| format!("the admin API answered without a count: {body}"))
}

// ------------------------------------------------------ receiver journal

/// One POST as the **merchant's receiver** recorded it.
///
/// Read back out of WireMock rather than out of the sender, for
/// `tests/webhooks.rs`'s reason: re-signing in the test and comparing would
/// only prove that one function agrees with itself.
struct RecordedPost {
    /// The path, so a POST to some other endpoint could never be counted as
    /// this delivery.
    url: String,
    /// The exact bytes, which are what the signature covers.
    body: Vec<u8>,
    /// Header names lower-cased; HTTP header names are case-insensitive and
    /// WireMock echoes whatever casing the sender used.
    headers: BTreeMap<String, String>,
}

impl RecordedPost {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Hand-written so the body renders as the JSON it is.
///
/// The derived one prints `Vec<u8>` as several hundred decimal numbers, and
/// the assertion this type appears in — "the merchant was told twice" — is
/// one whose failure message an operator has to be able to read. Lossy on
/// purpose: a body that is not UTF-8 is itself worth seeing rather than
/// hiding behind an error.
impl std::fmt::Debug for RecordedPost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordedPost")
            .field("url", &self.url)
            .field("body", &String::from_utf8_lossy(&self.body))
            .field("headers", &self.headers)
            .finish()
    }
}

/// Every POST the receiver has recorded on [`RECEIVER_PATH`], oldest first.
async fn receiver_posts(receiver_url: &str) -> anyhow::Result<Vec<RecordedPost>> {
    let body: Value = reqwest::Client::new()
        .get(format!("{receiver_url}/__admin/requests"))
        .send()
        .await
        .context("asking the receiver for its request journal")?
        .json()
        .await
        .context("reading the receiver's request journal")?;
    let entries = body
        .get("requests")
        .and_then(Value::as_array)
        .with_context(|| format!("the receiver's journal has a `requests` array: {body}"))?;

    let mut posts = Vec::new();
    for entry in entries {
        let request = entry
            .get("request")
            .context("a journal entry carries a request")?;
        if request.get("method").and_then(Value::as_str) != Some("POST") {
            continue;
        }
        let url = request
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if url != RECEIVER_PATH {
            continue;
        }
        posts.push(RecordedPost {
            url,
            body: request
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .as_bytes()
                .to_vec(),
            headers: request
                .get("headers")
                .and_then(Value::as_object)
                .map(|map| {
                    map.iter()
                        .filter_map(|(name, value)| {
                            value
                                .as_str()
                                .map(|value| (name.to_ascii_lowercase(), value.to_owned()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    // WireMock answers newest first; chronological is what a "one and only
    // one" assertion reads.
    posts.reverse();
    Ok(posts)
}

fn status_queries() -> Value {
    json!({ "method": "GET", "urlPathPattern": "/collection/v1_0/requesttopay/.*" })
}

fn submits() -> Value {
    json!({ "method": "POST", "urlPath": "/collection/v1_0/requesttopay" })
}

// --------------------------------------------------------------- the reads

#[derive(Debug, sqlx::FromRow)]
struct StoredCharge {
    id: String,
    state: String,
    provider_txn_id: Option<String>,
    provider_reference_id: Uuid,
}

async fn charge_for(pool: &PgPool, intent_id: &str) -> anyhow::Result<StoredCharge> {
    sqlx::query_as::<_, StoredCharge>(
        "SELECT id, state::TEXT AS state, provider_txn_id, provider_reference_id \
         FROM charges WHERE payment_intent_id = $1",
    )
    .bind(intent_id)
    .fetch_one(pool)
    .await
    .context("reading the charge")
}

async fn charge_count(pool: &PgPool, intent_id: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM charges WHERE payment_intent_id = $1")
        .bind(intent_id)
        .fetch_one(pool)
        .await
        .context("counting the charges")
}

/// `(status, amount_received)` — the second is not on the wire, so it can
/// only be read here (`vpay_api::model` does not carry it).
async fn stored_intent(pool: &PgPool, id: &str) -> anyhow::Result<(String, i64)> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT status::TEXT, amount_received FROM payment_intents WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .context("reading the intent")
}

/// The one event this object has, by id.
///
/// Separate from [`event_types`] rather than folded into it because the two
/// answer different questions: that one is the "exactly one webhook" count,
/// this one is what the delivered body and the `Vpay-Event-Id` header are
/// checked against. `fetch_one` is part of the assertion — a second event
/// here is a second webhook, and it should fail loudly rather than be
/// silently truncated to the first.
async fn event_id_for(pool: &PgPool, object_id: &str) -> anyhow::Result<String> {
    sqlx::query_scalar("SELECT id FROM events WHERE object_id = $1")
        .bind(object_id)
        .fetch_one(pool)
        .await
        .context("reading the event's id")
}

async fn event_types(pool: &PgPool, object_id: &str) -> anyhow::Result<Vec<String>> {
    sqlx::query_scalar("SELECT type::TEXT FROM events WHERE object_id = $1 ORDER BY seq")
        .bind(object_id)
        .fetch_all(pool)
        .await
        .context("reading the events table")
}

#[derive(Debug, sqlx::FromRow)]
struct StoredJob {
    attempts: i32,
    locked_by: Option<String>,
}

async fn poll_job(pool: &PgPool, charge_id: &str) -> anyhow::Result<Option<StoredJob>> {
    job_by_dedupe_key(pool, &vpay_worker::jobs::poll_dedupe_key(charge_id)).await
}

/// The `deliver_webhook` job for one delivery, if the queue still has it.
///
/// Its `locked_by` is the whole reason this exists: it names the process
/// whose task is inside `handle_deliver`, which is how the SIGTERM case picks
/// which of two running workers to signal, and `None` afterwards is the
/// queue's own statement that the job was finished rather than left behind.
async fn delivery_job(pool: &PgPool, delivery_id: Uuid) -> anyhow::Result<Option<StoredJob>> {
    job_by_dedupe_key(pool, &vpay_worker::jobs::webhook_dedupe_key(delivery_id)).await
}

async fn job_by_dedupe_key(pool: &PgPool, key: &str) -> anyhow::Result<Option<StoredJob>> {
    sqlx::query_as::<_, StoredJob>("SELECT attempts, locked_by FROM jobs WHERE dedupe_key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("reading the job `{key}`"))
}

/// What vpay owes a merchant, and what happened on each attempt
/// (migration `0022`).
#[derive(Debug, Clone, sqlx::FromRow)]
struct StoredDelivery {
    id: Uuid,
    endpoint_id: String,
    url: String,
    /// Failed attempts, so it is directly the retry-ladder index. A
    /// first-attempt success leaves it `0`, which is what makes a non-zero
    /// value here evidence that a send was cut off and re-run.
    attempt: i32,
    state: String,
    status_code: Option<i32>,
}

/// Every delivery in the database, oldest first.
///
/// Unscoped on purpose: "exactly one delivery row" is the assertion, and a
/// query filtered to the one the test already knows about could not fail it.
async fn deliveries(pool: &PgPool) -> anyhow::Result<Vec<StoredDelivery>> {
    sqlx::query_as::<_, StoredDelivery>(
        "SELECT id, endpoint_id, url, attempt, state, status_code \
         FROM webhook_deliveries ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .context("reading the webhook deliveries")
}

/// Every attempt this charge has on record: `(operation, status_code,
/// error_kind)`.
///
/// `status_code IS NULL` **and** `error_kind IS NULL` together are migration
/// `0016`'s encoding for "issued, never answered" — the row a process writes
/// before it touches the network and cannot complete if it is killed. It is
/// this suite's own proof, from vpay's bookkeeping rather than the rail's,
/// that the request really was in flight when the signal arrived.
async fn attempts(
    pool: &PgPool,
    charge_id: &str,
) -> anyhow::Result<Vec<(String, Option<i32>, Option<String>)>> {
    sqlx::query_as::<_, (String, Option<i32>, Option<String>)>(
        "SELECT operation, status_code, error_kind FROM provider_requests \
         WHERE charge_id = $1 ORDER BY sent_at",
    )
    .bind(charge_id)
    .fetch_all(pool)
    .await
    .context("reading the provider_requests rows")
}

/// The assertion `docs/flows/crash-safety.md` cares about most: whatever the
/// recovery did, every attempt against the rail carried **one** reference.
/// "A fresh reference on retry is how you double-charge a customer."
async fn assert_one_reference(pool: &PgPool, charge_id: &str, expected: Uuid) {
    let references = support::attempted_references(pool, charge_id)
        .await
        .expect("reading the attempted references");
    assert_eq!(
        references,
        vec![expected],
        "every provider_requests row for a charge must carry the same \
         provider_reference_id; a second one is a second payment"
    );
}

// --------------------------------------------------------------- the clock

/// Ages a dead worker's lease past `RecoveryPolicy::lease`, so the next
/// worker's reaper can free it.
///
/// **This and `age_the_crashed_charge` are the only things in this file that
/// are simulated, and both are time.**
/// `vpay-worker-bin` builds `RecoveryPolicy::default()` — a five-minute lease
/// — and takes no flag for it, so a test that waited for the lease to expire
/// for real would take five minutes. Moving `locked_at` into the past is the
/// test controlling *the queue's clock*, the same thing
/// `support::make_every_job_runnable` does for `run_at` and
/// `worker_recovery.rs::strand_the_poll_job` does for exactly this column.
///
/// What is *not* simulated: the lease itself was taken by a real worker
/// process that was really killed, `locked_by` is that process's own
/// `worker_id`, and the write below is guarded on it — so this cannot free a
/// lease belonging to anything else, and the reap, the claim and the re-run
/// that follow are all the shipping code's.
async fn age_the_dead_workers_lease(
    pool: &PgPool,
    charge_id: &str,
    dead_worker: &str,
) -> anyhow::Result<()> {
    let aged = sqlx::query(
        "UPDATE jobs SET locked_at = locked_at - INTERVAL '10 minutes' \
         WHERE dedupe_key = $1 AND locked_by = $2",
    )
    .bind(vpay_worker::jobs::poll_dedupe_key(charge_id))
    .bind(dead_worker)
    .execute(pool)
    .await
    .context("ageing the dead worker's lease")?
    .rows_affected();
    anyhow::ensure!(
        aged == 1,
        "expected exactly one job leased by {dead_worker}, aged {aged}"
    );
    Ok(())
}

/// Ages a charge the killed server left `submitting` past
/// `RecoveryPolicy::not_found_window`, so the next worker may recover it.
///
/// Since lane G (`vpay_worker::recovery`'s "Nothing younger than the window
/// is recovered"), a `submitting` charge younger than the window is one whose
/// confirm may still be running, and the worker leaves it alone — that is
/// the fix for the confirm/worker race the demo exposed. A crashed server's
/// charge is only distinguishable from a live confirm's by age, so a test
/// that waited for real would take the full sixty seconds; moving
/// `created_at` into the past is the test controlling *the charge's clock*,
/// exactly as `age_the_dead_workers_lease` above controls the queue's, and
/// as `worker_recovery.rs::age_the_crash` does for the same column.
///
/// What is *not* simulated: the charge was really left `submitting` by a
/// really killed process, and the write below is guarded on that state, so
/// it cannot age a charge a confirm has already moved on.
async fn age_the_crashed_charge(pool: &PgPool, charge_id: &str) -> anyhow::Result<()> {
    let aged = sqlx::query(
        "UPDATE charges SET created_at = created_at - INTERVAL '10 minutes' \
         WHERE id = $1 AND state = 'submitting'",
    )
    .bind(charge_id)
    .execute(pool)
    .await
    .context("ageing the crashed server's charge")?
    .rows_affected();
    anyhow::ensure!(
        aged == 1,
        "expected exactly one submitting charge {charge_id}, aged {aged}"
    );
    Ok(())
}

// -------------------------------------------------------------- the spawns

/// The shipping worker, configured the way `compose.e2e.yml` configures it.
///
/// **`vpay-server` plus the literal argument `worker`** — the same two words
/// `compose.e2e.yml`'s `command:` and `deployment-worker.yaml`'s `args:`
/// carry since issue #77 (2026-09-07). It was `Command::new(shipping_binary(
/// "vpay-worker-bin"))` with no argument until then, and this is the seam
/// that makes the two `kill -9` scenarios below a test of the *subcommand*
/// and not only of `vpay_worker::run_loop`: `worker_recovery.rs` drives
/// `run_once`/`run_loop` in-process, so it would stay green against a
/// `worker` subcommand that did nothing at all, while `boot_worker`'s wait
/// for `job loop running` here would not.
///
/// `--worker-concurrency 1` so the process has exactly one job in flight and
/// "the poll was in flight when it died" is unambiguous.
/// `--observability-bind 127.0.0.1:0` because two workers run in this file
/// and a fixed port would collide (a `:0` port is a real configuration —
/// `vpay-server/tests/cli.rs`'s `worker` module uses it too).
fn spawn_worker(name: &'static str, database_url: &str, config: &Path) -> Proc {
    let mut cmd = Command::new(shipping_binary("vpay-server"));
    cmd.arg("worker")
        .env("DATABASE_URL", database_url)
        .env("VPAY_CONFIG", config)
        .env("VPAY_PROFILE", "kill9")
        .env("VPAY_LOG_FORMAT", "text")
        .env("RUST_LOG", "info")
        .env("VPAY_WORKER_CONCURRENCY", "1")
        .env("VPAY_OBSERVABILITY_BIND", "127.0.0.1:0")
        .env(
            "VPAY_SHUTDOWN_GRACE_SECONDS",
            SHUTDOWN_GRACE_SECONDS.to_string(),
        );
    Proc::spawn(name, cmd)
}

/// Boots a worker and returns it once its loop is running, or fails naming
/// what the process actually said.
fn boot_worker(name: &'static str, database_url: &str, config: &Path) -> Proc {
    let mut worker = spawn_worker(name, database_url, config);
    if worker
        .wait_for_line("job loop running", BOOT_TIMEOUT)
        .is_none()
    {
        panic!(
            "{name} never reached `job loop running` within {BOOT_TIMEOUT:?}\n{}",
            worker.log()
        );
    }
    worker
}

/// Stops a worker with `SIGTERM` and asserts it drained cleanly.
///
/// Exit `0` is the whole assertion: `vpay-server worker` exits `1` when the
/// shutdown grace period elapses with jobs still in flight, so a `0` here is
/// the binary's own statement that it finished its work and let go of every
/// lease.
///
/// **This is teardown, and its assertion string says what it does and does
/// not prove: "a worker with nothing in flight".** Issue #85 was about
/// precisely that gap, and it is
/// [`a_worker_sigtermed_mid_delivery_drains_it_and_the_merchant_is_told_exactly_once`]
/// that closes it — a worker signalled *with* work outstanding. This helper
/// is deliberately unchanged: a teardown that also asserted about in-flight
/// work would be asserting about whatever the previous case happened to
/// leave.
fn stop_worker_cleanly(mut worker: Proc) {
    worker.sigterm();
    let Some(exit) = worker.wait_within(Duration::from_secs(30)) else {
        panic!(
            "the worker did not exit within 30s of SIGTERM\n{}",
            worker.log()
        );
    };
    assert_eq!(
        exit.code(),
        Some(0),
        "a worker with nothing in flight must drain cleanly and exit 0; got {exit:?}\n{}",
        worker.log()
    );
}

/// Waits for a charge to reach `succeeded`, or says what the queue and the
/// worker looked like when it gave up.
async fn wait_for_settlement(
    pool: &PgPool,
    intent_id: &str,
    worker: &mut Proc,
    within: Duration,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + within;
    loop {
        if charge_for(pool, intent_id).await?.state == "succeeded" {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let queued: Vec<(String, i32, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT dedupe_key, attempts, locked_by, last_error FROM jobs ORDER BY dedupe_key",
            )
            .fetch_all(pool)
            .await
            .unwrap_or_default();
            anyhow::bail!(
                "the charge never settled within {within:?}; jobs: {queued:?}\n{}",
                worker.log()
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ---------------------------------------------------------------- the cases

/// A worker `SIGKILL`ed while its status query is in flight loses nothing:
/// the next worker reaps its lease, re-runs the poll, and the charge settles
/// exactly once.
///
/// This is the case `docs/flows/crash-safety.md` says nothing proves. The
/// sequence, and what each step is evidence of:
///
/// 1. a merchant confirms through the real API, with the MSISDN that arms the
///    slow-status scenario — an ordinary confirm in every other respect;
/// 2. the shipping `vpay-worker-bin` starts, claims the poll job and issues
///    the status query. Two independent witnesses say it is in flight: the
///    rail's journal has the request, and `provider_requests` has the row the
///    worker wrote *before* the network call and cannot complete;
/// 3. `SIGKILL`. The exit status is asserted to be signal 9 — no destructor
///    ran, no lease was handed back, no drain happened;
/// 4. **the lease and the unanswered attempt are the only trace**: the charge
///    has not moved, the intent has not moved, `amount_received` is zero and
///    no event exists. A crash must not settle anything;
/// 5. a second worker starts, reaps the lease its predecessor could not hand
///    back, and re-runs the poll;
/// 6. the charge settles **once**, by four independent counts, and the rail
///    saw exactly the requests the ladder implies — one submit, two status
///    queries (the killed one and the one that answered).
#[tokio::test]
async fn a_worker_killed_mid_poll_settles_the_charge_exactly_once_after_its_lease_is_reaped()
-> anyhow::Result<()> {
    let (_postgres, repositories, pool, database_url, _mtn, mtn_url) = containers().await?;
    let workspace = Workspace::new();

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();

    // The API runs in-process here — it is not the process being killed —
    // but the configuration it boots from is the very same value the worker
    // subprocess reads out of a file, so the two cannot disagree about the
    // rail.
    let mut captured: Option<Config> = None;
    let served = support::serve(&repositories, &server_pem, |base_url| {
        let config = config_with(base_url, &mtn_url, jwks_a);
        captured = Some(config.clone());
        config
    })
    .await?;
    let config_path = workspace.write_config(&captured.expect("the harness built a configuration"));

    let client = sdk_client(&served.base_url, &pem_a);
    let created = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .map_err(|e| anyhow::anyhow!("creating the intent: {e}"))?;
    let confirmed = client
        .payment_intents()
        .confirm(
            &created.id,
            ConfirmPaymentIntentParams::mtn_momo(SLOW_STATUS_MSISDN),
            RequestOptions::new(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("confirming the intent: {e}"))?;
    assert_eq!(
        confirmed.status,
        IntentStatus::Processing,
        "the confirm itself must be ordinary; the crash under test is the worker's"
    );

    let charge = charge_for(&pool, &created.id).await?;
    assert_eq!(charge.state, "submitted");

    // ---- the worker that will die -------------------------------------
    let mut victim = boot_worker("worker-1 (to be killed)", &database_url, &config_path);
    let victim_pid = victim.pid();

    // In flight, on two independent records. Neither is this test's own
    // bookkeeping: one is the rail's journal, one is the row the worker
    // committed before it opened the socket.
    let deadline = Instant::now() + IN_FLIGHT_TIMEOUT;
    let dead_worker = loop {
        let queried = journal_count(&mtn_url, status_queries()).await?;
        let job = poll_job(&pool, &charge.id).await?;
        let unanswered =
            attempts(&pool, &charge.id)
                .await?
                .iter()
                .any(|(operation, status, error)| {
                    operation == "query_status" && status.is_none() && error.is_none()
                });
        if queried >= 1
            && unanswered
            && let Some(StoredJob {
                locked_by: Some(owner),
                ..
            }) = job
        {
            break owner;
        }
        assert!(
            Instant::now() < deadline,
            "the worker's status query was not in flight within {IN_FLIGHT_TIMEOUT:?} \
             (rail saw {queried} status queries, attempt row committed: {unanswered}); \
             killing it now would prove nothing\n{}",
            victim.log()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(
        dead_worker.contains(&format!("/{victim_pid}/")),
        "the lease must belong to the process this test is about to kill \
         (worker_id is host/pid/random — vpay_worker::worker_id): {dead_worker} has no \
         /{victim_pid}/"
    );

    // ---- the signal ---------------------------------------------------
    let exit = victim.kill9();
    assert_eq!(
        exit.signal(),
        Some(9),
        "the process must have been *killed*, not have exited on its own: {exit:?}"
    );
    assert_eq!(
        exit.code(),
        None,
        "a signalled process has no exit code; one here would mean something caught SIGKILL"
    );

    // ---- the lease is the only trace ----------------------------------
    let job = poll_job(&pool, &charge.id)
        .await?
        .context("the poll job vanished when its worker died")?;
    assert_eq!(
        job.locked_by.as_deref(),
        Some(dead_worker.as_str()),
        "the killed worker's lease must still be held by it; nothing else can hand it back"
    );
    assert_eq!(
        job.attempts, 1,
        "the claim increments attempts before the work, so a job that kills its worker \
         still counts up (vpay_db::jobs::claim)"
    );

    let after_kill = charge_for(&pool, &created.id).await?;
    assert_eq!(
        after_kill.state, "submitted",
        "a crash mid-poll must move no charge state"
    );
    let (status, received) = stored_intent(&pool, &created.id).await?;
    assert_eq!(status, "processing", "and no intent state");
    assert_eq!(received, 0, "and no money");
    assert!(
        event_types(&pool, &created.id).await?.is_empty(),
        "and it must tell no merchant anything: an event here would be a webhook about a \
         settlement that never happened"
    );
    assert_eq!(
        attempts(&pool, &charge.id).await?,
        vec![
            ("submit".to_owned(), Some(ANSWERED_SENTINEL), None),
            ("query_status".to_owned(), None, None),
        ],
        "the only trace of the killed poll is the attempt row it wrote before the network \
         call — write first, network second, even for a read"
    );

    // ---- the recovery -------------------------------------------------
    age_the_dead_workers_lease(&pool, &charge.id, &dead_worker).await?;

    let mut rescuer = boot_worker("worker-2 (the survivor)", &database_url, &config_path);
    assert!(
        rescuer
            .wait_for_line(
                "freed job leases whose worker never came back",
                BOOT_TIMEOUT
            )
            .is_some(),
        "the second worker must reap the dead one's lease at boot — `claim` matches only \
         `locked_at IS NULL`, so nothing else would ever pick this charge up\n{}",
        rescuer.log()
    );

    wait_for_settlement(&pool, &created.id, &mut rescuer, SETTLE_TIMEOUT).await?;

    // ---- exactly once, on four independent records --------------------
    assert_eq!(
        charge_count(&pool, &created.id).await?,
        1,
        "one charge per intent, forever (AGENTS.md); a recovery that opened a second one \
         is a second payment"
    );
    let settled = charge_for(&pool, &created.id).await?;
    assert_eq!(settled.state, "succeeded");
    assert_eq!(
        settled.provider_txn_id.as_deref(),
        Some(KILL9_TXN_ID),
        "the rail's own identifier for the money movement, and it must be the one the \
         kill9 scenario returned rather than another stub's"
    );
    let (status, received) = stored_intent(&pool, &created.id).await?;
    assert_eq!(status, "succeeded");
    assert_eq!(
        received, AMOUNT,
        "amount_received must be the amount, once — a settlement applied twice would \
         double it"
    );
    assert_eq!(
        event_types(&pool, &created.id).await?,
        vec!["payment_intent.succeeded".to_owned()],
        "exactly one event, so exactly one webhook: a merchant must not hear about this \
         payment twice because a worker died"
    );
    assert!(
        poll_job(&pool, &charge.id).await?.is_none(),
        "a settled charge's poll job must be deleted, not left to run forever"
    );

    // ---- and the rail saw what the ladder implies ---------------------
    assert_eq!(
        journal_count(&mtn_url, submits()).await?,
        1,
        "the payer's handset must have buzzed once. A crash during a *poll* is never \
         grounds to submit again (docs/flows/crash-safety.md's retry rule)"
    );
    assert_eq!(
        journal_count(&mtn_url, status_queries()).await?,
        2,
        "two status queries and no more: the one that was killed, and the one that \
         answered. A third would mean the ladder ran a rung it did not need"
    );
    assert_one_reference(&pool, &charge.id, charge.provider_reference_id).await;

    stop_worker_cleanly(rescuer);
    served.server.abort();
    Ok(())
}

/// The server `SIGKILL`ed with a `requesttopay` in flight — kill point 2 of
/// `docs/flows/crash-safety.md` — leaves a charge the worker recovers,
/// without ever submitting a second time.
///
/// `worker_recovery.rs` proves the recovery *table* against this state by
/// writing it. This proves the state is what a killed process actually
/// leaves, which is the half that was missing: the ordering claim ("write
/// first, network second") is a claim about what survives a crash, and until
/// something crashed, nothing had checked.
///
/// 1. the shipping `vpay-server` is spawned and a merchant confirms through
///    it with the MSISDN whose submit takes 30 s;
/// 2. once the rail has the POST **and** the charge is committed in
///    `submitting` with an unanswered attempt row, the server is `SIGKILL`ed
///    mid-request. The payer's handset is buzzing and no answer will ever
///    arrive — the exact situation the ordering exists for;
/// 3. what survives is asserted directly: the charge, its reference, its
///    unanswered attempt row, and the poll job that was committed in the
///    same transaction. The intent is still `requires_payment_method`,
///    because the confirm never reached `persist_submitted`;
/// 4. a worker recovers it: `SubmitAttempt::Unanswered` → **poll**, never
///    resubmit, and the charge settles once. The rail's journal shows one
///    submit, which is the assertion the retry rule is really about.
#[tokio::test]
async fn a_server_killed_mid_submit_leaves_a_charge_the_worker_settles_without_a_second_submit()
-> anyhow::Result<()> {
    let (_postgres, _repositories, pool, database_url, _mtn, mtn_url) = containers().await?;
    let workspace = Workspace::new();

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();
    let key_path = workspace.write_signing_key(&server_pem);

    // The port has to be known before the configuration is written:
    // `deployment.public_base_url` is what the merchant OP derives its
    // issuer from, so a placeholder would make the server mint tokens its
    // own validator rejects.
    let addr = free_addr();
    let base_url = format!("http://{addr}");
    let config = config_with(&base_url, &mtn_url, jwks_a);
    let config_path = workspace.write_config(&config);

    let mut cmd = Command::new(shipping_binary("vpay-server"));
    cmd.env("VPAY_BIND", addr.to_string())
        .env("DATABASE_URL", &database_url)
        .env("VPAY_CONFIG", &config_path)
        .env("VPAY_OAUTH_SIGNING_KEY_FILE", &key_path)
        .env("VPAY_PROFILE", "kill9")
        .env("VPAY_LOG_FORMAT", "text")
        .env("RUST_LOG", "info")
        .env("VPAY_OBSERVABILITY_BIND", "127.0.0.1:0");
    let mut server = Proc::spawn("vpay-server (to be killed)", cmd);

    // Ready when it answers, not when it says it is: `/healthz` over a real
    // socket is what a merchant's first request meets.
    let http = reqwest::Client::new();
    let deadline = Instant::now() + BOOT_TIMEOUT;
    loop {
        if let Ok(response) = http.get(format!("{base_url}/healthz")).send().await
            && response.status().is_success()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the server never served /healthz within {BOOT_TIMEOUT:?}\n{}",
            server.log()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let client = sdk_client(&base_url, &pem_a);
    let created = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .map_err(|e| anyhow::anyhow!("creating the intent: {e}"))?;

    // The confirm never returns: the rail holds its answer for 30 s and the
    // server is killed long before that. The handle is dropped at the end of
    // the test with the failed request still in it, which is exactly what a
    // merchant would experience.
    let confirming = {
        let base_url = base_url.clone();
        let pem = pem_a.clone();
        let intent_id = created.id.clone();
        tokio::spawn(async move {
            sdk_client(&base_url, &pem)
                .payment_intents()
                .confirm(
                    &intent_id,
                    ConfirmPaymentIntentParams::mtn_momo(SLOW_SUBMIT_MSISDN),
                    RequestOptions::new(),
                )
                .await
                .map(|intent| intent.status)
        })
    };

    // Kill point 2, exactly: the rail has the POST and vpay has the row that
    // says "issued, no answer". Waiting for both is what makes this the
    // *second* row of the recovery table and not the first or the third.
    let deadline = Instant::now() + IN_FLIGHT_TIMEOUT;
    let charge = loop {
        let submitted = journal_count(&mtn_url, submits()).await?;
        if submitted >= 1
            && let Ok(charge) = charge_for(&pool, &created.id).await
            && charge.state == "submitting"
            && attempts(&pool, &charge.id).await? == vec![("submit".to_owned(), None, None)]
        {
            break charge;
        }
        assert!(
            Instant::now() < deadline,
            "the submit was not in flight within {IN_FLIGHT_TIMEOUT:?} (rail saw \
             {submitted} submits); killing the server now would prove nothing\n{}",
            server.log()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let exit = server.kill9();
    assert_eq!(
        exit.signal(),
        Some(9),
        "the server must have been killed mid-request, not have exited: {exit:?}"
    );
    confirming.abort();

    // ---- what a killed confirm leaves --------------------------------
    let after_kill = charge_for(&pool, &created.id).await?;
    assert_eq!(
        after_kill.state, "submitting",
        "the charge and its reference were committed before the POST; that commit is the \
         whole of crash-safety.md's push-rail ordering"
    );
    assert_eq!(
        after_kill.provider_reference_id, charge.provider_reference_id,
        "and the reference survived unchanged — it is the only name this payment has"
    );
    assert_eq!(
        attempts(&pool, &charge.id).await?,
        vec![("submit".to_owned(), None, None)],
        "one attempt, issued and unanswered: migration 0016's `status_code IS NULL` is \
         what tells the recovery table this is row 2 and not row 1"
    );
    let job = poll_job(&pool, &charge.id)
        .await?
        .context("a crashed confirm left no poll job; nothing would ever drive this charge")?;
    assert_eq!(
        job.locked_by, None,
        "the job was committed with the charge and never claimed — the server does not \
         run jobs"
    );
    let (status, received) = stored_intent(&pool, &created.id).await?;
    assert_eq!(
        status, "requires_payment_method",
        "the confirm never reached persist_submitted, so the intent is where a crashed \
         confirm leaves it — the state SETTLEABLE_STATUSES exists to accept"
    );
    assert_eq!(received, 0);
    assert!(event_types(&pool, &created.id).await?.is_empty());

    // ---- the recovery -------------------------------------------------
    age_the_crashed_charge(&pool, &charge.id).await?;
    let mut worker = boot_worker("worker (the recovery)", &database_url, &config_path);
    wait_for_settlement(&pool, &created.id, &mut worker, SETTLE_TIMEOUT).await?;

    assert_eq!(charge_count(&pool, &created.id).await?, 1);
    let settled = charge_for(&pool, &created.id).await?;
    assert_eq!(settled.state, "succeeded");
    assert_eq!(
        settled.provider_txn_id.as_deref(),
        Some(CATCH_ALL_TXN_ID),
        "this charge's reference has no mapping of its own, so the rail's catch-all \
         SUCCESSFUL is what settled it"
    );
    let (status, received) = stored_intent(&pool, &created.id).await?;
    assert_eq!(
        status, "succeeded",
        "a settlement must land on the intent a crashed confirm left behind"
    );
    assert_eq!(received, AMOUNT);
    assert_eq!(
        event_types(&pool, &created.id).await?,
        vec!["payment_intent.succeeded".to_owned()]
    );

    assert_eq!(
        journal_count(&mtn_url, submits()).await?,
        1,
        "**the double-charge assertion.** An unanswered submit is resolved by asking the \
         rail, never by asking it again: a second POST here is a second prompt on the \
         payer's handset for the same money"
    );
    assert_one_reference(&pool, &charge.id, charge.provider_reference_id).await;

    stop_worker_cleanly(worker);
    Ok(())
}

/// A worker `SIGTERM`ed with a webhook delivery in flight **finishes the
/// delivery**, exits `0`, and the merchant's receiver hears about the payment
/// exactly once.
///
/// This is the case `docs/flows/crash-safety.md` recorded as a hand
/// measurement and said nothing re-ran (issue #85). It is the same question
/// the two `SIGKILL` cases above ask, of the other signal: they prove what a
/// process that *cannot* clean up leaves behind, and this proves that the
/// cleanup a process **is** given the chance to do actually happens.
///
/// The sequence, and what each step is evidence of:
///
/// 1. an ordinary confirm on [`SIGTERM_MSISDN`], which arms no rail mapping:
///    the rail answers a plain 202 and then `SUCCESSFUL`, because the thing
///    made slow here is the *merchant's receiver*, not the rail;
/// 2. **two** shipping workers run for the whole scenario. One of them
///    settles the charge, fans the event out and claims the
///    `deliver_webhook` job; which one is not decided by the test but read
///    off `jobs.locked_by`, whose `worker_id` carries the pid;
/// 3. the POST is in the receiver's journal and the job is leased. Both
///    witnesses are outside the process holding it, which is what makes "the
///    delivery is in flight" a fact rather than an inference. `slow-ack.json`
///    holds the acknowledgement for [`RECEIVER_ACK_DELAY`], and the two
///    `const` assertions at the top of this file pin that interval strictly
///    between the delivery client's own deadline and the drain's budget;
/// 4. `SIGTERM`, to the claimant. Its transcript is then read back and the
///    run is **rejected** unless [`DRAIN_BEGAN`] precedes
///    [`WEBHOOK_DELIVERED`] in it: a signal that arrived after the receiver
///    had already answered would have proved nothing, and this case says so
///    rather than passing;
/// 5. exit `0`, with no signal — SIGTERM is caught, so a *signalled* death
///    would be the regression — and no `Drain::TimedOut` warning;
/// 6. exactly one delivery row, `succeeded`, `attempt = 0`; its job deleted;
///    and **exactly one POST at the receiver**, which verifies under the
///    configured secret with the Rust SDK a merchant installs. That count is
///    what the drain buys: a worker that exited on the signal instead of
///    draining would leave the receiver's copy accepted and the row still
///    `pending`, and the survivor would send it again;
/// 7. the same four-record exactly-once invariant as the two cases above;
/// 8. and the survivor — a worker that was running, unsignalled, throughout —
///    is watched past the first rung of the delivery ladder
///    (`vpay_worker::delivery_delay(0)`) and must send nothing. It could only
///    have claimed that job by ignoring the lease the draining worker still
///    held, which is what `Jobs::claim`'s `locked_at IS NULL` and
///    `FOR UPDATE SKIP LOCKED` exist for.
///
/// # Why the second worker co-runs rather than restarting afterwards
///
/// The hand measurement restarted the container. A restart cannot be used to
/// prove step 8: by the time a freshly-spawned process has connected,
/// migrated and reconciled, the drain it was meant to overlap has finished
/// and the job it was meant to be unable to steal no longer exists — so the
/// lease would be respected *vacuously*, on some runs and not others, with
/// nothing in the test able to tell which. A worker that is already running
/// when the signal lands overlaps by construction. What is lost is the
/// restart itself, and nothing else: `boot_worker` above is the same boot in
/// either case.
#[tokio::test]
async fn a_worker_sigtermed_mid_delivery_drains_it_and_the_merchant_is_told_exactly_once()
-> anyhow::Result<()> {
    let (_postgres, repositories, pool, database_url, _mtn, mtn_url) = containers().await?;
    let receiver = vpay_testkit::containers::start_wiremock(&receiver_mappings_dir())
        .await
        .context("the merchant's webhook receiver container starts")?;
    let receiver_url = format!(
        "http://127.0.0.1:{}",
        receiver
            .get_host_port_ipv4(8080)
            .await
            .context("the receiver's mapped port")?
    );
    let endpoint_url = format!("{receiver_url}{RECEIVER_PATH}");
    let workspace = Workspace::new();

    let (server_pem, _server_jwks) = generate_key();
    let (pem_a, jwks_a) = generate_key();

    // As in the mid-poll case: the API is in-process because it is not the
    // process being signalled, but the configuration it boots from is the
    // very same value both worker subprocesses read out of a file.
    let mut captured: Option<Config> = None;
    let served = support::serve(&repositories, &server_pem, |base_url| {
        let config = config_with_receiver(base_url, &mtn_url, jwks_a, &endpoint_url);
        captured = Some(config.clone());
        config
    })
    .await?;
    let config_path = workspace.write_config(&captured.expect("the harness built a configuration"));

    // ---- two workers, both shipping, both running ---------------------
    let mut first = boot_worker("worker-1", &database_url, &config_path);
    let mut second = boot_worker("worker-2", &database_url, &config_path);

    let client = sdk_client(&served.base_url, &pem_a);
    let created = client
        .payment_intents()
        .create(create_params(), RequestOptions::new())
        .await
        .map_err(|e| anyhow::anyhow!("creating the intent: {e}"))?;
    let confirmed = client
        .payment_intents()
        .confirm(
            &created.id,
            ConfirmPaymentIntentParams::mtn_momo(SIGTERM_MSISDN),
            RequestOptions::new(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("confirming the intent: {e}"))?;
    assert_eq!(
        confirmed.status,
        IntentStatus::Processing,
        "the confirm must be ordinary; the signal under test lands much later"
    );
    let charge = charge_for(&pool, &created.id).await?;
    assert_eq!(charge.state, "submitted");

    // ---- the delivery, in flight, on two independent records ----------
    //
    // Neither is this test's own bookkeeping: one is the receiver's journal
    // (WireMock records a request when it *matches* it, before serving any
    // delay), one is the lease the claiming worker took in the queue.
    let deadline = Instant::now() + DELIVERY_IN_FLIGHT_TIMEOUT;
    let (delivery, owner) = loop {
        let posts = receiver_posts(&receiver_url).await?.len();
        let rows = deliveries(&pool).await?;
        let leased = match rows.as_slice() {
            [delivery] if delivery.state == "pending" => delivery_job(&pool, delivery.id)
                .await?
                .and_then(|job| job.locked_by)
                .map(|owner| (delivery.clone(), owner)),
            _ => None,
        };
        if posts >= 1
            && let Some(found) = leased
        {
            break found;
        }
        assert!(
            Instant::now() < deadline,
            "no webhook delivery was in flight within {DELIVERY_IN_FLIGHT_TIMEOUT:?} (the \
             receiver saw {posts} POSTs on {RECEIVER_PATH}; deliveries: {rows:?}); signalling \
             a worker now would prove nothing\n{}\n{}",
            first.log(),
            second.log()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(delivery.endpoint_id, RECEIVER_ENDPOINT_ID);
    assert_eq!(
        delivery.url, endpoint_url,
        "the URL is denormalised onto the row when the delivery is created \
         (migration 0022), and it must be the slow path this case steers with"
    );

    // Which of the two is holding it decides which one is signalled — read
    // from the queue, never assumed. `worker_id` is host/pid/random
    // (`vpay_worker::worker_id`).
    let first_pid = first.pid();
    let second_pid = second.pid();
    let (mut victim, mut survivor) = if owner.contains(&format!("/{first_pid}/")) {
        (first, second)
    } else if owner.contains(&format!("/{second_pid}/")) {
        (second, first)
    } else {
        panic!(
            "the delivery's lease is held by `{owner}`, which is neither worker this test \
             started (pids {first_pid} and {second_pid})\n{}\n{}",
            first.log(),
            second.log()
        )
    };

    // ---- the signal ---------------------------------------------------
    victim.sigterm();
    let Some(exit) = victim.wait_within(SIGTERM_EXIT_TIMEOUT) else {
        panic!(
            "the signalled worker was still alive {SIGTERM_EXIT_TIMEOUT:?} after SIGTERM; \
             that is neither a clean drain nor a timed-out one\n{}",
            victim.log()
        )
    };
    victim.absorb_until_closed(LOG_FLUSH_TIMEOUT);
    let transcript = victim.log();

    assert_eq!(
        exit.code(),
        Some(0),
        "a worker that finished its in-flight job inside the grace period exits 0; \
         `vpay-server worker` exits 1 on `Drain::TimedOut`, and that is a different \
         outcome from this one. Got {exit:?}\n{transcript}"
    );
    assert_eq!(
        exit.signal(),
        None,
        "SIGTERM is *caught* here — `vpay_config::signal` installs a handler before \
         anything else runs — so a process that died OF the signal did not drain at all: \
         {exit:?}\n{transcript}"
    );

    // ---- and the signal really did land mid-delivery -------------------
    //
    // The one assertion that keeps this case honest on a slow machine. Every
    // line below is the *victim's own*, in the order it wrote them.
    let heard = victim
        .said(SIGTERM_HEARD)
        .unwrap_or_else(|| panic!("the worker never logged `{SIGTERM_HEARD}`\n{transcript}"));
    let draining = victim
        .said(DRAIN_BEGAN)
        .unwrap_or_else(|| panic!("the worker never logged `{DRAIN_BEGAN}`\n{transcript}"));
    let delivered = victim
        .said(WEBHOOK_DELIVERED)
        .unwrap_or_else(|| panic!("the worker never logged `{WEBHOOK_DELIVERED}`\n{transcript}"));
    let complete = victim
        .said(SHUTDOWN_COMPLETE)
        .unwrap_or_else(|| panic!("the worker never logged `{SHUTDOWN_COMPLETE}`\n{transcript}"));
    assert!(
        heard < draining && draining < delivered && delivered < complete,
        "the webhook must be delivered BY THE DRAIN, i.e. strictly between `{DRAIN_BEGAN}` \
         and `{SHUTDOWN_COMPLETE}`. A `{WEBHOOK_DELIVERED}` before `{DRAIN_BEGAN}` means the \
         receiver answered before the signal arrived and this run proves nothing about the \
         drain — raise RECEIVER_ACK_DELAY (it must stay under \
         vpay_worker::webhooks::WEBHOOK_REQUEST_TIMEOUT) rather than relaxing this. \
         Indices: heard={heard} draining={draining} delivered={delivered} \
         complete={complete}\n{transcript}"
    );
    assert!(
        victim.said(DRAIN_TIMED_OUT).is_none(),
        "the drain must not have timed out; `Drain::TimedOut` hands leases back and is the \
         branch this case is distinguishing itself from\n{transcript}"
    );

    // ---- exactly one POST, and it is signed ---------------------------
    let event_id = event_id_for(&pool, &created.id).await?;
    let posts = receiver_posts(&receiver_url).await?;
    assert_eq!(
        posts.len(),
        1,
        "**the double-send assertion.** The receiver accepted the in-flight POST before the \
         signal arrived; if the drain had not finished it, the row would still be `pending` \
         and a worker would send those same bytes again. A merchant must not be told twice \
         about one payment because an operator restarted a worker: {posts:?}"
    );
    let post = posts.first().expect("exactly one POST, just asserted");
    assert_eq!(post.url, RECEIVER_PATH);
    assert_eq!(
        post.header("vpay-event-id"),
        Some(event_id.as_str()),
        "the convenience header carries the event id a merchant dedupes on"
    );
    let signature = post
        .header("vpay-signature")
        .unwrap_or_else(|| panic!("the delivery carried no Vpay-Signature: {post:?}"));
    let verified =
        vpay_sdk::webhooks::verify(&post.body, signature, RECEIVER_SECRET, SIGNATURE_TOLERANCE)
            .expect(
                "the bytes the receiver got must verify under the configured secret with the \
                 Rust SDK a merchant installs — 'signed' is not 'a header was present'",
            );
    assert_eq!(verified.id, event_id);
    assert_eq!(verified.kind, "payment_intent.succeeded");

    // ---- what the drain finished --------------------------------------
    let rows = deliveries(&pool).await?;
    assert_eq!(
        rows.len(),
        1,
        "one event, one endpoint, one delivery row — a second is a second webhook: {rows:?}"
    );
    let settled_delivery = rows.first().expect("exactly one delivery, just asserted");
    assert_eq!(
        settled_delivery.state, "succeeded",
        "the drain must have recorded the receiver's 2xx; a `pending` row here is work the \
         drain abandoned"
    );
    assert_eq!(settled_delivery.status_code, Some(200));
    assert_eq!(
        settled_delivery.attempt, 0,
        "`attempt` counts FAILED attempts, so a first-attempt success leaves it 0. A 1 here \
         means the send was cut off and walked a rung of the ladder"
    );
    assert!(
        delivery_job(&pool, delivery.id).await?.is_none(),
        "a succeeded delivery's job must be deleted, not left leased by a process that no \
         longer exists"
    );

    // ---- exactly once, on the same four records as the cases above ----
    assert_eq!(
        charge_count(&pool, &created.id).await?,
        1,
        "one charge per intent, forever (AGENTS.md)"
    );
    let settled = charge_for(&pool, &created.id).await?;
    assert_eq!(settled.state, "succeeded");
    assert_eq!(
        settled.provider_txn_id.as_deref(),
        Some(CATCH_ALL_TXN_ID),
        "this charge's MSISDN arms no mapping, so the rail's catch-all SUCCESSFUL is what \
         settled it — which is the point of choosing an MSISDN that arms nothing"
    );
    let (status, received) = stored_intent(&pool, &created.id).await?;
    assert_eq!(status, "succeeded");
    assert_eq!(
        received, AMOUNT,
        "amount_received must be the amount, once — a settlement applied twice would double \
         it"
    );
    assert_eq!(
        event_types(&pool, &created.id).await?,
        vec!["payment_intent.succeeded".to_owned()],
        "exactly one event, so exactly one webhook owed"
    );
    assert!(
        poll_job(&pool, &charge.id).await?.is_none(),
        "a settled charge's poll job must be deleted"
    );
    assert_eq!(
        journal_count(&mtn_url, submits()).await?,
        1,
        "the payer's handset must have buzzed once"
    );
    assert_eq!(
        journal_count(&mtn_url, status_queries()).await?,
        1,
        "one status query: the rail answers SUCCESSFUL on the first rung, so a second would \
         mean the ladder ran a rung it did not need"
    );
    assert_one_reference(&pool, &charge.id, charge.provider_reference_id).await;

    // ---- and the worker that was left running sends nothing -----------
    //
    // Watched past the first rung of the delivery ladder, so this is an
    // observation over the whole window in which a re-send would have
    // happened rather than a snapshot taken before one could.
    let past_the_first_rung = vpay_worker::delivery_delay(0)
        .context("the delivery ladder has a first rung")?
        + Duration::from_secs(5);
    tokio::time::sleep(past_the_first_rung).await;
    let posts = receiver_posts(&receiver_url).await?;
    assert_eq!(
        posts.len(),
        1,
        "still one POST {past_the_first_rung:?} later, i.e. past `delivery_delay(0)`. The \
         surviving worker could only have sent a second by claiming a job the draining \
         worker still held, which `Jobs::claim`'s `locked_at IS NULL` and `FOR UPDATE SKIP \
         LOCKED` are exactly what prevent: {posts:?}"
    );
    survivor.absorb();
    assert!(
        survivor.said(WEBHOOK_DELIVERED).is_none(),
        "the surviving worker must never have delivered this webhook; the drain did\n{}",
        survivor.log()
    );
    assert_eq!(deliveries(&pool).await?.len(), 1);

    stop_worker_cleanly(survivor);
    served.server.abort();
    Ok(())
}

/// [`SIGTERM_MSISDN`] must go on arming nothing in the shared rail tree.
///
/// The case above depends on the rail being *boring* for that number: an
/// ordinary 202 and the catch-all `SUCCESSFUL`. If a later mapping ever keys
/// on it — the way `requesttopay.json` keys a `PAYER_NOT_FOUND` on
/// `237600000400` — that case would stop settling and the failure would be
/// read as a drain bug. A file scan rather than a comment, because a comment
/// asking a future author to check something is not a check.
///
/// It needs no container and is in this file rather than in the conformance
/// suite because the *dependency* is this file's.
#[test]
fn the_sigterm_scenario_confirms_with_an_msisdn_that_arms_no_rail_mapping() {
    let dir = mappings_dir("mtn");
    let mut mentions = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the mtn mapping directory is readable") {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a mapping file is readable");
        // The `metadata.why` prose in these files names other MSISDNs at
        // length, so a bare `contains` over the whole file would be a match
        // on documentation. The parsed request bodies are what a stub
        // actually matches on.
        let document: Value = serde_json::from_str(&text).expect("a mapping file is JSON");
        let requests = document
            .get("mappings")
            .and_then(Value::as_array)
            .expect("a mapping file has a `mappings` array")
            .iter()
            .filter_map(|mapping| mapping.get("request"))
            .map(ToString::to_string)
            .collect::<String>();
        if requests.contains(SIGTERM_MSISDN) {
            mentions.push(path.display().to_string());
        }
    }
    assert!(
        mentions.is_empty(),
        "{SIGTERM_MSISDN} is matched on by {mentions:?}. \
         a_worker_sigtermed_mid_delivery_drains_it_and_the_merchant_is_told_exactly_once \
         needs the rail to be ordinary for it: pick a fresh documentation MSISDN for that \
         case, or point it at whatever the new mapping now returns"
    );
}
