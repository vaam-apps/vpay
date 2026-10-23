# 2026-10-22 — vpay registers Notch Pay's callback endpoint itself (RFC-0013 steps 2–4)

Branch `claude/happy-hopper-r3h6rz`, from `master` at `e8e70ef` (#338).
RFC-0013 delivery steps 2 and 3, and step 4 for Notch Pay without the `PUT`
and `DELETE` it names. **Proven against a WireMock Notch Pay and a real
Postgres. No call was made to Notch Pay, and no endpoint has been registered
with a real account.**

## Whose decision

The maintainer, in the running session's chat: "the webhook registration
should be done automatically then, e.g via cli or at startup fo the providers
that supports it", then "Yes, start it". That answers RFC-0013 open question
1 as `ensure`. "At startup" is read as RFC-0013 § 2 reads it: the job is
seeded at every **worker** start and is due at once. `vpay-server`'s serve
boot asks no rail anything, because a boot that waits on a provider is a
deployment the provider can take down.

## What was built

| Piece                                                                                                                        | Where                                                                                                             |
| ---------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `CallbackUrlMode`, `Capabilities::callback_url_mode`, `callbacks_are_coherent`                                               | `vpay-provider/src/lib.rs`                                                                                        |
| `ProviderAdapter::ensure_callback_endpoint`, defaulting to `Unsupported`; `EndpointCheck`, `EndpointState`, `EndpointReport` | `vpay-provider/src/lib.rs`; forwarded and counted by `measured.rs` under the operation `ensure_callback_endpoint` |
| `mtn_momo`, `orange_money`: `PerRequest`. `notchpay`: `Registered`, and the method                                           | the three adapters; Notch Pay's in `vpay-adapter-notchpay/src/callbacks.rs`                                       |
| `JobKind::EnsureCallbacks`, dedupe key `ensure:callbacks`, the sixth seed                                                    | `vpay-worker/src/jobs.rs`, `run_loop.rs`                                                                          |
| Migration `0051_jobs-ensure-callbacks.sql`, reopening `jobs.kind_is_known`                                                   | `backends/migrations/`, with its `MANIFEST.sha256` line                                                           |
| The plan, the pass, the handler                                                                                              | `vpay-worker/src/callbacks.rs`                                                                                    |
| The lock `CALLBACK_REGISTRATION` (`vpaycbk`), `TxRepositories::try_lock_callback_registration`                               | `vpay-db/src/lock_keys.rs`, `repository.rs`, `jobs.rs`                                                            |
| `providers[].callbacks.register` (`ensure` / `check` / `off`)                                                                | `vpay-config/src/config.rs`                                                                                       |
| `ConfigError::IncoherentCallbackMode`, `ConfigError::CallbacksOnPerRequestRail`, refused at boot                             | `vpay-config/src/lib.rs`, `vpay-api/src/v1/boot.rs`                                                               |
| `vpay-server callbacks ensure [--check]`                                                                                     | `vpay-config/src/cli.rs`, `vpay-server/src/callbacks.rs`, `main.rs`                                               |
| The plan projected from YAML at worker boot                                                                                  | `vpay-server/src/worker.rs`                                                                                       |
| `vpay_callback_endpoint_checks_total{provider,state}`                                                                        | `vpay-core/src/metrics.rs`                                                                                        |
| WireMock mappings for `GET`/`POST /webhooks`, with a scenario                                                                | `backends/tests/conformance/wiremock/notchpay/mappings/webhooks.json`                                             |
| `config/application-notchpay-sandbox.yml` sets `callbacks.register: check`                                                   | so runbook B2's worker lists and never creates on the real sandbox account                                        |

[RFC-0013](../../rfc/0013-vpay-registers-its-own-callbacks.md) § "As built"
lists every place this differs from the RFC, and why. The four that matter
most:

- **Notch Pay's endpoint is recognised by its description**, never by its
  URL: Notch Pay answered `"url": null` for every endpoint on 2026-10-21. The
  description is `vpay:{deployment.name}:notchpay:default url:{16 hex digits
of SHA-256 of the callback URL}`.
- **So an endpoint someone registered by hand at vpay's URL is invisible**,
  and vpay creates its own beside it. Callbacks then arrive twice, which costs
  a second pull-forward.
- **Nothing is ever modified or deleted on Notch Pay.** `DELETE` answered
  `404` on 2026-10-21 and the client has no `PUT`. A superseded or duplicate
  endpoint of vpay's is logged by id; a current one missing an event is
  `drifted`.
- **A counter, not RFC-0013's gauge**, because the singleton runs on
  whichever worker claims it.

## How it is proven

**The race, and the mutation.** `two_racing_passes_create_one_endpoint`
starts two passes together against a stub whose empty list answers after a
second, so both reach the list before either creates. It passes: one
`created`, the other `locked`, and **one** `POST /webhooks` in the stub's
journal. With the lock removed (the `try_lock_callback_registration` check
short-circuited to `false &&`), run by hand on 2026-10-22, it failed:

```text
test two_racing_passes_create_one_endpoint ... FAILED
assertion `left == right` failed: two racing passes, one create
  left: 2
 right: 1
```

The mutation was reverted before anything else ran.

**`backends/tests/integration/tests/callback_registration.rs`**, five cases,
each with its own Postgres and its own WireMock:

- `the_pass_creates_vpays_endpoint_once_and_then_finds_it`: `created`, and
  the journal's body carries the callback URL, the description and the five
  events; the second pass reads `present` by the description; one `POST`.
- `two_racing_passes_create_one_endpoint`: above.
- `check_never_writes_and_off_asks_nothing`: `check` from configuration and
  `--check` over `ensure` both answer `missing` with no `POST`; `off` makes
  no request at all. In every case `mtn_momo`, configured `ensure`, is never
  asked.
- `the_seeded_job_registers_the_endpoint_and_reschedules_hourly`: the job,
  seeded by `seed_singletons` and claimed by `run_once`, creates once and
  settles `Rescheduled(1h)`.
- `the_job_reschedules_after_a_refusal_and_sooner_after_silence`: a `401`
  is `ProviderAccountBlocked` and reschedules at the hour; an unreachable
  rail reschedules at five minutes; neither creates.

**Conformance**, over all three rails:
`the_submit_tells_the_rail_where_to_call_back` now asserts its census
against `callback_url_mode`, and
`the_endpoint_check_is_unsupported_exactly_on_a_per_request_rail` is new.

**Unit tests** for the adapter's decision (eleven, in `callbacks.rs`: the
empty account, found with a null URL, superseded, duplicate, another
deployment's marker, a marker that is only a prefix, foreign, drifted, a
create answer with no id, and the refusal mapping), the worker's pass
(`RailPass` states, exit status and retry delay), the plan, the boot refusals,
the YAML block, the metric vocabulary, the lock key's mnemonic, the job kind
against migration `0051`'s CHECK, and the linked adapters
(`every_registered_callback_url_belongs_to_a_rail_that_calls_back`,
`only_notchpay_registers_its_callback_url`).

**Existing suites that now see a sixth singleton**: `worker_recovery.rs`'s
`park_the_housekeeping_jobs` names it, and
`the_housekeeping_jobs_are_seeded_once_and_reschedule_themselves` expects
six; `postgres_smoke.rs` expects 51 migrations.

## Gates

Run on this branch with every file staged, from a Claude Code cloud session.
Postgres 16 and WireMock 3.9.2 ran in Docker. `cargo nextest` and `just` are
not installed here, so each recipe's commands ran directly, and `cargo test`
replaced nextest (see the integration note below).

| Check                                                                  | Result                                                                                                                                                                                             |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all --check`                                              | clean                                                                                                                                                                                              |
| `cargo clippy --workspace --all-targets -- -D warnings`                | clean                                                                                                                                                                                              |
| `cargo test --workspace --lib --bins` (integration, conformance out)   | **1579 passed, 0 failed, 0 ignored**                                                                                                                                                               |
| `cargo test --workspace --tests` (the same crates, their `tests/` too) | **1890 passed, 0 failed, 0 ignored**                                                                                                                                                               |
| `cargo test --doc --workspace`                                         | **131 passed, 0 failed, 1 ignored** (`sdks/rust/src/lib.rs` `ReadmeDoctests`, untouched)                                                                                                           |
| `cargo test -p vpay-tests-conformance`                                 | **97 passed, 0 failed, 0 ignored**                                                                                                                                                                 |
| `cargo test -p vpay-tests-integration`                                 | **390 passed, 5 failed, 0 ignored**: every failure passes in its own process (below)                                                                                                               |
| `cratestack check --schema schemas/vpay.cstack` (0.15.0)               | `schema OK` (the schema is untouched)                                                                                                                                                              |
| the 14 `cargo xtask` gates `verify` names                              | all ok: `verify-migrations` 51 files, `verify-doc-counts` 13 counts, `verify-links` 2672 links                                                                                                     |
| `cargo xtask verify-citations`                                         | ok                                                                                                                                                                                                 |
| `verify-ui`                                                            | not run: it greps `frontends/` and `examples/`, which this change does not touch                                                                                                                   |
| `verify-ignored` (a CI step, not one `verify` names)                   | **failed in CI on `f25930e5`**: 54 test binaries listed, 53 expected. `callback_registration.rs` is the 54th; `expected_suites` moved to 54 in the next commit, with CI's 2382 total and 0 ignored |

`cargo xtask verify-docs` is a report and never fails. Its report grows by
three entries against `master`: two long functions this change adds,
`vpay_worker::callbacks::report` (101 lines, one log line per state) and the
Notch Pay adapter's `decide` (85), and the `print_stdout` allowance on
`callbacks ensure`'s report.

**The five integration failures, and why none is this change's.**

- `worker_e2e.rs`'s `a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`
  and `a_decline_after_submission_returns_the_intent_to_requires_payment_method`,
  and `webhooks.rs`'s `the_ladder_walks_delivery_delay_and_then_succeeds` and
  `a_delivery_past_the_last_rung_is_exhausted_and_not_rescheduled`, assert
  exact counts on the process-wide Prometheus recorder. `cargo test` runs a
  binary's cases as threads of one process, so each scrape held two cases'
  counts (`vpay_webhook_deliveries_total{outcome="exhausted"} 2`). nextest,
  which CI runs, gives each case a process (`worker_e2e.rs:140` says so).
  Run each alone with `--exact`, all four pass.
- `postgres_smoke.rs`'s `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`
  failed first because the `cratestack` CLI was not installed, and then
  because this shell set `RUST_BACKTRACE=1`, which appends a backtrace to the
  CLI's refusal the test compares verbatim. With the CLI on `PATH` and
  `RUST_BACKTRACE=0` it passes: the measured drift, **206** changes, is
  unchanged by migration `0051`.

The integration suite had to be run three times: the first two died with
"No space left on device" while linking. The third ran with
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_INCREMENTAL=0`.

## Not done

- **Step 5**: no endpoint has been registered with a real Notch Pay account.
  It needs the maintainer's approval to call Notch Pay, and runbook B4 is
  written for it. No public URL is needed to register one.
- **Retiring and repairing** on Notch Pay: no working `DELETE`, no `PUT`.
- **A foreign endpoint** cannot be detected on Notch Pay.
- **A create whose answer is dropped** is not tested: WireMock cannot record
  a request and then drop its answer. The adapter never retries a create
  within a call, by construction.
- **"Every registration call answering `503` leaves the worker loop running
  and `worker_e2e`'s charge settling"** (RFC-0013 § 10): the handler's
  reschedule after an unreachable rail is tested; the full loop with a
  failing rail is not.
- **Stripe and PayPal** have no adapter, so nothing here runs for them.
- **No alert rule** on `vpay_callback_endpoint_checks_total`; the refused-key
  case logs with `alert = true`. _(Since 2026-10-23 there is one,
  `VpayCallbackEndpointUnconfirmed`, tested against synthetic series only:
  [2026-10-23-callback-endpoint-alert.md](2026-10-23-callback-endpoint-alert.md).)_
- **No `docs/status.md` change**: no route, gate or `NotImplemented` token
  moved.
