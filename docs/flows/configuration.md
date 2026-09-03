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
| `--observability-bind` | `VPAY_OBSERVABILITY_BIND` | `0.0.0.0:9090` |
| `--oauth-signing-key-file` (`vpay-server` only) | `VPAY_OAUTH_SIGNING_KEY_FILE` | none |
| `--log-filter` | `RUST_LOG` | `info` |
| `--log-format` (`json`\|`text`) | `VPAY_LOG_FORMAT` | `json` |
| `--shutdown-grace-seconds` | `VPAY_SHUTDOWN_GRACE_SECONDS` | `25` |

`--observability-bind` is on **both** binaries — the worker had no HTTP
listener at all before it — and serves exactly two paths, `GET /livez` and
`GET /metrics`, from a second socket. It is never the `--bind` port: that one
is fronted by an Ingress, and `/metrics` names every rail this deployment
talks to, every route pattern it serves and every error code it has produced.
The chart's NetworkPolicy admits 9090 from the monitoring namespace only, and
it can express that *because* the two are different ports.

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

**This is CLI/env plumbing, not the boot sequence below.** Every flag in the
table is now consumed by something; the one that was not — `--public-base-url`
— was **removed on 2026-09-03** (step-6 decision (7)) rather than wired.
It had been accepted and parsed and read by nothing since it was added, which
is easy to get wrong now that `/v1/oauth` publishes an issuer: the issuer is
`vpay_api::op::issuer_for(&config)`, which reads
**`deployment.public_base_url` from the YAML config file**, and that is
unchanged. What went away is the *flag* and its `VPAY_PUBLIC_BASE_URL`
variable — the second, inert spelling of one idea.

The two halves of that removal behave differently, and the difference matters
to whoever upgrades (`backends/crates/vpay-config/src/cli.rs`, the
`--public-base-url` is gone section):

- Passing **`--public-base-url`** now fails at parse time, loudly, on an
  unknown argument. A chart or compose file that still sets the flag breaks on
  upgrade — the honest cost of removing an interface.
- Setting **`VPAY_PUBLIC_BASE_URL`** does **not** fail. clap reads an
  environment variable only for a flag it declares, so a stale variable in a
  Secret or a compose file is silently ignored: no error, no effect. That is
  the same thing it did before the removal (it was inert then too), so nothing
  breaks — but nothing tells anyone either. Nothing in this repository sets it
  (`.env.example` dropped its row in the same change).
`--profile` only ever selects a config *file name*, per the "no environment
branching" rule; it is never matched on to change behaviour.

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

**Steps 1–4 are implemented and wired into both binaries; the config hash
half of step 4 is not.** *Updated 2026-09-03 (Step 2); this paragraph said
"step 4 is not" until then.*
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
**Step 4 now exists** (`vpay_db::ConfigReconcile::reconcile`, 2026-09-03):
both binaries make `currencies` and `providers` match the deployment's own
configuration in **one transaction**, opened by taking the
`pg_advisory_xact_lock` `lock_keys::CONFIG_RECONCILE` so that N replicas and
both binaries booting at once cannot interleave. Configuration is the
authority and the tables are the mirror: a rail present in the seed is
upserted, and a rail **absent** from it is set `enabled = false` rather than
deleted, because a rail that has ever taken money must stay nameable
forever. **The config hash half of step 4 is still not implemented** —
nothing records or compares one. See `docs/status.md` for the authoritative
state.

**What the seeds are joined against.** `vpay_api::v1::boot::boot_seeds` is
the single derivation both binaries call: it walks the YAML's `providers`
and, for each, looks up a *linked adapter* to take `flow`,
`supports_refunds`, `supports_partial_refunds`, `delivers_callbacks` and
`requires_ip_allowlist` from — capabilities come from the adapter, the
`enabled` flag from the YAML
(`boot_seeds_joins_the_yaml_against_the_linked_adapters`,
`a_disabled_rail_is_seeded_disabled_rather_than_omitted`). **A YAML provider
code with no linked adapter is `ConfigError::ProviderWithoutAdapter` and
exit `78`**, before the port is bound
(`a_configured_rail_with_no_linked_adapter_is_a_named_config_error` as a unit
test, and `a_provider_code_with_no_linked_adapter_is_exit_78` in
`backends/apps/vpay-server/tests/cli.rs` as a subprocess test against a
fixture config; `the_repositorys_own_configuration_passes_the_adapter_join`
asserts the real `config/application.yml` satisfies it).

`providers.display_name` is **derived from the code**
(`display_name_for`: `mtn_momo` → `Mtn Momo`) rather than configured. The
provider port has no `display_name()`, so this is a placeholder that is
honest about being one — `a_display_name_is_derived_from_the_code_without_panicking`
pins it, empty string included.

**`vpay-server`'s actual startup order, as of 2026-09-03**, which is the
"cheapest hard failure first" ordering this document's own steps imply:

1. Install the SIGINT/SIGTERM handlers and the rustls crypto provider.
2. Load and validate the YAML config (steps 1–3 above). Missing or invalid
   → exit `78`, before any network round trip.
2b. Link the adapters this binary was built with and **join the YAML's rails
   against them** (`adapters_by_code` + `boot_seeds`). A configured rail with
   no adapter → exit `78`, still before any network round trip.
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
5. **Reconcile `currencies` and `providers` from configuration**
   (`vpay_db::ConfigReconcile::reconcile`, one transaction, advisory-locked
   — step 4 above). Fatal on failure. The seeds were derived back at step 2b
   (`boot_seeds`, which is why a rail with no adapter exits `78` before
   Postgres is even contacted). `vpay-worker-bin` does the same thing, from
   the same function, at the same point in its own boot.
6. Announce the key as active in `oauth_signing_keys`
   (`ensure_active_signing_key`, advisory-locked). Fatal on failure: a
   process whose key is not published mints tokens nothing can verify.
7. Sweep expired client-assertion `jti`s **and expired `idempotency_keys`**
   once — boot-time stopgaps, both non-fatal, because there is no worker job
   loop to schedule either properly. `vpay-worker-bin` sweeps nothing.
8. Bind the listener, **then** build the token validator, because it needs
   the port actually bound (`--bind 127.0.0.1:0` is a real configuration)
   and validates over loopback against this process's own
   `/v1/oauth/jwks.json`.
9. Serve.

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
| Every merchant registration carries a unique `merchant_id` | The `/v1` tenancy boundary has no foreign key behind it |
| Currency exponent matches the canonical table | A 100× amount bug is otherwise silent |
| `livemode` ⇒ every host is `https://` | |
| `livemode` ⇒ no host labelled `wiremock`/`stub`/`mock`/`localhost` | **The most valuable rule here.** It is what makes "the code cannot tell a stub from a real rail" safe to live with |
| `livemode` ⇒ secrets come from `${}`, not literals | Stops a real key reaching git |
| `partial-refunds` ⇒ `refunds` | Enforced both in Rust and by a database CHECK constraint — see below |

The three `livemode` rules — `https`-only, no stub-labelled host, and
`${}`-only secrets — are implemented and tested in `vpay-config`
(`validate_host`, `validate_secret`).

**The `merchant_id` row is new on 2026-09-03 and is fully enforced.**
`MerchantClient::merchant_id` is required (no default to `client_id`: a
config that forgot it would otherwise boot and silently invent a tenancy
boundary) and unique across `merchant_clients`
(`ConfigError::DuplicateMerchantId`). Proven by
`a_merchant_client_without_a_merchant_id_does_not_load` and
`two_merchant_clients_sharing_a_merchant_id_are_rejected` in `vpay-config`,
with `oauth-duplicate-merchant-id.yml` as the fixture. It is separate from
`client_id` deliberately — a credential may be rotated, a tenant may not —
and it is what every `/v1` query filters by, because there is no `merchants`
table and therefore no foreign key to catch a query that forgot.

**`ProviderHost.enabled` is also new**: absent means enabled
(`a_provider_with_no_enabled_line_is_enabled`), and `enabled: false` keeps
the rail configured while `reconcile` writes `enabled = false` into
`providers` (`an_explicitly_disabled_provider_stays_disabled`). A disabled
rail cannot be named on a new intent or on a confirm
(`a_disabled_rail_is_configured_but_not_offered`,
`a_disabled_rail_cannot_be_named_on_a_new_intent`).

**The "every referenced provider exists and is enabled" row, exactly.** Half
of it is now a boot rule and half of it is not, and conflating the two would
overstate what boots safely. What *is* enforced at boot is the join above: a
rail named in the YAML with no linked adapter exits `78`. What is **not** a
boot rule is the original intent of this row — a *merchant*'s reference to a
rail — because there is still no merchant→rail routing concept in this
config shape; an OAuth `MerchantClient` names no rails. A payment intent's
rails are instead checked per request, against the deployment's enabled set,
and answered as a `400` naming `payment_method_types`.

**The `partial-refunds ⇒ refunds` row is
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

Neither has anything to do with `vpay-config` or a deployment's YAML. *That
sentence used to end "there is still no YAML-loading or reconciliation code
in this repo", which stopped being true for loading on 2026-09-02 and for
reconciliation on 2026-09-03 — see the boot sequence above. The
`partial_refunds_imply_refunds` CHECK is now reachable from `reconcile`
itself: a seed setting `supports_partial_refunds` without `supports_refunds`
is a `DbError::Query` that rolls the whole reconcile back.*

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
requirement (steps 1–3 above; **57 tests in `vpay-config`** as of 2026-09-03
— 29 in `config`, 18 in `cli`, 5 in `oauth`, 5 crate-level; it was 53 on
2026-09-02, and the four new ones are the `merchant_id` and `enabled` rules
described below — plus subprocess tests in each binary). `--database-url` is likewise required at runtime and opens
a real pool. *Updated 2026-09-02 — this section had said all of that was
"not started".*

**New 2026-09-02:** `--oauth-signing-key-file` / `VPAY_OAUTH_SIGNING_KEY_FILE`
on `vpay-server`, required at runtime and checked *before* the database
connection, so its three failure modes exit `78` and are covered by
subprocess tests that need no Docker (named in the boot sequence above).
Eighteen of `vpay-config`'s 57 tests are in its `cli` module.

**Also new 2026-09-02, and a boot rule rather than a flag:**
`ConfigError::MerchantMissingV1Audience` refuses to start a deployment whose
merchant registration cannot target `vpay_config::MERCHANT_AUDIENCE`
(`vpay:v1`) — because neither runtime symptom names the cause. The fixture
that proves it (`a_merchant_client_that_cannot_target_the_v1_audience_is_rejected`)
is verbatim what `config/application.yml` shipped until that day, and
`the_example_config_registers_its_merchant_for_the_v1_audience` asserts the
real file satisfies the rule by carrying the *constant*, not a second copy
of the spelling.

**New 2026-09-03 (Step 2), and the reason this section's "Not started" list
shrank:** boot step 4's reconciliation half is implemented
(`vpay_db::ConfigReconcile::reconcile`, one advisory-locked transaction,
called by both binaries), the YAML↔adapter join that feeds it is implemented
(`vpay_api::v1::boot::boot_seeds`, exit `78` for a rail with no adapter), and
`merchant_clients` gained a required, unique `merchant_id` while `providers`
gained `enabled`. `vpay-config` now runs **57 tests** (up from 53), measured
by `cargo nextest run -p vpay-config` on 2026-09-03. The reconcile itself is
proven against a real Postgres by
`reconcile_is_idempotent_and_disables_a_dropped_provider_code`,
`reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` and
`two_concurrent_reconciles_with_the_seeds_in_opposite_orders_both_succeed_and_converge`
in `backends/crates/vpay-db/tests/repositories.rs` — 74 container-backed
tests passed on this machine that day.

**Still not started:** the **config hash** half of step 4 — nothing records
or compares one, so nothing detects a replica booted from a different config
file; the two boot-guard rules that need a payment-routing `merchants`
concept ("every merchant's rail host is in the allowlist", and the
merchant-facing half of "every referenced provider exists and is enabled" —
see that row above for what *is* enforced); a `display_name` that is a real
port capability rather than a derivation of the code; and any hot reload —
a config change is still a redeploy. (`--public-base-url`, which this
paragraph used to list here as still-inert, was removed on 2026-09-03 — see
the flag table above.) See [../status.md](../status.md).

**New 2026-09-03 (Step 3), and one of these is a bug this pass found rather
than a feature it added:**

- **Livemode had never been bootable.** `validate_secret`'s rule is "a
  credential must be *written* as a `${VAR}` placeholder, not as a literal"
  — a question about step 1's text — and it was being asked of step 2's
  *resolved* values, where a correctly written `${MTN_API_KEY}` and a
  literal `hunter2` are the same string. So the rule enforced nothing and
  refused every correct livemode config. The pre-resolution text of each
  `providers[].credentials` value is now captured before resolution and
  checked against that (`RawProviderSecrets`, private to `vpay_config::config`).
  The two rules now answer the two different questions they were always
  meant to: *was it written as a reference* and *did the reference resolve*
  (`a_livemode_config_with_a_literal_secret_is_rejected`,
  `a_livemode_config_whose_placeholders_resolve_loads`,
  `a_livemode_placeholder_that_does_not_resolve_is_still_the_unresolved_error`,
  `a_sandbox_config_with_a_literal_secret_loads`).
- **`providers[].currency`** is required and must be one this deployment's
  currency table knows (`a_rail_currency_outside_the_canonical_table_is_rejected`).
  It is checked at boot, not only when a request builds a `ProviderConfig`,
  so an unknown code is a refusal to start rather than a 500 on a merchant's
  confirm.
- **`providers[].callback_url`** is optional and defaults to
  `{public_base_url}/provider/{code}/callback`. The *effective* value — not
  just an override — is put through `validate_host`, so a livemode
  deployment cannot hand a live rail a plaintext or stub callback host
  (`a_livemode_callback_url_that_is_not_https_is_rejected`,
  `a_livemode_deployment_cannot_derive_a_plaintext_callback_url`,
  `a_derived_callback_url_survives_a_trailing_slash_and_an_override_wins`).
- **`REQUIRED_RAIL_KEYS`** refuses to boot a rail missing a key its adapter
  cannot work without — MTN: `settings.target_environment`,
  `settings.api_user`, `credentials.subscription_key`, `credentials.api_key`;
  Orange: `credentials.merchant_key`, `credentials.client_id`,
  `credentials.client_secret`. A present-but-empty value counts as missing.
  This is a provider-code match outside an adapter crate, which ADR-0002
  forbids; it is a deliberate, recorded interim —
  [ADR-0012](../adr/0012-rail-configuration-requirements-in-config.md) — and
  it moves behind the port the day the port grows a `required_settings()`
  hook. Nothing here selects *behaviour*; it selects a refusal to start
  (`a_rail_missing_a_required_setting_is_rejected`,
  `a_rail_missing_a_required_credential_is_rejected`,
  `a_required_key_present_but_empty_is_treated_as_missing`,
  `a_rail_this_crate_has_no_key_table_for_is_not_refused_here`).
- **`ProviderHost::to_provider_config(&Deployment)`** is the only place a
  `vpay_provider::ProviderConfig` is built from configuration, so the server
  and the worker cannot disagree about a rail's callback URL or currency
  (`to_provider_config_projects_the_example_config_onto_the_port`, which
  compares the whole projected value rather than field by field, and
  `to_provider_config_names_a_currency_it_cannot_parse`). The two timeouts
  are constants, not YAML knobs: no deployment has asked for a different
  budget and a knob nobody sets is a knob nobody has tested.
- **Seven environment variables** are now referenced by
  `config/application.yml` and must be supplied by every deployment. Six are
  rail credentials — `MTN_SUBSCRIPTION_KEY`, `MTN_API_KEY`, `MTN_API_USER`,
  `ORANGE_MERCHANT_KEY`, `ORANGE_CLIENT_ID`, `ORANGE_CLIENT_SECRET` — and the
  seventh, added 2026-09-03 with Step 5, is **`MERCHANT_WEBHOOK_SECRET`**, the
  `${VAR}` behind `merchant_clients[].webhooks[].secrets`. All seven are set on
  **both** services in `compose.e2e.yml` (which `compose.demo.yml` layers on
  top of, inheriting them) and listed in `.env.example`; miss one and the
  process exits `78` at boot, by design — **including `vpay-server`**, which
  never delivers a webhook but loads and validates the same document, which is
  why `backends/apps/vpay-server/tests/cli.rs` had to start setting it.
- `vpay-config` now runs **77 tests, 77 passed, 0 skipped**
  (`cargo nextest run -p vpay-config`, re-measured 2026-09-03 on
  `claude/step5-webhooks`; it was 57 before the Step 3 pass, and the "70" this
  line claimed until Step 5 was Step 3's number left stale). Four of the 77 are
  the webhook-endpoint cases — `every_webhook_endpoint_rule_refuses_its_own_fixture`
  (one fixture per rule), `a_merchant_with_no_webhooks_configured_is_valid`,
  `a_livemode_webhook_secret_written_as_a_placeholder_loads_and_carries_the_resolved_value`
  and `a_webhook_endpoints_debug_output_never_contains_a_secret`.
