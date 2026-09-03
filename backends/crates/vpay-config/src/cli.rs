//! Command-line configuration, shared by `vpay-server` and `vpay-worker-bin`.
//!
//! Every option auto-resolves from an environment variable (via clap's `env`
//! feature) so the same binary works identically whether it is invoked with
//! flags, with env vars (as in `compose.yml` / a container), or a mix — with
//! an explicit flag always winning over its environment variable.
//!
//! `CommonArgs` is `#[command(flatten)]`ed into both binaries' top-level
//! parser so the two cannot drift on the options they share.
//!
//! # `--profile` selects a file, never a code path
//!
//! `--profile` / `VPAY_PROFILE` is a *label*, used only to pick a YAML config
//! file (`docs/adr/0003-yaml-configuration.md`) and to stamp logs/traces. It
//! must **never** be matched on (`if profile == "production"`, `if
//! profile.is_live()`, …) to change behaviour anywhere in this workspace.
//! Sandbox and production are two deployments of the same image, distinguished
//! only by which config file they load — not by a code branch. See
//! `AGENTS.md` ("No environment branching").

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Structured logging output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogFormat {
    /// One JSON object per line — the default, for machine-parsed log
    /// pipelines.
    Json,
    /// Human-readable text — convenient for a local terminal.
    Text,
}

/// Options common to every vpay binary.
///
/// Flattened into each binary's top-level [`clap::Parser`] struct via
/// `#[command(flatten)]` so the server and worker cannot silently diverge on
/// a shared flag's name, env var, or default.
///
/// `Debug` is hand-written below instead of derived, because
/// `database_url` routinely embeds a plaintext password
/// (`postgres://user:password@host/db`). `ServerArgs`/`WorkerArgs` are far
/// more likely to reach a log line than the YAML config ever is — a startup
/// trace or an early `anyhow` error commonly prints the parsed CLI args
/// before anything else has happened — so this matters even more than
/// [`crate::config::ProviderHost`]'s equivalent redaction.
#[derive(Clone, Parser)]
pub struct CommonArgs {
    /// Postgres connection string.
    ///
    /// `Option<String>` at the clap level only: both `vpay-server` and
    /// `vpay-worker-bin` treat this as required at runtime (see each
    /// binary's own `main.rs`) — a missing value is a hard startup failure,
    /// not a silently DB-less scaffold mode. It stays optional here rather
    /// than `required = true` on the `clap` attribute so the CLI-parsing
    /// layer and the binary-level "what does this process actually need to
    /// run" decision stay separate concerns; `config` (below) follows the
    /// same shape for the identical reason.
    ///
    /// Routinely embeds a password (`postgres://user:pass@host/db`) — see
    /// [`CommonArgs`]'s hand-written `Debug` impl below, which prints only
    /// whether this is set, never its value.
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Deployment profile label.
    ///
    /// Selects which YAML config file to load — never a code path. See the
    /// module-level docs above and `docs/adr/0003-yaml-configuration.md`.
    #[arg(long, env = "VPAY_PROFILE", default_value = "sandbox")]
    pub profile: String,

    /// Path to the YAML configuration file (ADR-0003).
    ///
    /// `Option<PathBuf>` at the clap level only, for the same reason
    /// [`Self::database_url`] is — see that field's doc comment. Both
    /// `vpay-server` and `vpay-worker-bin` treat this as required at
    /// runtime: `vpay_config::Config::load` is called with it before either
    /// binary connects to the database or binds a listener, and a missing
    /// value is a hard, loud startup failure. A payment gateway that boots
    /// with no validated deployment configuration is exactly the
    /// half-configured process ADR-0003 says must never serve traffic.
    #[arg(long, env = "VPAY_CONFIG")]
    pub config: Option<PathBuf>,

    /// `tracing-subscriber` env-filter directive, e.g. `info` or
    /// `vpay_api=debug,info`.
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_filter: String,

    /// Structured logging output format.
    #[arg(long, env = "VPAY_LOG_FORMAT", value_enum, default_value = "json")]
    pub log_format: LogFormat,

    /// Seconds to wait for in-flight work to finish before a forced shutdown.
    ///
    /// Both binaries bound by this, and both exit non-zero when it elapses
    /// rather than when it does not: `vpay-server` gives in-flight HTTP
    /// requests at most this many seconds to finish, and `vpay-worker-bin`
    /// gives in-flight *jobs* the same (`vpay_worker::run_loop`'s drain).
    ///
    /// The worker's cutoff is not free, which is why the number matters more
    /// there. A job cut off mid-flight has already had its `attempts`
    /// incremented and may have called a rail; the drain hands its lease
    /// back (`vpay_db::jobs::release_all`) so another worker re-runs it
    /// immediately rather than after the lease interval, and every handler
    /// is written as a compare-and-swap so the re-run is a no-op if the
    /// first pass committed. Set this comfortably above
    /// `vpay_provider::DEFAULT_REQUEST_TIMEOUT` (20 s) so an ordinary poll
    /// waiting on a rail is not the thing that gets cut off.
    #[arg(long, env = "VPAY_SHUTDOWN_GRACE_SECONDS", default_value_t = 25)]
    pub shutdown_grace_seconds: u64,
}

/// Redacts `database_url` (a Postgres connection string that routinely
/// embeds a plaintext password) while leaving every other field visible.
///
/// Prints only whether `database_url` is set, not its value — same
/// redaction shape as [`crate::config::ProviderHost`]'s hand-written impl:
/// presence is the useful debugging signal (did the flag/env var resolve at
/// all), the value never is.
impl fmt::Debug for CommonArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommonArgs")
            .field(
                "database_url",
                &self.database_url.as_deref().map(|_| "[redacted]"),
            )
            .field("profile", &self.profile)
            .field("config", &self.config)
            .field("log_filter", &self.log_filter)
            .field("log_format", &self.log_format)
            .field("shutdown_grace_seconds", &self.shutdown_grace_seconds)
            .finish()
    }
}

/// `vpay-server` CLI.
///
/// `#[derive(Debug)]` is safe here even though `common: CommonArgs` carries
/// `database_url`: the derive formats `common` via *its own* `Debug` impl,
/// which [`CommonArgs`] hand-writes to redact — same composition argued in
/// [`crate::config::Config`]'s doc comment, proved by this module's tests.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "vpay-server",
    version,
    about = "vpay payment gateway API server",
    long_about = "vpay payment gateway API server.\n\nWrites rows and returns; it never calls a payment rail itself \
                  (see docs/flows). This binary is a scaffold — run with --help \
                  to see the full flag set, and see docs/status.md for what is \
                  actually implemented behind it."
)]
pub struct ServerArgs {
    /// Socket address the HTTP listener binds to.
    #[arg(long, env = "VPAY_BIND", default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

    /// Public base URL this deployment is reachable at.
    ///
    /// Used to build redirect-rail return URLs and webhook endpoints once
    /// those exist.
    #[arg(long, env = "VPAY_PUBLIC_BASE_URL")]
    pub public_base_url: Option<String>,

    /// Path to the file holding this deployment's RS256 signing key, as a
    /// PEM-encoded RSA private key (PKCS#8 or PKCS#1).
    ///
    /// In a real deployment this is a Kubernetes Secret mounted into the
    /// pod's filesystem — never an environment variable holding the key
    /// itself, and never a value in the YAML config. The process reads it
    /// once at boot (`vpay_api::op::keys::LoadedSigningKey::from_file`),
    /// derives the `kid` from it, and never writes it anywhere: migration
    /// `0010_reshape-oauth-signing-keys.sql` dropped the column that used to
    /// hold private key material precisely so that this file is the only
    /// place it exists.
    ///
    /// `Option<PathBuf>` at the clap level only, for the same reason
    /// [`CommonArgs::database_url`] and [`CommonArgs::config`] are — the
    /// "what does this process need to run" decision belongs to `main.rs`,
    /// not to the parser. `vpay-server` treats it as required at runtime,
    /// because a server that boots without a signing key can serve no
    /// authenticated surface at all. `vpay-worker-bin` deliberately does not
    /// take this flag: the worker issues no tokens, so mounting the signing
    /// key into it would widen the Secret's blast radius for no capability.
    ///
    /// **The path is not redacted from `Debug`, deliberately**: a filesystem
    /// path is not secret, and "which file did it try" is the first thing an
    /// operator needs when a Secret is misconfigured — the same reasoning as
    /// [`CommonArgs`]'s `Debug`, which prints `config`'s path in full and
    /// redacts only `database_url`, whose *value* embeds a password. Nothing
    /// in this crate ever reads the file, so no key material passes through
    /// `ServerArgs` at all.
    #[arg(long, env = "VPAY_OAUTH_SIGNING_KEY_FILE")]
    pub oauth_signing_key_file: Option<PathBuf>,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// `vpay-worker-bin` CLI.
///
/// Same composition note as [`ServerArgs`]: `#[derive(Debug)]` is safe
/// because `common: CommonArgs` formats via its own redacting `Debug` impl.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "vpay-worker-bin",
    version,
    about = "vpay background worker",
    long_about = "vpay background worker: submit, poll, reconcile, deliver.\n\nClaims jobs \
                  from the `jobs` table, drives live charges to a terminal state against \
                  the configured rails, and sweeps what has expired. Webhook delivery is \
                  not implemented (docs/status.md)."
)]
pub struct WorkerArgs {
    /// How many jobs this worker runs at once.
    ///
    /// One `tokio` task per unit, each claiming, running and settling one job
    /// at a time (`vpay_worker::run_loop`). Four by default, because the
    /// number that matters is not CPU: every job is dominated by one
    /// authenticated rail request, so this is really "how many rail requests
    /// in flight per worker", and it is bounded from both ends. Below it, a
    /// deployment adds workers rather than raising this — horizontal scaling
    /// is what the `FOR UPDATE SKIP LOCKED` claim is for. Above it, the
    /// binding constraint is the *rail's* tolerance, not ours: mobile-money
    /// APIs rate-limit per partner account, and a worker that opens fifty
    /// concurrent status queries gets throttled into a retry storm that looks
    /// like an outage.
    ///
    /// It is also bounded by the Postgres pool (`vpay_db::connect`): each
    /// task holds a connection for the duration of a claim and of each write,
    /// so a concurrency far above the pool size turns rail latency into pool
    /// contention.
    ///
    /// Zero is refused by [`WorkerArgs::concurrency`] rather than silently
    /// treated as one — a worker configured to run no jobs is a deployment
    /// mistake that would otherwise look like a healthy, permanently idle
    /// process.
    #[arg(long, env = "VPAY_WORKER_CONCURRENCY", default_value_t = 4)]
    pub worker_concurrency: usize,

    #[command(flatten)]
    pub common: CommonArgs,
}

impl WorkerArgs {
    /// [`Self::worker_concurrency`], refusing zero.
    ///
    /// Returns the flag's spelling in the error rather than a bare number,
    /// because the whole content of this failure is which knob to turn. It is
    /// checked here rather than with a `clap` `value_parser` range so the
    /// message names both the flag and its environment variable — the
    /// container case, where nobody typed a flag at all.
    ///
    /// # Errors
    ///
    /// A message naming the flag if the value is zero.
    pub fn concurrency(&self) -> Result<usize, String> {
        if self.worker_concurrency == 0 {
            return Err(
                "--worker-concurrency / VPAY_WORKER_CONCURRENCY is 0: this worker would claim \
                 no jobs at all and would look healthy while every live charge went undriven"
                    .to_owned(),
            );
        }
        Ok(self.worker_concurrency)
    }
}

// NOTE on what is and is not tested in *this* module: clap's `env`
// resolution reads straight from `std::env::var_os` with no injectable
// source, so actually exercising "the CLI resolves a value from a real
// environment variable" deterministically would require mutating the
// current process's real environment. `std::env::set_var`/`remove_var` are
// `unsafe` as of edition 2024 (not thread-safe against a parallel test run)
// — and this workspace sets `unsafe_code = "forbid"` (`Cargo.toml`
// `[workspace.lints.rust]`, `AGENTS.md`: "`unsafe` is forbidden"), as a hard
// `rustc`-level forbid with no per-test carve-out. So unlike
// `unwrap`/`expect`/`panic`, there is no exemption available here even
// inside `#[cfg(test)]`.
//
// This module instead tests two things that together cover the contract
// without ever touching process env:
//   1. Every option on both `ServerArgs` and `WorkerArgs` declares the exact
//      env var name we document (`server_command_declares_the_documented_env_vars`
//      etc. below) — this is read straight off the built `clap::Command`, so
//      renaming or dropping an `env = "..."` attribute fails a test.
//   2. Flags parse to the values they carry (defaults / explicit flags).
//
// The actual end-to-end proof that setting an env var on a *child process*
// changes the parsed result — including the flag-beats-env precedence case —
// lives in `backends/apps/vpay-server/tests/cli.rs` and
// `backends/apps/vpay-worker-bin/tests/cli.rs`. Those use
// `std::process::Command::env`, which sets only the *child's* environment
// (a safe API, no `unsafe`, no interference with this process or other
// tests), so they can control real env vars without hitting the forbid
// above.
#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{LogFormat, ServerArgs, WorkerArgs};

    /// `(arg id, expected env var)` for every option common to both
    /// binaries, i.e. every field of [`CommonArgs`](super::CommonArgs).
    const COMMON_ENV_VARS: [(&str, &str); 6] = [
        ("database_url", "DATABASE_URL"),
        ("profile", "VPAY_PROFILE"),
        ("config", "VPAY_CONFIG"),
        ("log_filter", "RUST_LOG"),
        ("log_format", "VPAY_LOG_FORMAT"),
        ("shutdown_grace_seconds", "VPAY_SHUTDOWN_GRACE_SECONDS"),
    ];

    /// `(arg id, expected env var)` for options unique to `vpay-worker-bin`.
    ///
    /// Its own table rather than an entry in [`COMMON_ENV_VARS`]: concurrency
    /// is a property of a process that *claims jobs*, and the server claims
    /// none. `the_server_is_not_given_a_worker_concurrency` below fails if it
    /// is ever flattened into `CommonArgs` for symmetry.
    const WORKER_ONLY_ENV_VARS: [(&str, &str); 1] =
        [("worker_concurrency", "VPAY_WORKER_CONCURRENCY")];

    /// `(arg id, expected env var)` for options unique to `vpay-server`.
    ///
    /// `oauth_signing_key_file` is here and not in `COMMON_ENV_VARS` on
    /// purpose: only the server issues tokens, so only the server is handed
    /// the Secret. `worker_command_declares_the_documented_env_vars` below
    /// would start passing if it were ever flattened into `CommonArgs`, but
    /// `the_worker_is_not_handed_the_signing_key` fails first.
    const SERVER_ONLY_ENV_VARS: [(&str, &str); 3] = [
        ("bind", "VPAY_BIND"),
        ("public_base_url", "VPAY_PUBLIC_BASE_URL"),
        ("oauth_signing_key_file", "VPAY_OAUTH_SIGNING_KEY_FILE"),
    ];

    /// Asserts a single arg on a built [`clap::Command`] declares exactly
    /// the expected env var — reading clap's own metadata, no process env
    /// involved.
    fn assert_env_var(cmd: &clap::Command, arg_id: &str, expected_env: &str) {
        let arg = cmd
            .get_arguments()
            .find(|a| a.get_id().as_str() == arg_id)
            .unwrap_or_else(|| panic!("arg `{arg_id}` not found on `{}`", cmd.get_name()));
        assert_eq!(
            arg.get_env(),
            Some(std::ffi::OsStr::new(expected_env)),
            "arg `{arg_id}` on `{}` should resolve from env var `{expected_env}`",
            cmd.get_name()
        );
    }

    #[test]
    fn server_command_is_well_formed() {
        <ServerArgs as CommandFactory>::command().debug_assert();
    }

    #[test]
    fn worker_command_is_well_formed() {
        <WorkerArgs as CommandFactory>::command().debug_assert();
    }

    #[test]
    fn server_command_declares_the_documented_env_vars() {
        let cmd = <ServerArgs as CommandFactory>::command();
        for (id, env) in COMMON_ENV_VARS.iter().chain(SERVER_ONLY_ENV_VARS.iter()) {
            assert_env_var(&cmd, id, env);
        }
    }

    #[test]
    fn worker_command_declares_the_documented_env_vars() {
        let cmd = <WorkerArgs as CommandFactory>::command();
        for (id, env) in COMMON_ENV_VARS.iter().chain(WORKER_ONLY_ENV_VARS.iter()) {
            assert_env_var(&cmd, id, env);
        }
    }

    /// The server runs no job loop, so a `--worker-concurrency` on it would
    /// be a knob that changes nothing — the shape of dormant configuration
    /// this repository refuses.
    #[test]
    fn the_server_is_not_given_a_worker_concurrency() {
        let server = <ServerArgs as CommandFactory>::command();
        assert!(
            !server
                .get_arguments()
                .any(|arg| arg.get_id().as_str() == "worker_concurrency"),
            "vpay-server claims no jobs; a concurrency flag there would configure nothing"
        );
        assert!(
            ServerArgs::try_parse_from(["vpay-server", "--worker-concurrency", "8"]).is_err(),
            "vpay-server accepted --worker-concurrency"
        );
    }

    /// The default is the documented four, and it reaches the field the
    /// binary reads. Pinned as a literal so a change to it is a change to
    /// this test — the number is a rail-politeness decision (see the field's
    /// doc comment), not an implementation detail.
    #[test]
    fn the_worker_concurrency_defaults_to_four_and_parses_what_it_is_given() {
        assert_eq!(
            WorkerArgs::parse_from(["vpay-worker-bin"]).worker_concurrency,
            4
        );
        let args = WorkerArgs::parse_from(["vpay-worker-bin", "--worker-concurrency", "16"]);
        assert_eq!(args.worker_concurrency, 16);
        assert_eq!(args.concurrency(), Ok(16));
    }

    /// Zero is a deployment mistake that would otherwise present as a
    /// permanently idle, permanently healthy worker. It parses (so the
    /// message can name the flag) and is refused by `concurrency()`.
    #[test]
    fn a_worker_concurrency_of_zero_is_refused_by_name() {
        let args = WorkerArgs::parse_from(["vpay-worker-bin", "--worker-concurrency", "0"]);
        let error = args
            .concurrency()
            .expect_err("0 must not be accepted as a concurrency");
        assert!(
            error.contains("--worker-concurrency") && error.contains("VPAY_WORKER_CONCURRENCY"),
            "the refusal must name both spellings of the knob to turn; got: {error}"
        );
    }

    /// Every option in this CLI is meant to auto-resolve from the
    /// environment (the whole point of Task 1). This catches an option added
    /// later without an `env = "..."` attribute, on either binary.
    #[test]
    fn every_declared_option_on_both_commands_has_an_env_var() {
        for cmd in [
            <ServerArgs as CommandFactory>::command(),
            <WorkerArgs as CommandFactory>::command(),
        ] {
            for arg in cmd.get_arguments() {
                let id = arg.get_id().as_str();
                if id == "help" || id == "version" {
                    continue;
                }
                assert!(
                    arg.get_env().is_some(),
                    "`{id}` on `{}` has no env var",
                    cmd.get_name()
                );
            }
        }
    }

    /// `CommonArgs` is `#[command(flatten)]`ed into both binaries. This
    /// confirms the flattened options carry byte-identical env var names on
    /// both commands, so the two binaries provably cannot drift on a shared
    /// flag.
    #[test]
    fn the_flattened_common_args_are_identical_on_both_binaries() {
        let server = <ServerArgs as CommandFactory>::command();
        let worker = <WorkerArgs as CommandFactory>::command();
        for (id, env) in COMMON_ENV_VARS {
            assert_env_var(&server, id, env);
            assert_env_var(&worker, id, env);
        }
    }

    #[test]
    fn server_defaults_match_the_documented_contract() {
        let args = ServerArgs::parse_from(["vpay-server"]);

        assert_eq!(args.bind, "0.0.0.0:8080".parse().expect("valid addr"));
        assert_eq!(args.public_base_url, None);
        assert_eq!(args.oauth_signing_key_file, None);
        assert_eq!(args.common.database_url, None);
        assert_eq!(args.common.profile, "sandbox");
        assert_eq!(args.common.config, None);
        assert_eq!(args.common.log_filter, "info");
        assert_eq!(args.common.log_format, LogFormat::Json);
        assert_eq!(args.common.shutdown_grace_seconds, 25);
    }

    #[test]
    fn worker_defaults_match_the_documented_contract() {
        let args = WorkerArgs::parse_from(["vpay-worker-bin"]);

        assert_eq!(args.common.database_url, None);
        assert_eq!(args.common.profile, "sandbox");
        assert_eq!(args.common.config, None);
        assert_eq!(args.common.log_filter, "info");
        assert_eq!(args.common.log_format, LogFormat::Json);
        assert_eq!(args.common.shutdown_grace_seconds, 25);
        assert_eq!(args.worker_concurrency, 4);
    }

    /// The signing-key path parses as a path and reaches the field the
    /// server reads it from. `--oauth-signing-key-file` is the kebab-case
    /// spelling clap derives from the field name; pinning it here means a
    /// rename cannot silently change the flag a Helm chart passes.
    #[test]
    fn the_signing_key_file_flag_parses_to_the_path_it_was_given() {
        let args = ServerArgs::parse_from([
            "vpay-server",
            "--oauth-signing-key-file",
            "/etc/vpay/secrets/oauth-signing-key.pem",
        ]);

        assert_eq!(
            args.oauth_signing_key_file,
            Some(std::path::PathBuf::from(
                "/etc/vpay/secrets/oauth-signing-key.pem"
            ))
        );
    }

    /// The worker mints no tokens, so it must not accept — and a deployment
    /// must not be able to mount — the signing key against it. This fails if
    /// the flag is ever moved into `CommonArgs` for symmetry.
    #[test]
    fn the_worker_is_not_handed_the_signing_key() {
        let worker = <WorkerArgs as CommandFactory>::command();
        assert!(
            !worker
                .get_arguments()
                .any(|arg| arg.get_id().as_str() == "oauth_signing_key_file"),
            "the worker issues no tokens; giving it the signing key widens the Secret's blast \
             radius for no capability"
        );

        assert!(
            WorkerArgs::try_parse_from([
                "vpay-worker-bin",
                "--oauth-signing-key-file",
                "/etc/vpay/secrets/oauth-signing-key.pem",
            ])
            .is_err(),
            "the worker must reject the flag outright, not ignore it"
        );
    }

    /// A path is not a secret and stays visible in `Debug` — the flip side
    /// of the `database_url` redaction above, and stated as a test so that
    /// "should this be redacted too?" has a recorded answer rather than
    /// being re-litigated by whoever reads the `Debug` impl next.
    #[test]
    fn the_signing_key_path_stays_visible_in_debug_output() {
        let args = ServerArgs::parse_from([
            "vpay-server",
            "--oauth-signing-key-file",
            "/etc/vpay/secrets/oauth-signing-key.pem",
        ]);

        let formatted = format!("{args:?}");
        assert!(
            formatted.contains("/etc/vpay/secrets/oauth-signing-key.pem"),
            "an operator diagnosing a missing Secret mount needs the path: {formatted}"
        );
    }

    #[test]
    fn an_explicit_flag_overrides_the_default() {
        let args = ServerArgs::parse_from(["vpay-server", "--bind", "127.0.0.1:9999"]);
        assert_eq!(args.bind, "127.0.0.1:9999".parse().expect("valid addr"));
    }

    #[test]
    fn explicit_flags_resolve_through_the_flattened_common_args() {
        let args = WorkerArgs::parse_from([
            "vpay-worker-bin",
            "--profile",
            "prod-config",
            "--log-format",
            "text",
            "--shutdown-grace-seconds",
            "5",
        ]);

        assert_eq!(args.common.profile, "prod-config");
        assert_eq!(args.common.log_format, LogFormat::Text);
        assert_eq!(args.common.shutdown_grace_seconds, 5);
    }

    #[test]
    fn an_unparseable_bind_address_is_a_clean_parse_error_not_a_panic() {
        let result = ServerArgs::try_parse_from(["vpay-server", "--bind", "not-an-address"]);
        assert!(result.is_err());
    }

    /// A `--database-url` password must never appear in `CommonArgs`'s
    /// `Debug` output. This is the test that would fail if someone
    /// re-derived `Debug` on `CommonArgs`.
    #[test]
    fn common_args_debug_output_never_contains_the_database_password() {
        let args = ServerArgs::parse_from([
            "vpay-server",
            "--database-url",
            "postgres://vpay:hunter2-live-password@db.internal:5432/vpay",
        ]);

        let formatted = format!("{:?}", args.common);

        assert!(
            !formatted.contains("hunter2-live-password"),
            "database password leaked into Debug output: {formatted}"
        );
    }

    /// Same check through the whole `ServerArgs`/`WorkerArgs` — the types
    /// actually likely to be logged at startup (see the doc comment on
    /// `CommonArgs`) — proving the derive-delegates-to-nested-Debug
    /// composition holds for both binaries' top-level parsers, not just
    /// `CommonArgs` in isolation.
    #[test]
    fn server_and_worker_args_debug_output_never_contains_the_database_password() {
        let server = ServerArgs::parse_from([
            "vpay-server",
            "--database-url",
            "postgres://vpay:hunter2-live-password@db.internal:5432/vpay",
        ]);
        let worker = WorkerArgs::parse_from([
            "vpay-worker-bin",
            "--database-url",
            "postgres://vpay:hunter2-live-password@db.internal:5432/vpay",
        ]);

        let server_formatted = format!("{server:?}");
        let worker_formatted = format!("{worker:?}");

        assert!(!server_formatted.contains("hunter2-live-password"));
        assert!(!worker_formatted.contains("hunter2-live-password"));
    }

    /// The redaction must not swallow everything: `profile`, `config`,
    /// `log_filter`, `log_format`, and `shutdown_grace_seconds` must stay
    /// visible, and `database_url`'s *presence* (not its value) must still
    /// be observable — otherwise the redacted `Debug` is useless for
    /// diagnosing "did the flag even resolve."
    #[test]
    fn common_args_debug_output_still_contains_the_non_secret_fields() {
        let args = ServerArgs::parse_from([
            "vpay-server",
            "--database-url",
            "postgres://vpay:hunter2-live-password@db.internal:5432/vpay",
            "--profile",
            "prod-config",
            "--log-format",
            "text",
            "--shutdown-grace-seconds",
            "5",
        ]);

        let formatted = format!("{:?}", args.common);

        assert!(formatted.contains("prod-config"), "{formatted}");
        assert!(formatted.contains("Text"), "{formatted}");
        assert!(formatted.contains('5'), "{formatted}");
        assert!(formatted.contains("[redacted]"), "{formatted}");
    }

    /// When `database_url` is unset, `Debug` must say so plainly (`None`),
    /// not silently omit the field or claim a redacted value exists.
    #[test]
    fn common_args_debug_output_shows_none_when_database_url_is_unset() {
        let args = ServerArgs::parse_from(["vpay-server"]);
        let formatted = format!("{:?}", args.common);
        assert!(formatted.contains("database_url: None"), "{formatted}");
    }
}
