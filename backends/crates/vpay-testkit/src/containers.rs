//! The one place a `postgres:16-alpine` testcontainer is started.
//!
//! Every suite in this workspace that needs a real Postgres used to build the
//! image request itself (`vpay-db`'s `tests/postgres.rs` and
//! `tests/repositories.rs`, all four of `backends/tests/integration/tests/*`,
//! and both binaries' `tests/cli.rs`) — eight byte-identical copies of
//! `PostgresImage::default().with_tag("16-alpine").start()`. They now all call
//! [`start_postgres_with_retry`], because the retry below is the kind of thing
//! that is only useful if *no* call site is missing it.
//!
//! # Why the tag is pinned
//!
//! `testcontainers-modules` 0.15 defaults to `postgres:11-alpine`, which is
//! not cached on the machines this suite runs on and cannot always be pulled.
//! `16-alpine` is cached and is also the correct choice regardless:
//! `compose.yml` runs Postgres 16, so testing against 11 would itself be a
//! version mismatch.
//!
//! # Why a retry, when `.config/nextest.toml` already serialises these
//!
//! That file bounds how many of *our* tests may start a container at once
//! (`postgres-containers = { max-threads = 1 }`), and that did not remove the
//! flake on a rootless-Docker host sharing a daemon with ~24 unrelated
//! containers. The remaining failure is not contention between our tests: it
//! is testcontainers asking for a random free host port, and rootlesskit's
//! port manager racing anything else on the host that grabbed that ephemeral
//! port in between:
//!
//! ```text
//! failed to start a container: Docker responded with status code 500:
//! failed to set up container networking: driver failed programming external
//! connectivity on endpoint …: error while calling RootlessKit
//! PortManager.AddPort(): listen tcp4 0.0.0.0:33298: bind: address already in use
//! ```
//!
//! Nothing inside this workspace can prevent that; a different random port on
//! the next attempt is the only cure. So the retry is deliberately narrow —
//! see [`start_postgres_with_retry`] for exactly which errors it will and
//! will not swallow.
//!
//! # What is deliberately unchanged
//!
//! The returned [`ContainerAsync`] is an ordinary local value owned by the
//! caller, and its `Drop` still stops and removes the container. A shared
//! `static` container was tried and rejected in this repo before (a
//! `ContainerAsync` in a `static` never runs `Drop` at process exit, which
//! leaked hundreds of live containers) — see `.config/nextest.toml`'s own
//! comment. This module does not reopen that decision: it starts one
//! container per call, exactly as the eight copies it replaced did.

use std::error::Error as _;
use std::future::Future;
use std::path::Path;
use std::time::Duration;

use testcontainers::core::{AccessMode, IntoContainerPort as _, Mount, WaitFor};
use testcontainers::runners::AsyncRunner as _;
use testcontainers::{ContainerAsync, GenericImage, ImageExt as _, TestcontainersError};
use testcontainers_modules::postgres::Postgres;

/// How many times a container start is attempted before the failure is the
/// caller's problem. Four rather than "until it works": a host that cannot
/// hand out a free ephemeral port four times in a row is broken in a way a
/// test suite should surface, not paper over.
const MAX_ATTEMPTS: u32 = 4;

/// Base backoff, multiplied by the attempt number (250 ms, 500 ms, 750 ms).
/// The collision is with whatever transiently owns that one port, so the wait
/// only has to outlive a short-lived socket, not a service restart.
const RETRY_BACKOFF: Duration = Duration::from_millis(250);

/// The only error text that earns a retry. Matching on a message is
/// unpleasant, but the Docker daemon reports this as a plain HTTP 500 with a
/// free-text body — `bollard` and `testcontainers` both surface it as an
/// opaque string, so there is no typed variant to match on instead.
const PORT_COLLISION: &str = "address already in use";

/// Starts a fresh, unmigrated `postgres:16-alpine` container, retrying only a
/// host-port collision.
///
/// Retries up to `MAX_ATTEMPTS` times, and **only** when the error chain
/// mentions `PORT_COLLISION`. Any other failure — daemon unreachable, image
/// missing, out of memory, wait strategy timed out — is returned immediately
/// and unwrapped, because a genuinely broken Docker daemon that is retried
/// four times still fails, just four times more slowly and with the original
/// cause four levels deep in a log. The error returned after the final
/// attempt is the [`TestcontainersError`] as testcontainers produced it, with
/// no wrapping of our own, so a caller's `.context(…)`/`.expect(…)` message
/// reads exactly as it did before this helper existed.
///
/// Each retry prints to stderr with the attempt number and the contended
/// port, so a host that is quietly flaky is visible rather than showing up
/// only as a suite that takes longer than it used to. Caveat worth knowing
/// before trusting a silent run: nextest captures a *passing* test's output
/// (`success-output` defaults to `never`), so these lines appear when the
/// start still fails after every attempt, under `--no-capture`, or under
/// plain `cargo test -- --nocapture` — not on a run that a retry rescued.
/// Making them always visible means setting `success-output` for the
/// container test group in `.config/nextest.toml`, which would also dump the
/// full output of every passing container test; that trade is left to a
/// maintainer rather than taken here.
///
/// The caller owns the returned container: dropping it stops and removes it.
/// Callers that need a *migrated* database keep doing that themselves — the
/// integration suite and `vpay-db` run migrations by different routes
/// (`sqlx::migrate!` against `backends/migrations` vs.
/// `vpay_db::run_migrations`), and folding either one in here would make this
/// helper pick a side.
///
/// # Errors
///
/// Returns the underlying [`TestcontainersError`] if the container could not
/// be started for any non-port-collision reason, or if every attempt lost a
/// port race.
pub async fn start_postgres_with_retry() -> Result<ContainerAsync<Postgres>, TestcontainersError> {
    retry_container_start("postgres:16-alpine", || {
        Postgres::default().with_tag("16-alpine").start()
    })
    .await
}

/// The WireMock image `compose.yml` runs, pinned.
///
/// 3.9.2 rather than `latest`: the conformance suite's mappings are written
/// against this version's request-matching and response-templating
/// behaviour, and a rail stub that silently changes what it matches is a
/// suite that silently stops testing what it says it tests. Same tag as
/// `compose.yml`, so a developer's `just up` and CI exercise one WireMock.
const WIREMOCK_IMAGE: (&str, &str) = ("wiremock/wiremock", "3.9.2");

/// The port WireMock listens on inside the container. The host port is
/// whatever Docker hands out — ask the returned container for it with
/// `get_host_port_ipv4(8080)`.
const WIREMOCK_PORT: u16 = 8080;

/// Where the image reads its `mappings/` and `__files/` from.
const WIREMOCK_ROOT: &str = "/home/wiremock";

/// Starts a WireMock container serving `mappings_dir`, retrying only a
/// host-port collision (the same policy [`start_postgres_with_retry`] uses).
///
/// # This is not a test double
///
/// It is the same `wiremock/wiremock` image `compose.yml` runs, reached over
/// HTTP exactly as a real rail is, configured by files on disk. That is what
/// ADR-0006 means by "a stub rail is a WireMock host in configuration": the
/// adapter under test builds a real request, a real server matches it, and a
/// real response comes back over a socket. The Rust `wiremock` crate — an
/// *in-process* double that would replace the adapter's transport — is the
/// thing this function exists to make unnecessary, and this crate
/// deliberately does not depend on it (see `Cargo.toml`).
///
/// # What `mappings_dir` must be
///
/// The WireMock *root* directory — the one that contains `mappings/`, e.g.
/// `backends/tests/conformance/wiremock/mtn`, not that directory's
/// `mappings` child. It is bind-mounted read-only at
/// `WIREMOCK_ROOT`: read-only because a container that could rewrite the
/// suite's own mappings is a test that can change its own expectations, and
/// WireMock only writes there when asked to record, which this never does.
///
/// A path that does not exist is *not* rejected here: Docker would create an
/// empty directory for it and WireMock would start with zero mappings,
/// answering 404 to everything. That failure is loud in the caller's
/// assertions and the alternative — a filesystem check in this helper —
/// would only move it. Callers build the path from `CARGO_MANIFEST_DIR`.
///
/// # Flags
///
/// `--global-response-templating` so a mapping may use `{{request.*}}`
/// helpers; `--verbose` so a
/// mismatched request prints WireMock's own diff into the container log,
/// which is the only way to debug "the stub returned 404" without attaching
/// to the container.
///
/// # Readiness
///
/// [`WaitFor::healthcheck`], which uses the image's own
/// `curl -f http://localhost:8080/__admin/health` (a 5 s start period, polled
/// every 100 ms). Not a log-line match: the banner this image prints is
/// decorative, colour-coded and version-dependent, and matching it would
/// break on an upgrade for no reason. Not a fixed sleep either — that is how
/// a suite becomes slow *and* flaky at once.
///
/// The caller owns the returned container: dropping it stops and removes it.
///
/// # Errors
///
/// The underlying [`TestcontainersError`] if the container could not be
/// started for any non-port-collision reason, or if every attempt lost a
/// port race.
pub async fn start_wiremock(
    mappings_dir: &Path,
) -> Result<ContainerAsync<GenericImage>, TestcontainersError> {
    let (image, tag) = WIREMOCK_IMAGE;
    let host_path = mappings_dir.display().to_string();

    retry_container_start(image, || {
        GenericImage::new(image, tag)
            .with_exposed_port(WIREMOCK_PORT.tcp())
            .with_wait_for(WaitFor::healthcheck())
            .with_cmd(["--global-response-templating", "--verbose"])
            .with_mount(
                Mount::bind_mount(host_path.clone(), WIREMOCK_ROOT)
                    .with_access_mode(AccessMode::ReadOnly),
            )
            .start()
    })
    .await
}

/// The retry policy itself, generic over what is being started.
///
/// Split out from [`start_postgres_with_retry`] so the policy can be proven
/// without a Docker daemon: the tests below drive it with a `start` closure
/// that fails on demand, which is the only way to observe the loop at all —
/// the real collision is a race on the host and cannot be summoned to order,
/// and three consecutive full-workspace runs on the host this was written for
/// produced zero retries. An untested retry is a retry that can silently be a
/// no-op or an infinite loop.
///
/// `label` names the image in the retry line only; nothing branches on it.
async fn retry_container_start<T, F, Fut>(
    label: &str,
    mut start: F,
) -> Result<T, TestcontainersError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, TestcontainersError>>,
{
    let mut attempt: u32 = 1;

    loop {
        let error = match start().await {
            Ok(started) => return Ok(started),
            Err(error) => error,
        };

        let text = error_chain_text(&error);
        if attempt >= MAX_ATTEMPTS || !text.contains(PORT_COLLISION) {
            return Err(error);
        }

        let backoff = RETRY_BACKOFF * attempt;
        eprintln!(
            "vpay-testkit: {label} start attempt {attempt}/{MAX_ATTEMPTS} lost host port {port} \
             ({PORT_COLLISION}); retrying in {backoff_ms} ms",
            port = collided_port(&text).unwrap_or("unknown"),
            backoff_ms = backoff.as_millis(),
        );
        tokio::time::sleep(backoff).await;
        attempt += 1;
    }
}

/// Flattens an error and every `source()` behind it into one string.
///
/// The interesting text is nested: `TestcontainersError::Client` is
/// `#[error(transparent)]` over `ClientError::StartContainer`, which formats
/// its `bollard` cause inline — so today the top-level `Display` alone would
/// contain the port-collision message. Walking the chain anyway means a
/// future testcontainers/bollard release that stops interpolating its cause
/// degrades into "no retry", which is the safe direction, rather than
/// silently matching nothing.
fn error_chain_text(error: &TestcontainersError) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        text.push_str(": ");
        text.push_str(&current.to_string());
        cause = current.source();
    }
    text
}

/// Digs the contended port out of the daemon's free-text error, for the log
/// line only — nothing branches on it, so an unparsable message costs a
/// less-informative `eprintln!` and nothing else.
///
/// Takes the last all-digit `:`-separated field before the collision phrase,
/// which picks `33298` out of `listen tcp4 0.0.0.0:33298: bind: address
/// already in use` without assuming the address family, the bind address, or
/// the `bind: ` prefix stays exactly as it is today.
fn collided_port(text: &str) -> Option<&str> {
    let (before, _) = text.split_once(PORT_COLLISION)?;
    before
        .rsplit(':')
        .map(str::trim)
        .find(|field| !field.is_empty() && field.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use testcontainers::TestcontainersError;

    use super::{
        MAX_ATTEMPTS, PORT_COLLISION, RETRY_BACKOFF, WIREMOCK_PORT, collided_port,
        retry_container_start, start_wiremock,
    };

    /// The MTN rail stub's directory, the same one `compose.yml` bind-mounts
    /// into `wiremock-mtn` and the same one the conformance suite starts.
    fn mtn_mappings_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/wiremock/mtn")
    }

    /// How many stub mappings the files in `dir/mappings` declare between
    /// them.
    ///
    /// Counted from disk rather than hard-coded: the rail's mapping set grows
    /// as its adapter is built, and a literal here would have to be bumped by
    /// whoever adds a mapping — which is exactly the kind of assertion people
    /// "fix" by editing the number.
    fn mappings_declared_on_disk(dir: &Path) -> usize {
        let mut total = 0;
        for entry in std::fs::read_dir(dir.join("mappings")).expect("the mappings directory exists")
        {
            let path = entry.expect("a readable directory entry").path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a readable mapping file");
            let document: serde_json::Value =
                serde_json::from_str(&text).expect("a mapping file is valid JSON");
            total += document
                .get("mappings")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
        }
        total
    }

    /// Docker-backed: the rail stub really starts, really reads the
    /// bind-mounted directory, and really serves what is in it.
    ///
    /// The assertion is deliberately *not* "the admin API answered": that
    /// would pass with an empty mount, which is the single most likely way
    /// this helper breaks (a wrong path, a directory Docker silently
    /// created). Comparing WireMock's loaded count against the count in the
    /// files on disk is what proves the mount arrived — and it stays true as
    /// the adapters' own mappings land, because both sides are derived.
    #[tokio::test]
    async fn a_wiremock_container_serves_the_mappings_it_was_pointed_at() {
        let dir = mtn_mappings_dir();
        let expected = mappings_declared_on_disk(&dir);
        assert!(
            expected > 0,
            "the MTN stub directory declares no mappings; this test would prove nothing"
        );

        let container = start_wiremock(&dir)
            .await
            .expect("the WireMock rail stub starts");
        let port = container
            .get_host_port_ipv4(WIREMOCK_PORT)
            .await
            .expect("the mapped host port");

        // The vendored-roots client, over plain HTTP to loopback — the same
        // constructor the binaries use, so this exercises no test-only
        // transport.
        let http = vpay_provider::http::client().expect("the vendored-roots client builds");
        let response = http
            .get(format!("http://127.0.0.1:{port}/__admin/mappings"))
            .send()
            .await
            .expect("the admin API answers");

        assert_eq!(response.status().as_u16(), 200);
        let body: serde_json::Value = response.json().await.expect("the admin API returns JSON");
        let loaded = body
            .get("mappings")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        assert_eq!(
            loaded, expected,
            "WireMock loaded {loaded} mappings, the bind-mounted directory declares {expected}: \
             the mount or the path is wrong"
        );
    }

    /// The exact message observed on the rootless-Docker host this retry was
    /// written for, verbatim. If a testcontainers upgrade reshapes it, this
    /// is the test that says so.
    const OBSERVED: &str = "failed to start a container: Docker responded with status code 500: \
                            failed to set up container networking: driver failed programming \
                            external connectivity on endpoint \
                            elastic_hopper_6b0e7c9e-3d2e-4f0a-9c1d-8a7b6c5d4e3f: error while \
                            calling RootlessKit PortManager.AddPort(): listen tcp4 \
                            0.0.0.0:33298: bind: address already in use";

    /// A daemon that is simply not there — the shape of failure that must
    /// never be retried, because retrying it only hides it for longer.
    const BROKEN_DAEMON: &str = "failed to start a container: error trying to connect: No such file or directory \
         (os error 2)";

    fn error(message: &str) -> TestcontainersError {
        TestcontainersError::Other(message.to_owned().into())
    }

    #[test]
    fn the_observed_message_is_recognised_and_yields_its_port() {
        assert!(
            OBSERVED.contains(PORT_COLLISION),
            "the retry predicate must match the message it exists for"
        );
        assert_eq!(collided_port(OBSERVED), Some("33298"));
    }

    #[test]
    fn an_unrelated_failure_is_neither_matched_nor_parsed() {
        assert!(
            !BROKEN_DAEMON.contains(PORT_COLLISION),
            "a daemon that is simply down must fail immediately, not be retried"
        );
        assert_eq!(collided_port(BROKEN_DAEMON), None);
    }

    /// The case the whole module exists for: two lost port races in a row,
    /// then a start that works. The caller must never see the collisions.
    #[tokio::test]
    async fn a_port_collision_is_retried_until_a_start_succeeds() {
        let attempts = Cell::new(0_u32);

        let outcome = retry_container_start("test-image", || {
            attempts.set(attempts.get() + 1);
            let attempt = attempts.get();
            async move {
                if attempt <= 2 {
                    Err(error(OBSERVED))
                } else {
                    Ok(attempt)
                }
            }
        })
        .await;

        assert_eq!(
            outcome.expect("a start that eventually succeeds must be reported as success"),
            3,
            "the third attempt's value must be the one returned"
        );
        assert_eq!(attempts.get(), 3, "exactly three starts must be attempted");
    }

    /// A host that loses the race every time must fail — and fail with the
    /// daemon's own error, not a wrapper of ours — after exactly
    /// [`MAX_ATTEMPTS`] tries, having waited the documented 250 ms × attempt
    /// between them. Both halves matter: no cap is an infinite hang, and no
    /// backoff is four attempts inside the same contended millisecond.
    #[tokio::test]
    async fn a_permanent_port_collision_gives_up_after_the_capped_attempts() {
        let attempts = Cell::new(0_u32);
        let started_at = Instant::now();

        let outcome: Result<(), _> = retry_container_start("test-image", || {
            attempts.set(attempts.get() + 1);
            async { Err(error(OBSERVED)) }
        })
        .await;

        let failure = outcome.expect_err("every attempt failed, so the call must fail");
        assert!(
            failure.to_string().contains(PORT_COLLISION),
            "the error returned must be the daemon's own, unwrapped: {failure}"
        );
        assert_eq!(
            attempts.get(),
            MAX_ATTEMPTS,
            "the retry must be capped, not unbounded"
        );

        // 250 + 500 + 750 ms between four attempts. Asserted as a lower bound
        // only: a loaded machine sleeps longer than it is asked to, never
        // shorter, so this cannot flake in the direction of a false pass.
        let expected = RETRY_BACKOFF * (1 + 2 + 3);
        assert!(
            started_at.elapsed() >= expected,
            "backoff must grow with the attempt number: waited {:?}, expected at least {expected:?}",
            started_at.elapsed()
        );
    }

    /// The guarantee that makes the retry safe to have at all: anything that
    /// is not a port collision fails on the first attempt, so a genuinely
    /// broken Docker daemon is never masked.
    #[tokio::test]
    async fn any_other_failure_is_returned_on_the_first_attempt() {
        let attempts = Cell::new(0_u32);

        let outcome: Result<(), _> = retry_container_start("test-image", || {
            attempts.set(attempts.get() + 1);
            async { Err(error(BROKEN_DAEMON)) }
        })
        .await;

        let failure = outcome.expect_err("the start failed, so the call must fail");
        assert!(
            failure.to_string().contains("No such file or directory"),
            "the original cause must survive untouched: {failure}"
        );
        assert_eq!(
            attempts.get(),
            1,
            "a non-collision failure must not be retried at all"
        );
    }
}
