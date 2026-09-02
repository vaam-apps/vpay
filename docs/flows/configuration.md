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
| `--oauth-signing-key-file` (`vpay-server` only) | `VPAY_OAUTH_SIGNING_KEY_FILE` | none |
| `--log-filter` | `RUST_LOG` | `info` |
| `--log-format` (`json`\|`text`) | `VPAY_LOG_FORMAT` | `json` |
| `--shutdown-grace-seconds` | `VPAY_SHUTDOWN_GRACE_SECONDS` | `25` |

`--version` reports the workspace version (`0.1.0`). Run
`cargo run -p vpay-server -- --help` to see the live flag set — that is more
trustworthy than this table if the two ever disagree.

`--oauth-signing-key-file` names the RS256 private key (PKCS#8 PEM) the
merchant OP signs `/v1` access tokens with. It is `vpay-server` only — the
worker issues no tokens, so mounting the Secret into it would widen its
blast radius for no capability, and `the_worker_is_not_handed_the_signing_key`
pins that. It is a **file**, never an env value, because that is how a
Kubernetes Secret reaches a pod; `cargo xtask gen-signing-key --out <dir>`
generates one, and `just gen-e2e-signing-key` does the openssl equivalent
for the compose stack. The *path* is deliberately visible in `Debug` output
(`the_signing_key_path_stays_visible_in_debug_output`) — a path is not a
secret, and "which file did it try" is the first thing an operator needs —
while the file's contents never enter the CLI types at all.

**This is CLI/env plumbing, not the boot sequence below**, and one flag is
still pure plumbing: **`--public-base-url` is accepted and parsed and read
by nothing.** This is easy to get wrong now that `/v1/oauth` publishes an
issuer, so to be exact — the issuer is
`vpay_api::op::issuer_for(&config)`, which reads
**`deployment.public_base_url` from the YAML config file**, not this flag.
Two spellings of the same idea, one of them inert. `--profile` only ever
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

**Steps 1–3 are implemented and wired into both binaries; step 4 is not.**
`vpay_config::Config::load` implements the YAML layering, the `${}`
resolution and the validation rules below, and both `vpay-server` and
`vpay-worker-bin` call it before opening a database connection — a missing
or invalid `--config` / `VPAY_CONFIG` is exit 78 (proven by subprocess tests
in each binary's `tests/cli.rs`; see `docs/status.md`, "YAML config
loading"). *This paragraph said "neither binary calls it" until 2026-09-02;
that had been false since 2026-08-11.* The deployment consequence is real:
`backends/Dockerfile` bakes `config/` into the image and sets `VPAY_CONFIG`,
and `compose.e2e.yml` supplies every `${VAR}` the file names, because a
process without them does not start.
Step 4 has no implementation at all: nothing reconciles configuration into
the database or records a config hash, even though a database layer now
exists (`vpay-db`). Two of the validation rules below are also unimplemented
for a structural reason — see the table. See `docs/status.md` for the
authoritative state.

**`vpay-server`'s actual startup order, as of 2026-09-02**, which is the
"cheapest hard failure first" ordering this document's own steps imply:

1. Install the SIGINT/SIGTERM handlers and the rustls crypto provider.
2. Load and validate the YAML config (steps 1–3 above). Missing or invalid
   → exit `78`, before any network round trip.
3. **Load the RS256 signing key** from `--oauth-signing-key-file` /
   `VPAY_OAUTH_SIGNING_KEY_FILE`, and derive the issuer from
   `deployment.public_base_url` so the key stamps the same `iss` the OP
   advertises. A missing flag, a missing file, a file that is not an RSA
   private key, or a key under 2048 bits each exit `78` — **before the
   database connection**, which is why all three cases are covered by
   subprocess tests that need no Docker
   (`a_missing_signing_key_flag_is_exit_78_naming_the_problem`,
   `a_signing_key_file_that_does_not_exist_is_exit_78_naming_the_path`,
   `a_signing_key_file_that_is_not_a_key_is_exit_78_without_echoing_its_contents`).
   A server that cannot sign can mint no merchant token; it would bind a
   port, answer `/healthz` with a cheerful 200, and refuse every real
   request.
4. Connect to Postgres and run migrations. Unreachable → exit `69`.
5. Announce the key as active in `oauth_signing_keys`
   (`ensure_active_signing_key`, advisory-locked). Fatal on failure: a
   process whose key is not published mints tokens nothing can verify.
6. Sweep expired client-assertion `jti`s once — a boot-time stopgap,
   non-fatal, because there is no worker job loop to schedule it properly.
7. Bind the listener, **then** build the token validator, because it needs
   the port actually bound (`--bind 127.0.0.1:0` is a real configuration)
   and validates over loopback against this process's own
   `/v1/oauth/jwks.json`.
8. Serve.

**A pre-existing gap this ordering does not fix:** a missing
`--database-url` still exits `1`, not `78`, because `main` produces a bare
`anyhow` error there with nothing for the exit-code classifier to read. The
`StartupError` added for the signing key covers only the signing key.

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
| `partial-refunds` ⇒ `refunds` | Enforced both in Rust and by a database CHECK constraint — see below |

The three `livemode` rules — `https`-only, no stub-labelled host, and
`${}`-only secrets — are implemented and tested in `vpay-config`
(`validate_host`, `validate_secret`). **The `partial-refunds ⇒ refunds` row is
not a `vpay-config` boot guard at all**, despite living in this table; see the
correction below for where it actually lives.

**Correction of the correction:** an earlier pass through this doc said the
rule "mirrors the DB CHECK" was false, because at the time there was no
database schema in this repo and `schemas/vpay.cstack`'s CrateStack grammar
has no way to express a cross-column constraint (`@db_enforce` only promotes
a single-field `@range`/`@length`/`@iso4217` validator to a column-level
CHECK; there is no `@@check(expr)`; see the `GAP` comment on `Provider` in
`schemas/vpay.cstack`). That was true of the `.cstack` grammar specifically,
but the database schema has since been implemented in raw SQL, which has no
such limitation. `backends/migrations/0002_create-providers.sql:37-38` now
declares `CONSTRAINT partial_refunds_imply_refunds CHECK (NOT
supports_partial_refunds OR supports_refunds)` on the `providers` table,
proven to fire by
`partial_refunds_without_refunds_is_rejected_by_the_database` in
`backends/tests/integration/tests/postgres_smoke.rs` (against a real
Postgres 16 via testcontainers). So the original "mirrors the DB CHECK"
framing was right after all — it just could not have been built through
`schemas/vpay.cstack`.

What has not changed: this is still not a `vpay-config` boot-time guard. It
is enforced twice, independently — belt and braces, not one mechanism
standing in for the other:

- **In Rust**, on every adapter's static capability declaration:
  `Capabilities::is_coherent` in
  `backends/crates/vpay-provider/src/lib.rs` requires
  `supports_partial_refunds ⇒ supports_refunds`, tested by
  `vpay-provider::tests::partial_refunds_imply_refunds` and by the
  conformance suite's `every_adapter_declares_coherent_capabilities`.
- **In the database**, on the `providers` table itself, as above.

Neither has anything to do with `vpay-config` or a deployment's YAML — there
is still no YAML-loading or reconciliation code in this repo (see the boot
sequence section above).

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
secrets, `partial-refunds ⇒ refunds`) are implemented and tested in Rust;
`partial-refunds ⇒ refunds` is additionally enforced by a database CHECK
constraint (`backends/migrations/0002_create-providers.sql`), tested against
a real Postgres — see the correction above.

`--shutdown-grace-seconds` bounds `vpay-server`'s shutdown drain; it is
accepted but inert on `vpay-worker-bin`.

YAML loading, `${}` placeholder resolution and validation are implemented
(`vpay_config::Config::load`) and wired into both binaries' boot as a hard
requirement (steps 1–3 above; **53 tests in `vpay-config`** as of 2026-09-02
— 25 in `config`, 18 in `cli`, 5 in `oauth`, 5 crate-level — plus subprocess
tests in each binary). `--database-url` is likewise required at runtime and opens
a real pool. *Updated 2026-09-02 — this section had said all of that was
"not started".*

**New 2026-09-02:** `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`
on `vpay-server`, required at runtime and checked *before* the database
connection, so its three failure modes exit `78` and are covered by
subprocess tests that need no Docker (named in the boot sequence above).
Eighteen of `vpay-config`'s 53 tests are in its `cli` module.

**Also new 2026-09-02, and a boot rule rather than a flag:**
`ConfigError::MerchantMissingV1Audience` refuses to start a deployment whose
merchant registration cannot target `vpay_config::MERCHANT_AUDIENCE`
(`vpay:v1`) — because neither runtime symptom names the cause. The fixture
that proves it (`a_merchant_client_that_cannot_target_the_v1_audience_is_rejected`)
is verbatim what `config/application.yml` shipped until that day, and
`the_example_config_registers_its_merchant_for_the_v1_audience` asserts the
real file satisfies the rule by carrying the *constant*, not a second copy
of the spelling.

**Not started:** step 4 — reconciling configuration into the database in one
transaction and recording a config hash — and the two boot-guard rules that
need a payment-routing `merchants` concept ("every merchant's rail host is in
the allowlist", "every referenced provider exists and is enabled").
`--public-base-url` remains accepted, parsed and read by nothing. See
[../status.md](../status.md).
