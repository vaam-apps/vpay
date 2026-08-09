# Configuration

Administration is YAML in git (ADR-0003). The dashboard cannot change any of it.

## What exists today: the CLI / env layer

Before any of the YAML system below is real, both binaries already parse a
CLI (`vpay-config::cli`, `clap`) where every option auto-resolves from an
environment variable, with an explicit flag beating its env var. The two
binaries share one `CommonArgs` (`#[command(flatten)]`), so they cannot drift
on a flag's name, env var, or default.

| Flag | Env var | Default |
|---|---|---|
| `--bind` (`vpay-server` only) | `VPAY_BIND` | `0.0.0.0:8080` |
| `--database-url` | `DATABASE_URL` | none |
| `--profile` | `VPAY_PROFILE` | `sandbox` |
| `--config` | `VPAY_CONFIG` | none |
| `--public-base-url` (`vpay-server` only) | `VPAY_PUBLIC_BASE_URL` | none |
| `--log-filter` | `RUST_LOG` | `info` |
| `--log-format` (`json`\|`text`) | `VPAY_LOG_FORMAT` | `json` |
| `--shutdown-grace-seconds` | `VPAY_SHUTDOWN_GRACE_SECONDS` | `25` |

`--version` reports the workspace version (`0.1.0`). Run
`cargo run -p vpay-server -- --help` to see the live flag set — that is more
trustworthy than this table if the two ever disagree.

**This is CLI/env plumbing, not the boot sequence below.** `--database-url`,
`--config` and `--public-base-url` are accepted and parsed, but nothing reads
them yet: no database connection is opened, no YAML file is loaded, no
redirect/webhook URL is constructed from the base URL. `--profile` only ever
selects a config *file name*, per the "no environment branching" rule; it is
never matched on to change behaviour.

`--shutdown-grace-seconds` is a partial exception: `vpay-server` actually uses
it to bound how long it waits for in-flight requests to drain after a
shutdown signal, via `serve_with_bounded_drain` in
`backends/apps/vpay-server/src/main.rs` — it races the drain against a clock
of that length and exits non-zero if the clock wins. `vpay-worker-bin` accepts
and logs the same flag but does nothing with it; there is no drain to bound
because there is no job loop yet. Neither binary's handling of the *timeout*
case is covered by a test today — see [../status.md](../status.md).

## There is no sandbox mode

Two statements that look contradictory and are not:

- **A sandbox *environment* — yes.** Two deployments: one talking to rail
  sandboxes and WireMock, one talking to real rails. Each has its own config
  file and its own database.
- **A sandbox *mode* — no.** No `if (sandbox)`, no code path that exists only
  outside production, no bean wired differently.

A profile selects a **configuration file**. It must never select a **code path**.
Same binary, same image digest, different YAML and different database.

Because Spring Boot is the idiom being borrowed, the trap it makes easy is worth
naming: `@Profile("!prod")`, `@ConditionalOnProperty` on business logic and
profile-specific bean overrides are all `if (sandbox)` wearing a
dependency-injection costume. Profiles may select *values*; never *beans that
behave differently*.

## Boot sequence

**Steps 1–4 below are not implemented.** Today, `vpay-server` parses its CLI
(the table above), binds the port and serves `/healthz` — that is the whole
boot sequence that actually exists. Nothing in this repo loads a YAML file,
resolves a `${}` placeholder, runs the validation rules below, or reconciles
anything into a database, because there is no database layer yet
(`docs/status.md`). The steps below describe the design this repo is building
towards, not current behaviour.

1. Load `application.yml`, overlay `application-{profile}.yml`.
2. Resolve `${}` placeholders. **An unresolved placeholder is fatal**, never an
   empty string — an empty subscription key otherwise fails much later and much
   more confusingly.
3. Validate (below).
4. Reconcile into the database in **one transaction**; record the config hash.
5. Only then bind the port.

**A validation failure exits non-zero without serving traffic.** A payment
gateway that boots half-configured is worse than one that does not boot.

## Rules that refuse to boot

| Rule | Why |
|---|---|
| Every merchant's rail host appears in that rail's allowlist | The host allowlist, checked before the FK |
| Every referenced provider exists and is enabled | A typo fails at boot, not at first payment |
| Currency exponent matches the canonical table | A 100× amount bug is otherwise silent |
| `livemode` ⇒ every host is `https://` | |
| `livemode` ⇒ no host labelled `wiremock`/`stub`/`mock`/`localhost` | **The most valuable rule here.** It is what makes "the code cannot tell a stub from a real rail" safe to live with |
| `livemode` ⇒ secrets come from `${}`, not literals | Stops a real key reaching git |
| `partial-refunds` ⇒ `refunds` | Enforced in Rust today, **not** by a database constraint — see below |

The three `livemode` rules — `https`-only, no stub-labelled host, and
`${}`-only secrets — are implemented and tested in `vpay-config`
(`validate_host`, `validate_secret`). **The `partial-refunds ⇒ refunds` row is
not a `vpay-config` boot guard at all**, despite living in this table; see the
correction below for where it actually lives.

**Correction:** this row previously said the rule "mirrors the DB CHECK." That
was false on two counts. First, there is no database schema in this repo yet,
and even once `schemas/vpay.cstack` is wired in, CrateStack's grammar has no
way to express it as a constraint: `@db_enforce` only promotes a single-field
`@range`/`@length`/`@iso4217` validator to a column-level CHECK, and there is
no `@@check(expr)` or other cross-column boolean constraint (see the `GAP`
comment on `Provider` in `schemas/vpay.cstack`). Second, this rule is not a
boot-time config guard at all — it is enforced by types, not deployment
config: `Capabilities::is_coherent` in
`backends/crates/vpay-provider/src/lib.rs` requires
`supports_partial_refunds ⇒ supports_refunds` on every adapter's static
capability declaration, tested by
`vpay-provider::tests::partial_refunds_imply_refunds` and by the conformance
suite's `every_adapter_declares_coherent_capabilities`. It has nothing to do
with `vpay-config` or a deployment's YAML.

## Config changes and in-flight payments

**Safe to mutate:** credentials (rotation works on in-flight transactions),
prompt TTL, rate limits, webhook endpoints, capability flags.

**Identity-defining, refused while any non-terminal charge references the
config:** host, currency, payee identifier, or the merchant/provider pairing. A
charge submitted to host A must be *polled* at host A; silently repointing it
means recovery asks the wrong server and gets `NotFound` forever.

## Status

The CLI/env layer (`vpay-config::cli`) is implemented and tested — flag
parsing, env-var resolution, flag-beats-env precedence, and shared options
between binaries. The config guard rules (stub-host detection, literal
secrets, `partial-refunds ⇒ refunds`) are implemented and tested in Rust.

`--shutdown-grace-seconds` bounds `vpay-server`'s shutdown drain; it is
accepted but inert on `vpay-worker-bin`.

**Not started:** everything else in the "Boot sequence" above — YAML loading,
`${}` placeholder resolution, validation wired into boot, and database
reconciliation. `--database-url` and `--config` are accepted CLI/env inputs
with nothing behind them yet. See [../status.md](../status.md).
