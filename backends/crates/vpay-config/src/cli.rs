//! Command-line configuration for `vpay-server` and its subcommands.
//!
//! Every option auto-resolves from an environment variable (via clap's `env`
//! feature) so the same binary works identically whether it is invoked with
//! flags, with env vars (as in `compose.yml` / a container), or a mix — with
//! an explicit flag always winning over its environment variable.
//!
//! # One binary, three modes (issue #77, 2026-09-07)
//!
//! There were two top-level parsers here until 2026-09-07: `ServerArgs` and
//! a `WorkerArgs` that carried its own `#[command(flatten)] common`, because
//! `vpay-server` and `vpay-worker-bin` were two packages producing two
//! binaries and two images. They are one package and one binary now, and
//! [`WorkerArgs`] is a [`clap::Args`] group on the `worker` subcommand
//! rather than a [`clap::Parser`] of its own.
//!
//! What that costs, and how it is paid: the flags the two processes shared
//! used to be *provably* identical, because [`CommonArgs`] was flattened
//! into both parsers and a test read both `clap::Command`s and compared
//! them. With one parser there is nothing left to compare — so every field
//! of `CommonArgs` is `global = true` instead, which is what keeps
//! `vpay-server worker --config x` and `vpay-server --config x worker`
//! *both* working, and therefore keeps every invocation `vpay-worker-bin`
//! accepted working unchanged apart from the inserted subcommand word.
//! `the_common_options_are_accepted_on_either_side_of_the_worker_subcommand`
//! below is what proves it, because a `global = true` dropped from one field
//! is otherwise silent: the flag still parses before the subcommand and
//! fails only after it.
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
//!
//! # `--public-base-url` is gone (step-6 decision (7))
//!
//! It was accepted, parsed and read by nothing. The URL that actually
//! matters — the one the merchant OP derives its `issuer` from — is
//! `deployment.public_base_url` in the YAML config
//! (`vpay_api::op::issuer_for`), and it always was. Two spellings of one
//! idea, one of them inert, is a trap: an operator who set the flag and
//! watched the issuer not change had no way to find out why.
//!
//! Passing `--public-base-url` now fails at parse time, loudly, which is the
//! answer a deployment wants. Setting `VPAY_PUBLIC_BASE_URL` does **not**
//! fail — clap reads an environment variable only for a flag it declares, so
//! a stale variable in a compose file or a Secret is simply ignored. Nothing
//! in this repository sets it any more (`.env.example` dropped its row in the
//! same change); a downstream deployment that still does will see no error
//! and no effect, exactly as before.

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

/// Options every mode of `vpay-server` takes.
///
/// Flattened into [`ServerArgs`] once, and **every field is
/// `global = true`**: `serve`, `worker` and `staff add` are one process's
/// three modes, not three programs, so `--config` means the same thing
/// wherever it appears on the line. Before issue #77 the same guarantee came
/// from flattening this struct into two separate parsers; see the module
/// docs for why the mechanism changed and which test replaced the one that
/// compared the two `clap::Command`s.
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
    /// `Option<String>` at the clap level only: `serve`, `worker` and
    /// `staff add` all treat this as required at runtime (see
    /// `vpay-server`'s `main.rs` and `worker.rs`) — a missing value is a
    /// hard startup failure,
    /// not a silently DB-less scaffold mode. It stays optional here rather
    /// than `required = true` on the `clap` attribute so the CLI-parsing
    /// layer and the binary-level "what does this process actually need to
    /// run" decision stay separate concerns; `config` (below) follows the
    /// same shape for the identical reason.
    ///
    /// Routinely embeds a password (`postgres://user:pass@host/db`) — see
    /// [`CommonArgs`]'s hand-written `Debug` impl below, which prints only
    /// whether this is set, never its value.
    #[arg(long, env = "DATABASE_URL", global = true)]
    pub database_url: Option<String>,

    /// Deployment profile label.
    ///
    /// Selects which YAML config file to load — never a code path. See the
    /// module-level docs above and `docs/adr/0003-yaml-configuration.md`.
    #[arg(long, env = "VPAY_PROFILE", default_value = "sandbox", global = true)]
    pub profile: String,

    /// Path to the YAML configuration file (ADR-0003).
    ///
    /// `Option<PathBuf>` at the clap level only, for the same reason
    /// [`Self::database_url`] is — see that field's doc comment. Every mode
    /// of `vpay-server` treats this as required at
    /// runtime: `vpay_config::Config::load` is called with it before the
    /// process connects to the database or binds a listener, and a missing
    /// value is a hard, loud startup failure. A payment gateway that boots
    /// with no validated deployment configuration is exactly the
    /// half-configured process ADR-0003 says must never serve traffic.
    #[arg(long, env = "VPAY_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// `tracing-subscriber` env-filter directive, e.g. `info` or
    /// `vpay_api=debug,info`.
    #[arg(long, env = "RUST_LOG", default_value = "info", global = true)]
    pub log_filter: String,

    /// Structured logging output format.
    #[arg(
        long,
        env = "VPAY_LOG_FORMAT",
        value_enum,
        default_value = "json",
        global = true
    )]
    pub log_format: LogFormat,

    /// Socket address the observability listener binds to: `GET /livez` and
    /// `GET /metrics`, under **both** `serve` and `worker`.
    ///
    /// A second listener rather than two more routes on `--bind`, and the
    /// separation is the point rather than an implementation detail.
    /// `/metrics` names every rail this deployment talks to, every route it
    /// serves and every error code it has produced — an operational map of
    /// the system, and one that must never be reachable from the internet.
    /// `--bind`'s port is the one behind the Ingress; this one is not
    /// (`deploy/helm/vpay/templates/networkpolicy.yaml` admits it from the
    /// monitoring namespace only), so keeping them apart is what makes that
    /// policy expressible at all. `vpay_api::router` deliberately mounts
    /// neither path, and a test in that crate fails if either appears.
    ///
    /// It is on `CommonArgs` — not beside `--bind` — because the `worker`
    /// subcommand needs it *more*: the worker has no other listener, so
    /// before this flag existed it had no liveness probe and no way to
    /// export the queue-depth gauge that tells an operator it is falling
    /// behind. `staff add` binds it too, and that is the one place the
    /// sharing is untidy rather than useful: a subcommand that writes one
    /// row and exits opens no listener at all (`vpay-server`'s
    /// `run_command` returns before `start_observability`).
    ///
    /// Port `9090` by default, matching `deploy/helm/vpay`'s
    /// `observability.port` and the `metrics` container port both
    /// Deployments declare. A `:0` port is a real configuration — the
    /// subprocess tests use it — and the bound address is logged, because
    /// with `:0` nothing else can know it.
    #[arg(
        long,
        env = "VPAY_OBSERVABILITY_BIND",
        default_value = "0.0.0.0:9090",
        global = true
    )]
    pub observability_bind: SocketAddr,

    /// Seconds to wait for in-flight work to finish before a forced shutdown.
    ///
    /// Both long-running modes bound by this, and both exit non-zero when it
    /// elapses rather than when it does not: `vpay-server serve` gives
    /// in-flight HTTP requests at most this many seconds to finish, and
    /// `vpay-server worker` gives in-flight *jobs* the same
    /// (`vpay_worker::run_loop`'s drain).
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
    #[arg(
        long,
        env = "VPAY_SHUTDOWN_GRACE_SECONDS",
        default_value_t = 25,
        global = true
    )]
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
            .field("observability_bind", &self.observability_bind)
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
    about = "vpay payment gateway",
    long_about = "vpay payment gateway.\n\nWith no subcommand it serves the API: it writes rows and returns, \
                  and it never calls a payment rail itself. `vpay-server worker` \
                  runs the job loop that does — submit, poll, reconcile, deliver \
                  (see docs/flows). One binary and one image since 2026-09-07 \
                  (issue #77). Run with --help to see the full flag set, and see \
                  docs/status.md for what is actually implemented behind it."
)]
pub struct ServerArgs {
    /// Socket address the HTTP listener binds to.
    #[arg(long, env = "VPAY_BIND", default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,

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
    /// not to the parser. `serve` treats it as required at runtime,
    /// because a server that boots without a signing key can serve no
    /// authenticated surface at all.
    ///
    /// **It is a top-level flag and it is deliberately NOT `global`**, so
    /// `vpay-server worker --oauth-signing-key-file …` is a parse error —
    /// the worker issues no token and reads no key.
    ///
    /// `vpay-server --oauth-signing-key-file … worker` is a parse error too,
    /// and *that* half is not clap's doing: the flag is written before the
    /// subcommand word, so clap accepts it, and [`SERVE_ONLY_FLAGS`] is what
    /// refuses it afterwards. It was accepted-and-read-by-nothing for the
    /// length of one review pass, between issue #77 (2026-09-07) folding
    /// `vpay-worker-bin` into this binary and the review of the same day.
    /// A flag a process accepts and never reads is the trap this module's
    /// header already describes for `--public-base-url`, and the answer is
    /// the same one: fail at parse time, loudly.
    ///
    /// **`VPAY_OAUTH_SIGNING_KEY_FILE` in the environment is still ignored,
    /// deliberately** — again exactly as `VPAY_PUBLIC_BASE_URL` is. It is
    /// what `vpay-worker-bin` did (clap read no env for a flag that binary
    /// did not declare), and a deployment that hands both containers one
    /// env block must not be a `CrashLoopBackOff`. The refusal is scoped to
    /// [`clap::parser::ValueSource::CommandLine`] for that reason: somebody
    /// who *typed* the flag beside `worker` is confused and should be told,
    /// where a shared ConfigMap is not.
    ///
    /// None of this is what keeps the Secret away from the worker. That is
    /// where the key is **mounted** —
    /// `deploy/helm/vpay/templates/deployment-worker.yaml` mounts no
    /// `signingKey` volume and sets no `VPAY_OAUTH_SIGNING_KEY_FILE`, and
    /// `compose.e2e.yml`'s worker service mounts no key file — and that was
    /// always the load-bearing half: a flag naming a path that is not in the
    /// container is not a leak.
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

    /// An operator subcommand instead of serving traffic.
    ///
    /// `None` — no subcommand — is the ordinary case and starts the server.
    /// A subcommand does its work and exits; it binds no listener, loads no
    /// signing key and reconciles nothing.
    ///
    /// # Why these live on the server binary rather than in an `xtask`
    ///
    /// `cargo xtask` is a *development* tool: it needs the source tree and a
    /// Rust toolchain, neither of which exists in the `FROM scratch` runtime
    /// image (ADR-0004). Creating the first staff member is something an
    /// operator does against a production database, from the same image
    /// that serves the API — `kubectl exec` or a one-shot Job — so the
    /// subcommand has to be in the shipped binary or it is not reachable
    /// where it is needed.
    #[command(subcommand)]
    pub command: Option<ServerCommand>,

    #[command(flatten)]
    pub common: CommonArgs,
}

/// The flags only the serve mode reads, as `(clap arg id, what an operator
/// typed)`.
///
/// Both are declared on [`ServerArgs`] and neither is `global`, so clap
/// already refuses them *after* the word `worker`. What clap cannot express
/// is that they are equally meaningless *before* it: `--bind` opens a port
/// the job loop does not route and `--oauth-signing-key-file` names a key it
/// does not sign with, and a flag a process accepts and never reads is a
/// trap — an operator who sets it and watches nothing change has no way to
/// find out why. `Command::args_conflicts_with_subcommands` would say this
/// declaratively and cannot be used: it conflicts *every* top-level arg with
/// every subcommand, which would take `--config` with it, and `--config`
/// working on either side of `worker` is the whole reason [`CommonArgs`] is
/// `global` (see this module's header).
///
/// Adding a row here is a decision about one flag. Do not add a `CommonArgs`
/// field to it — those are read by every mode by construction.
const SERVE_ONLY_FLAGS: [(&str, &str); 2] = [
    ("bind", "--bind"),
    ("oauth_signing_key_file", "--oauth-signing-key-file"),
];

impl ServerArgs {
    /// Parses this process's argv, or prints the failure and exits.
    ///
    /// **`main` must call this rather than `clap::Parser::parse`**, which is
    /// otherwise available on this type and does everything except the
    /// [`SERVE_ONLY_FLAGS`] check. The check is not expressible in clap's
    /// derive, so it lives one layer out; putting it in a function `main`
    /// calls is what makes the difference between the two entry points a
    /// single line rather than a rule to remember.
    ///
    /// # Panics
    ///
    /// Never returns on a parse failure, on `--help` or on `--version`: it
    /// exits the process the way `clap::Parser::parse` does, with clap's own
    /// exit code (`0` for help, `2` for a usage error).
    #[must_use]
    pub fn parse_checked() -> Self {
        match Self::try_parse_checked_from(std::env::args_os()) {
            Ok(args) => args,
            Err(error) => error.exit(),
        }
    }

    /// [`Self::parse_checked`] over an explicit argv, returning the error.
    ///
    /// Public and separate so the refusal is exercised by a unit test over a
    /// literal command line rather than only by a subprocess — and so the
    /// check cannot be bypassed by a caller that merely wanted a testable
    /// parse: this is the only entry point that both parses *and* checks.
    ///
    /// # Errors
    ///
    /// Any clap parse failure, plus [`clap::error::ErrorKind::ArgumentConflict`]
    /// when a [`SERVE_ONLY_FLAGS`] entry was written on the command line
    /// beside the `worker` subcommand.
    pub fn try_parse_checked_from<I, T>(argv: I) -> Result<Self, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        let mut command = <Self as clap::CommandFactory>::command();
        let mut matches = command.clone().try_get_matches_from(argv)?;
        refuse_serve_only_flags_before_worker(&mut command, &matches)?;
        <Self as clap::FromArgMatches>::from_arg_matches_mut(&mut matches)
            .map_err(|error| error.format(&mut command))
    }
}

/// Refuses a [`SERVE_ONLY_FLAGS`] entry typed beside `worker`.
///
/// Reads [`clap::ArgMatches::value_source`] rather than "is the value
/// present": `--bind` carries a `default_value`, so it is *always* present,
/// and an environment variable must stay ignored (see
/// [`ServerArgs::oauth_signing_key_file`] for why that is the deliberate
/// half). `ValueSource::CommandLine` is the only source that means a human
/// wrote the flag on the line that also says `worker`.
fn refuse_serve_only_flags_before_worker(
    command: &mut clap::Command,
    matches: &clap::ArgMatches,
) -> Result<(), clap::Error> {
    if matches.subcommand_name() != Some("worker") {
        return Ok(());
    }
    for (id, spelling) in SERVE_ONLY_FLAGS {
        if matches.value_source(id) != Some(clap::parser::ValueSource::CommandLine) {
            continue;
        }
        return Err(command.error(
            clap::error::ErrorKind::ArgumentConflict,
            format!(
                "`{spelling}` is read only when serving the API, and this command line says \
                 `worker`. It parses in that position because it was written before the \
                 subcommand word, but the job loop routes no traffic and signs no token, so \
                 the value would be read by nothing. Drop `{spelling}`, or drop `worker`."
            ),
        ));
    }
    Ok(())
}

/// The subcommands `vpay-server` offers, other than serving traffic.
///
/// Two: one long-running mode ([`Self::Worker`]) and one operator command
/// ([`Self::Staff`]). `None` — no subcommand at all — serves the API, which
/// is what keeps every `ENTRYPOINT`, every compose `command:` and every Helm
/// `args:` that predates issue #77 working with no edit.
///
/// There is deliberately **no `serve` variant**. A `serve` that had to be
/// spelled would break exactly those entrypoints, and a `serve` that were
/// accepted-but-optional would be two ways to say one thing, one of which no
/// deployment file in this repository uses.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum ServerCommand {
    /// Run the job loop: submit, poll, reconcile, deliver.
    ///
    /// The long-running mode that drives live charges. It was the separate
    /// binary `vpay-worker-bin` until 2026-09-07 (issue #77); nothing about
    /// what it *does* changed when it became a subcommand, and
    /// [docs/flows/crash-safety.md](../../../../docs/flows/crash-safety.md)
    /// still describes the loop it runs.
    Worker(WorkerArgs),

    /// Manage the humans who may sign in to `/dash/v1`
    /// ([ADR-0017](../../../../docs/adr/0017-staff-authentication.md)).
    ///
    /// A nested `Subcommand` rather than a flat `staff-add` so that a second
    /// staff operation (disable, list) is a sibling rather than another
    /// top-level verb, and so `vpay-server staff --help` is a page about
    /// staff.
    Staff {
        /// What to do.
        #[command(subcommand)]
        command: StaffCommand,
    },
}

/// `vpay-server staff …`.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum StaffCommand {
    /// Create a staff member and print a one-time password.
    ///
    /// **The only way a staff member is created.** ADR-0017 decision 1: no
    /// HTTP endpoint creates one, and there is no self-service sign-up — a
    /// dashboard account is a decision an operator takes, not a form a
    /// visitor fills in.
    ///
    /// The password is generated, printed once, and must be replaced at
    /// first sign-in; TOTP enrolment is mandatory at the same sign-in.
    Add {
        /// The tenant this person may see. Must be a `merchant_id` some
        /// `merchant_clients` entry registers — checked before the insert,
        /// because there is no merchants table to make it a foreign key and
        /// a typo would otherwise be an account that can never see anything.
        #[arg(long)]
        merchant: String,

        /// Their email address. Lower-cased before it is written, and
        /// unique across the whole deployment: sign-in names an address and
        /// no tenant.
        #[arg(long)]
        email: String,

        /// What the dashboard greets them by. Not an identifier and never
        /// matched on.
        #[arg(long)]
        name: String,
    },
}

/// `vpay-server worker` — the flags the job loop takes that no other mode
/// does.
///
/// A [`clap::Args`] group on [`ServerCommand::Worker`], not a
/// [`clap::Parser`]: it was `vpay-worker-bin`'s own top-level parser, with
/// its own `#[command(flatten)] common: CommonArgs`, until issue #77 folded
/// the two binaries into one on 2026-09-07. It carries no `common` field of
/// its own now, because [`CommonArgs`] is flattened into [`ServerArgs`] with
/// every field `global = true` — so `--config`, `--database-url`,
/// `--observability-bind` and the rest are read from
/// [`ServerArgs::common`] whichever side of the word `worker` they were
/// written on.
///
/// One field, and that is the whole difference between this mode and
/// `serve`. It stays here rather than moving to `CommonArgs` for the reason
/// `the_serve_mode_is_not_given_a_worker_concurrency` states: a process that
/// claims no jobs has no use for a concurrency, and a knob that configures
/// nothing is the shape of dormant configuration this repository refuses.
#[derive(Debug, Clone, clap::Args)]
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
// This module instead tests three things that together cover the contract
// without ever touching process env:
//   1. Every option on `ServerArgs` and on the `worker` subcommand declares
//      the exact env var name we document (`server_command_declares_the_documented_env_vars`
//      etc. below) — this is read straight off the built `clap::Command`, so
//      renaming or dropping an `env = "..."` attribute fails a test.
//   2. Flags parse to the values they carry (defaults / explicit flags).
//   3. Since issue #77: that a common flag is accepted on **either** side of
//      the `worker` subcommand word and reaches the same field. That is the
//      test that replaced `the_flattened_common_args_are_identical_on_both_binaries`,
//      which compared two `clap::Command`s that no longer both exist.
//
// The actual end-to-end proof that setting an env var on a *child process*
// changes the parsed result — including the flag-beats-env precedence case —
// lives in `backends/apps/vpay-server/tests/cli.rs`, for `serve` and for
// `worker` alike. Those use `std::process::Command::env`, which sets only
// the *child's* environment (a safe API, no `unsafe`, no interference with
// this process or other tests), so they can control real env vars without
// hitting the forbid above.
#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use clap::{CommandFactory, Parser};

    use super::{LogFormat, ServerArgs, ServerCommand, WorkerArgs};

    /// `(arg id, expected env var)` for every option common to every mode,
    /// i.e. every field of [`CommonArgs`](super::CommonArgs).
    const COMMON_ENV_VARS: [(&str, &str); 7] = [
        ("database_url", "DATABASE_URL"),
        ("profile", "VPAY_PROFILE"),
        ("config", "VPAY_CONFIG"),
        ("log_filter", "RUST_LOG"),
        ("log_format", "VPAY_LOG_FORMAT"),
        ("observability_bind", "VPAY_OBSERVABILITY_BIND"),
        ("shutdown_grace_seconds", "VPAY_SHUTDOWN_GRACE_SECONDS"),
    ];

    /// `(arg id, expected env var)` for options unique to the `worker`
    /// subcommand.
    ///
    /// Its own table rather than an entry in [`COMMON_ENV_VARS`]: concurrency
    /// is a property of a mode that *claims jobs*, and serving traffic claims
    /// none. `the_serve_mode_is_not_given_a_worker_concurrency` below fails if
    /// it is ever flattened into `CommonArgs` for symmetry.
    const WORKER_ONLY_ENV_VARS: [(&str, &str); 1] =
        [("worker_concurrency", "VPAY_WORKER_CONCURRENCY")];

    /// `(arg id, expected env var)` for options only the API server reads.
    ///
    /// `oauth_signing_key_file` is here and not in `COMMON_ENV_VARS` on
    /// purpose: only `serve` issues tokens, so only `serve` reads the Secret.
    /// `the_worker_subcommand_is_not_handed_the_signing_key` below is what
    /// fails if it is ever made `global` for symmetry with the rest of
    /// `CommonArgs`.
    const SERVER_ONLY_ENV_VARS: [(&str, &str); 2] = [
        ("bind", "VPAY_BIND"),
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

    /// The `worker` subcommand's own `clap::Command`, by the name an
    /// operator types and a compose `command:` carries.
    ///
    /// Looked up by string rather than reached through the enum, because the
    /// *string* is the contract: `compose.e2e.yml`, `compose.demo.yml` and
    /// `deployment-worker.yaml` all spell it, and clap derives it from the
    /// variant name. A rename would be a silent break in three files this
    /// crate cannot see.
    fn worker_subcommand() -> clap::Command {
        <ServerArgs as CommandFactory>::command()
            .find_subcommand("worker")
            .cloned()
            .expect("`vpay-server worker` must exist; compose and Helm spell it")
    }

    #[test]
    fn server_command_is_well_formed() {
        <ServerArgs as CommandFactory>::command().debug_assert();
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
        let cmd = worker_subcommand();
        for (id, env) in WORKER_ONLY_ENV_VARS {
            assert_env_var(&cmd, id, env);
        }
    }

    /// Serving traffic runs no job loop, so a `--worker-concurrency` there
    /// would be a knob that changes nothing — the shape of dormant
    /// configuration this repository refuses.
    #[test]
    fn the_serve_mode_is_not_given_a_worker_concurrency() {
        let server = <ServerArgs as CommandFactory>::command();
        assert!(
            !server
                .get_arguments()
                .any(|arg| arg.get_id().as_str() == "worker_concurrency"),
            "serving traffic claims no jobs; a concurrency flag there would configure nothing"
        );
        assert!(
            ServerArgs::try_parse_from(["vpay-server", "--worker-concurrency", "8"]).is_err(),
            "`vpay-server --worker-concurrency 8` (no subcommand) must not parse"
        );
    }

    /// The default is the documented four, and it reaches the field the
    /// binary reads. Pinned as a literal so a change to it is a change to
    /// this test — the number is a rail-politeness decision (see the field's
    /// doc comment), not an implementation detail.
    #[test]
    fn the_worker_concurrency_defaults_to_four_and_parses_what_it_is_given() {
        assert_eq!(worker_args(["vpay-server", "worker"]).worker_concurrency, 4);
        let args = worker_args(["vpay-server", "worker", "--worker-concurrency", "16"]);
        assert_eq!(args.worker_concurrency, 16);
        assert_eq!(args.concurrency(), Ok(16));
    }

    /// Parses a command line that must select [`ServerCommand::Worker`] and
    /// returns its [`WorkerArgs`], panicking with the parse error otherwise.
    fn worker_args<'a>(argv: impl IntoIterator<Item = &'a str>) -> WorkerArgs {
        let parsed = ServerArgs::try_parse_from(argv).expect("a valid `vpay-server worker` line");
        match parsed.command {
            Some(ServerCommand::Worker(args)) => args,
            other => panic!("expected the worker subcommand, got {other:?}"),
        }
    }

    /// Zero is a deployment mistake that would otherwise present as a
    /// permanently idle, permanently healthy worker. It parses (so the
    /// message can name the flag) and is refused by `concurrency()`.
    #[test]
    fn a_worker_concurrency_of_zero_is_refused_by_name() {
        let args = worker_args(["vpay-server", "worker", "--worker-concurrency", "0"]);
        let error = args
            .concurrency()
            .expect_err("0 must not be accepted as a concurrency");
        assert!(
            error.contains("--worker-concurrency") && error.contains("VPAY_WORKER_CONCURRENCY"),
            "the refusal must name both spellings of the knob to turn; got: {error}"
        );
    }

    /// Every option that configures a *deployment* auto-resolves from the
    /// environment (the whole point of Task 1). This catches one added later
    /// without an `env = "..."` attribute.
    ///
    /// The scope is the top-level command and `worker` — the two
    /// long-running modes, i.e. the ones a container starts with an
    /// environment and no argv. It is deliberately **not** every subcommand
    /// at every depth: `staff add`'s `--merchant`, `--email` and `--name`
    /// carry no env var and should not. They are the arguments of one
    /// operator invocation, not configuration of a running process; a
    /// `VPAY_EMAIL` in a pod's environment that silently decided who got an
    /// account would be a worse thing than the extra typing. This is the
    /// same scope the pre-#77 version of this test had (`ServerArgs` and
    /// `WorkerArgs`, the two top-level parsers), restated now that the shape
    /// no longer states it for us.
    #[test]
    fn every_deployment_option_on_both_long_running_modes_has_an_env_var() {
        for cmd in [
            <ServerArgs as CommandFactory>::command(),
            worker_subcommand(),
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

    /// **The test that replaced `the_flattened_common_args_are_identical_on_both_binaries`.**
    ///
    /// That one read two `clap::Command`s — `ServerArgs`'s and
    /// `WorkerArgs`'s — and compared the env var of every shared option, and
    /// it was the proof that the server and the worker could not drift.
    /// Issue #77 left one command, so there is nothing to compare; what has
    /// to hold instead is that a common option is accepted on **either** side
    /// of the `worker` word and lands in the same field either way. That is
    /// what `global = true` buys, and it is exactly what a `global` dropped
    /// from one field would break — silently, because the flag still parses
    /// in the position every current compose file and Helm chart happens not
    /// to use.
    ///
    /// Every field of `CommonArgs` that takes a value is exercised, not a
    /// representative one, because `global` is per-field.
    #[test]
    fn the_common_options_are_accepted_on_either_side_of_the_worker_subcommand() {
        let flags = [
            "--database-url",
            "postgres://vpay:vpay@db:5432/vpay",
            "--profile",
            "either-side",
            "--config",
            "/etc/vpay/application.yml",
            "--log-filter",
            "vpay_worker=debug",
            "--log-format",
            "text",
            "--observability-bind",
            "127.0.0.1:19091",
            "--shutdown-grace-seconds",
            "7",
        ];

        let mut before = vec!["vpay-server"];
        before.extend(flags);
        before.push("worker");

        let mut after = vec!["vpay-server", "worker"];
        after.extend(flags);

        for (position, argv) in [("before", before), ("after", after)] {
            let parsed = ServerArgs::try_parse_from(argv.clone()).unwrap_or_else(|e| {
                panic!(
                    "`{}` did not parse ({position} the subcommand): {e}",
                    argv.join(" ")
                )
            });
            assert!(
                matches!(parsed.command, Some(ServerCommand::Worker(_))),
                "{position}: the worker subcommand was not selected"
            );
            let common = parsed.common;
            assert_eq!(
                common.database_url.as_deref(),
                Some("postgres://vpay:vpay@db:5432/vpay"),
                "{position}: --database-url did not reach CommonArgs"
            );
            assert_eq!(common.profile, "either-side", "{position}: --profile");
            assert_eq!(
                common.config,
                Some(std::path::PathBuf::from("/etc/vpay/application.yml")),
                "{position}: --config"
            );
            assert_eq!(
                common.log_filter, "vpay_worker=debug",
                "{position}: --log-filter"
            );
            assert_eq!(
                common.log_format,
                LogFormat::Text,
                "{position}: --log-format"
            );
            assert_eq!(
                common.observability_bind,
                "127.0.0.1:19091".parse::<SocketAddr>().expect("valid addr"),
                "{position}: --observability-bind"
            );
            assert_eq!(
                common.shutdown_grace_seconds, 7,
                "{position}: --shutdown-grace-seconds"
            );
        }
    }

    /// A serve-only flag typed beside `worker` is refused, in the position
    /// clap accepts it.
    ///
    /// The pair this guards is asymmetric and that is the whole point.
    /// `vpay-server worker --bind …` is clap's own error, because `bind` is
    /// not on the subcommand and is not `global`
    /// (`the_worker_subcommand_binds_only_the_observability_listener`).
    /// `vpay-server --bind … worker` parses — clap sees a top-level flag
    /// followed by a subcommand and has no way to be told that this
    /// particular top-level flag means nothing to that particular
    /// subcommand. For one review pass it therefore parsed and was read by
    /// nothing, which is the `--public-base-url` trap this module's header
    /// describes, re-created by issue #77.
    ///
    /// Both flags, both orders, and the message names the flag — an
    /// operator's whole fix is knowing which word to delete.
    #[test]
    fn a_serve_only_flag_written_before_the_worker_subcommand_is_refused() {
        for (spelling, value) in [
            ("--bind", "0.0.0.0:8080"),
            ("--oauth-signing-key-file", "/secrets/oauth-signing-key.pem"),
        ] {
            let error =
                ServerArgs::try_parse_checked_from(["vpay-server", spelling, value, "worker"])
                    .expect_err(
                        "a serve-only flag before `worker` must be refused, not parsed and ignored",
                    );
            assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
            assert!(
                error.to_string().contains(spelling),
                "the refusal must name the flag to delete, got: {error}"
            );

            // The same flag after the subcommand is clap's own refusal, and
            // it must stay one: this check exists so that "refused" cannot
            // quietly become "refused only in the position we remembered".
            assert!(
                ServerArgs::try_parse_checked_from(["vpay-server", "worker", spelling, value])
                    .is_err(),
                "`vpay-server worker {spelling}` must still be a parse error"
            );
        }
    }

    /// The refusal is scoped to `worker`, and to flags typed on the line.
    ///
    /// Three negatives, each of which a too-broad check would break: serving
    /// traffic still takes both flags; `staff add` is not `worker` and is
    /// left exactly as issue #80 shipped it; and a `worker` line carrying
    /// only `CommonArgs` flags before the subcommand word still parses —
    /// that last one is what `global = true` buys and what this check must
    /// not cost.
    ///
    /// The fourth case, `VPAY_OAUTH_SIGNING_KEY_FILE` in the *environment*
    /// staying ignored rather than refused, cannot be written here: setting a
    /// real environment variable needs `unsafe` (see this module's note
    /// above). It is
    /// `worker::the_signing_key_env_var_is_ignored_rather_than_refused` in
    /// `backends/apps/vpay-server/tests/cli.rs`, over a child process.
    #[test]
    fn the_serve_only_refusal_touches_nothing_else() {
        for argv in [
            vec![
                "vpay-server",
                "--bind",
                "0.0.0.0:8080",
                "--oauth-signing-key-file",
                "/secrets/k.pem",
            ],
            vec![
                "vpay-server",
                "--oauth-signing-key-file",
                "/secrets/k.pem",
                "staff",
                "add",
                "--merchant",
                "acme",
                "--email",
                "ops@acme.example",
                "--name",
                "Ops",
            ],
            vec![
                "vpay-server",
                "--config",
                "/config/application.yml",
                "--observability-bind",
                "127.0.0.1:19090",
                "worker",
            ],
        ] {
            assert!(
                ServerArgs::try_parse_checked_from(argv.clone()).is_ok(),
                "`{}` must still parse",
                argv.join(" ")
            );
        }
    }

    /// The same property for `staff add`, which is the other subcommand and
    /// the one an operator runs by hand — `kubectl exec … -- vpay-server
    /// staff add --config /config/application.yml …` is the shape the
    /// runbook uses, and it only works because `--config` is `global`.
    #[test]
    fn the_common_options_are_accepted_after_the_staff_subcommand_too() {
        let args = ServerArgs::try_parse_from([
            "vpay-server",
            "staff",
            "add",
            "--merchant",
            "acme",
            "--email",
            "ops@acme.example",
            "--name",
            "Ops",
            "--config",
            "/config/application.yml",
        ])
        .expect("`vpay-server staff add … --config …` must parse");
        assert_eq!(
            args.common.config,
            Some(std::path::PathBuf::from("/config/application.yml"))
        );
    }

    #[test]
    fn server_defaults_match_the_documented_contract() {
        let args = ServerArgs::parse_from(["vpay-server"]);

        assert!(
            args.command.is_none(),
            "no subcommand must mean serve, or every existing ENTRYPOINT breaks"
        );
        assert_eq!(args.bind, "0.0.0.0:8080".parse().expect("valid addr"));
        assert_eq!(args.oauth_signing_key_file, None);
        assert_eq!(args.common.database_url, None);
        assert_eq!(args.common.profile, "sandbox");
        assert_eq!(args.common.config, None);
        assert_eq!(args.common.log_filter, "info");
        assert_eq!(args.common.log_format, LogFormat::Json);
        assert_eq!(
            args.common.observability_bind,
            "0.0.0.0:9090".parse().expect("valid addr")
        );
        assert_eq!(args.common.shutdown_grace_seconds, 25);
    }

    #[test]
    fn worker_defaults_match_the_documented_contract() {
        let args = ServerArgs::parse_from(["vpay-server", "worker"]);

        assert_eq!(args.common.database_url, None);
        assert_eq!(args.common.profile, "sandbox");
        assert_eq!(args.common.config, None);
        assert_eq!(args.common.log_filter, "info");
        assert_eq!(args.common.log_format, LogFormat::Json);
        assert_eq!(
            args.common.observability_bind,
            "0.0.0.0:9090".parse().expect("valid addr")
        );
        assert_eq!(worker_args(["vpay-server", "worker"]).worker_concurrency, 4);
        assert_eq!(args.common.shutdown_grace_seconds, 25);
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

    /// The worker mints no tokens, so `vpay-server worker
    /// --oauth-signing-key-file …` must be refused rather than ignored. This
    /// fails if the flag is ever made `global = true` for symmetry with
    /// `CommonArgs`.
    ///
    /// This case covers clap's half — the flag is not on the subcommand's
    /// `Command` at all. The other spelling, `vpay-server
    /// --oauth-signing-key-file … worker`, parses at the clap level and is
    /// refused a layer out; that is
    /// `a_serve_only_flag_written_before_the_worker_subcommand_is_refused`.
    ///
    /// **What neither case claims**: that this is what keeps the Secret away
    /// from the worker. That is the **mount** — `deployment-worker.yaml`
    /// templates no `signingKey` volume and sets no
    /// `VPAY_OAUTH_SIGNING_KEY_FILE` — and the environment variable is still
    /// read by nothing rather than refused (see the field's doc comment).
    #[test]
    fn the_worker_subcommand_is_not_handed_the_signing_key() {
        let worker = worker_subcommand();
        assert!(
            !worker
                .get_arguments()
                .any(|arg| arg.get_id().as_str() == "oauth_signing_key_file"),
            "the worker issues no tokens; a signing-key flag on it would be a knob reading nothing"
        );

        assert!(
            ServerArgs::try_parse_from([
                "vpay-server",
                "worker",
                "--oauth-signing-key-file",
                "/etc/vpay/secrets/oauth-signing-key.pem",
            ])
            .is_err(),
            "`vpay-server worker --oauth-signing-key-file` must be a parse error"
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
        let args = ServerArgs::parse_from([
            "vpay-server",
            "worker",
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

    /// `--public-base-url` was removed in step 6 (decision (7)) because it
    /// was read by nothing. This is the test that keeps it removed: adding
    /// it back "for symmetry with the YAML" would reintroduce two spellings
    /// of one idea, one of them inert, which is the exact confusion the
    /// removal was for.
    ///
    /// Asserted as a hard parse *error* rather than as a missing arg id, and
    /// in the `worker` position as well as the bare one: an operator who
    /// still passes the flag must be told, rather than have it silently
    /// ignored.
    #[test]
    fn neither_mode_still_accepts_the_removed_public_base_url_flag() {
        let cmd = <ServerArgs as CommandFactory>::command();
        for cmd in std::iter::once(&cmd).chain(cmd.get_subcommands()) {
            assert!(
                !cmd.get_arguments()
                    .any(|arg| arg.get_id().as_str() == "public_base_url"),
                "`{}` still declares --public-base-url; the issuer comes from the YAML \
                 (`deployment.public_base_url`, vpay_api::op::issuer_for) and always did",
                cmd.get_name()
            );
        }

        assert!(
            ServerArgs::try_parse_from([
                "vpay-server",
                "--public-base-url",
                "https://api.vpay.example"
            ])
            .is_err(),
            "vpay-server must refuse the removed flag outright, not ignore it"
        );
        assert!(
            ServerArgs::try_parse_from([
                "vpay-server",
                "worker",
                "--public-base-url",
                "https://api.vpay.example"
            ])
            .is_err(),
            "`vpay-server worker` must refuse the removed flag outright, not ignore it"
        );
    }

    /// The observability listener is a knob **both** long-running modes have,
    /// and its default is the port `deploy/helm/vpay` templates its probes
    /// and its `ServiceMonitor` against. A change to either number breaks a
    /// chart that cannot see this crate, so the number is pinned here rather
    /// than left to a default-value string.
    #[test]
    fn both_modes_take_the_observability_bind_and_default_to_9090() {
        let expected: SocketAddr = "0.0.0.0:9090".parse().expect("valid addr");
        assert_eq!(
            ServerArgs::parse_from(["vpay-server"])
                .common
                .observability_bind,
            expected
        );
        assert_eq!(
            ServerArgs::parse_from(["vpay-server", "worker"])
                .common
                .observability_bind,
            expected
        );

        let args = ServerArgs::parse_from(["vpay-server", "--observability-bind", "127.0.0.1:0"]);
        assert_eq!(
            args.common.observability_bind,
            "127.0.0.1:0".parse::<SocketAddr>().expect("valid addr"),
            "port 0 is a real configuration — every subprocess test binds it"
        );
    }

    /// The two listeners must be separately addressable: `/metrics` names
    /// every rail, route and error code this deployment has, and the
    /// NetworkPolicy that keeps it off the Ingress can only exist because it
    /// is on a different port from `--bind`.
    #[test]
    fn the_observability_bind_is_not_the_same_knob_as_the_traffic_bind() {
        let args = ServerArgs::parse_from([
            "vpay-server",
            "--bind",
            "127.0.0.1:18080",
            "--observability-bind",
            "127.0.0.1:19090",
        ]);
        assert_eq!(
            args.bind,
            "127.0.0.1:18080".parse::<SocketAddr>().expect("valid addr")
        );
        assert_eq!(
            args.common.observability_bind,
            "127.0.0.1:19090".parse::<SocketAddr>().expect("valid addr")
        );
        assert_ne!(args.bind, args.common.observability_bind);
    }

    /// The `worker` subcommand takes no `--bind` of its own — it serves no
    /// traffic — so `--observability-bind` is the only socket it opens. If
    /// someone ever moves `bind` into `CommonArgs`, this fails before
    /// anything else notices the worker started answering `/v1`.
    ///
    /// `--bind` is not `global`, so this is a parse error rather than an
    /// ignored flag. `vpay-server --bind … worker` is refused too, one layer
    /// out — see
    /// `a_serve_only_flag_written_before_the_worker_subcommand_is_refused`,
    /// which covers both flags in both positions.
    #[test]
    fn the_worker_subcommand_binds_only_the_observability_listener() {
        let worker = worker_subcommand();
        assert!(
            !worker
                .get_arguments()
                .any(|arg| arg.get_id().as_str() == "bind"),
            "the worker serves no traffic; a --bind there would open a port nothing routes"
        );
        assert!(
            ServerArgs::try_parse_from(["vpay-server", "worker", "--bind", "0.0.0.0:8080"])
                .is_err(),
            "`vpay-server worker --bind` must be a parse error"
        );
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

    /// Same check through the whole `ServerArgs` — the type actually likely
    /// to be logged at startup (see the doc comment on `CommonArgs`) —
    /// proving the derive-delegates-to-nested-`Debug` composition holds for
    /// the top-level parser and not just for `CommonArgs` in isolation, in
    /// **both** modes: `worker` reaches the same fields through a `global`
    /// propagation rather than through its own parser, so it is asserted
    /// rather than assumed.
    #[test]
    fn server_and_worker_args_debug_output_never_contains_the_database_password() {
        let serve = ServerArgs::parse_from([
            "vpay-server",
            "--database-url",
            "postgres://vpay:hunter2-live-password@db.internal:5432/vpay",
        ]);
        let worker = ServerArgs::parse_from([
            "vpay-server",
            "worker",
            "--database-url",
            "postgres://vpay:hunter2-live-password@db.internal:5432/vpay",
        ]);

        let serve_formatted = format!("{serve:?}");
        let worker_formatted = format!("{worker:?}");

        assert!(!serve_formatted.contains("hunter2-live-password"));
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
