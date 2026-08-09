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
#[derive(Debug, Clone, Parser)]
pub struct CommonArgs {
    /// Postgres connection string.
    ///
    /// Accepted and threaded through even though the database layer is not
    /// implemented yet (`docs/status.md`) — the wiring exists so turning the
    /// layer on later is a matter of using this value, not inventing a new
    /// flag.
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
    /// Optional because figment layering / file loading is not implemented
    /// yet (`docs/status.md`).
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
    /// `vpay-server` bounds by this: once the shutdown signal fires,
    /// in-flight HTTP requests get at most this many seconds to finish
    /// before the process stops waiting and exits (see `main.rs`).
    /// `vpay-worker-bin` accepts and validates this same flag for parity
    /// across binaries, but has no in-flight work to bound yet — the job
    /// loop is not implemented (`docs/status.md`) — so today it has no
    /// effect there.
    #[arg(long, env = "VPAY_SHUTDOWN_GRACE_SECONDS", default_value_t = 25)]
    pub shutdown_grace_seconds: u64,
}

/// `vpay-server` CLI.
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

    #[command(flatten)]
    pub common: CommonArgs,
}

/// `vpay-worker-bin` CLI.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "vpay-worker-bin",
    version,
    about = "vpay background worker",
    long_about = "vpay background worker: submit, poll, reconcile, deliver.\n\nThe job \
                  loop is not implemented yet (docs/status.md) — this process stays up \
                  and answers shutdown signals so orchestration (docker compose, k8s) \
                  behaves correctly around it, but it processes no jobs."
)]
pub struct WorkerArgs {
    #[command(flatten)]
    pub common: CommonArgs,
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

    /// `(arg id, expected env var)` for options unique to `vpay-server`.
    const SERVER_ONLY_ENV_VARS: [(&str, &str); 2] = [
        ("bind", "VPAY_BIND"),
        ("public_base_url", "VPAY_PUBLIC_BASE_URL"),
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
        for (id, env) in COMMON_ENV_VARS {
            assert_env_var(&cmd, id, env);
        }
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
}
