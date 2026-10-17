# 2026-10-17 — RFC-0010 delivery step 1: the two core changes for an `Origin` rail

## What this is, and what it is not

The core half of [RFC-0010](../../rfc/0010-stripe-adapter.md) § 7, landed on
its own because every `RefundDestination::Origin` rail needs it and no rail
depends on the rest. **No rail changed. No shipping rail declares `Origin`.
No rail has refunded anything.** Nothing in `vpay-worker`, the reconciler,
the checkout or the settlement transaction moved — § 7 says that if step 1
had needed them, the port had leaked, and it did not.

Two things changed in shipping code, both in `vpay_api::v1::refunds`:

1. `refund_at_rail` builds the adapter's `ChargeRef` with `ref_extra`
   carrying the charge's `provider_txn_id` under
   `REFUND_PROVIDER_TXN_ID_KEY` (the string `provider_txn_id`), or nothing
   when the charge recorded none. It was `BTreeMap::new()` since 2026-09-16.
2. Nothing else. The `Origin` arm of `resolve_destination` is as it was; what
   changed around it is the evidence that it works, below.

And one thing in the port's documentation: `ProviderAdapter::refund` now says
what `charge.ref_extra` carries on a refund, which kind of rail reads it, and
that an `Origin` adapter finding it missing answers `ProviderError::Config`.
RFC-0010 § 7 said that refusal was "the handler's, before any rail call"; it
is the adapter's, and the RFC carries the dated correction and the reason
(`charges.provider_txn_id` is nullable on purpose).

## The three debts, and how each was paid

`no_shipping_rail_returns_a_refund_to_the_paying_instrument` (conformance,
2026-09-16) asserted that every shipping adapter declares `Required` and named
three things owed before it could be relaxed.

| Debt                                                                                  | Paid by                                                                                                                                                                                                                                                      |
| ------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| An integration case sending `destination` to an `Origin` rail and asserting the `400` | `an_origin_rail_refuses_a_destination_through_the_route` in `backends/tests/integration/tests/refunds.rs`: `400`, `param: destination`, `type: invalid_request_error`, the payee not echoed, no `refunds` row, nothing reserved, the rail never called       |
| A case that a refund **without** a destination succeeds on it                         | `a_refund_without_a_destination_succeeds_on_an_origin_rail_and_names_the_payment`: `201` `pending`, the amount reserved, exactly one rail call carrying `destination: None`, the refund's own reference, and `provider_txn_id` as the only `ref_extra` entry |
| RFC-0003 § 1 amended, since it said every rail vpay carries needs a payee             | A dated paragraph under § 1                                                                                                                                                                                                                                  |

The rail those two cases drive is `OriginRail`, a `ProviderAdapter` that
exists only in the integration test binary: `Push`, refunds and partial
refunds on, `RefundDestination::Origin`, `submit`/`query_status`/`parse_callback`
answering `Unsupported`, and a `refund` that records what it was handed and
answers `Ok(Refunded { ref_extra: {}, fee: None })`. ADR-0006 permits a
double there and nowhere else, and `cargo xtask verify-no-mocks` passing
(below) is what proves it reaches no shipping process. The harness
(`support/mod.rs`) gained `serve_with_adapters` and
`reconcile_from_config_with`, each taking an adapter map; the unchanged
`serve`, `router_deps` and `reconcile_from_config` wrap them with
`adapters_by_code()` as before, so every other suite boots exactly as it did.

The conformance case became
`every_shipping_rails_refund_destination_agrees_with_its_parser`: a `Required`
rail must have overridden `parse_destination` (its answer to a payee map is not
`Unsupported`), an `Origin` rail must not have. It admits an `Origin` rail and
does not drive one, because that suite parameterises over the rails that ship.

Two pure unit cases pin the two `ref_extra` shapes without a database:
`a_refund_names_the_payment_it_unwinds` and
`a_refund_of_an_unnamed_payment_carries_no_key`.

## What ran, measured, and what did not

The sandbox that wrote this has **no Docker**, so every suite that boots a
testcontainers Postgres or WireMock — the whole of `backends/tests/integration`
and the per-rail half of `backends/tests/conformance` — could not be executed
here. **The two new integration cases have been compiled and never run by the
author.** CI's `rust` job, which runs `cargo nextest run --workspace`, is the
first execution; this page is to be read next to that run's result, not
instead of it. `just` is not installed in the sandbox either, so the gates
below ran as `cargo xtask <gate>` one by one, which is what the recipe does.

| Command                                                                                                                             | Result                                                                                                                                                                                                                          |
| ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo build --tests -p vpay-tests-integration -p vpay-tests-conformance`                                                           | Finished, 4m 48s cold                                                                                                                                                                                                           |
| `cargo test -p vpay-api --lib`                                                                                                      | **408 passed, 0 failed, 0 ignored** (384 on 2026-09-16; the two new cases are among the 24 since)                                                                                                                               |
| `cargo test -p vpay-api --lib -- refunds::tests`                                                                                    | 23 passed, 0 ignored, including `a_refund_names_the_payment_it_unwinds`, `a_refund_of_an_unnamed_payment_carries_no_key` and `an_origin_rail_refuses_a_destination`                                                             |
| `cargo test -p vpay-tests-conformance --test adapter_conformance -- every_shipping_rails_refund_destination_agrees_with_its_parser` | 1 passed, 66 filtered out — container-free, so it did run                                                                                                                                                                       |
| `cargo clippy -p vpay-api -p vpay-tests-conformance -p vpay-tests-integration --all-targets -- -D warnings`                         | **Failed once**, `clippy::indexing_slicing` on `&recorded[0]` in the new accepting case; replaced with a `let [call] = recorded.as_slice() else { panic!(…) }` slice pattern, which is also the stronger assertion. Then clean. |
| `cargo fmt --all -- --check`                                                                                                        | **Failed once** on one over-long `format!` in the new case; `cargo fmt --all` applied, then clean                                                                                                                               |
| `cargo test --doc -p vpay-api -p vpay-provider`                                                                                     | vpay_api 19 passed; vpay_provider 12 passed; 0 ignored                                                                                                                                                                          |
| `cargo nextest run --workspace` / `cargo test -p vpay-tests-integration --test refunds`                                             | **NOT RUN — no Docker.** CI is the evidence.                                                                                                                                                                                    |
| `just ci`                                                                                                                           | **NOT RUN — `just` is not installed and the integration half needs Docker.** The gates it chains are the rows below.                                                                                                            |

Then every gate `just verify` chains, as `cargo xtask <gate>`, on the staged
tree (`verify-links` reads `git ls-files`, so the new page was `git add`ed
first):

| Gate                        | Printed                                                                                                                                                                                                           |
| --------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `verify-no-mocks`           | ok — no test double reachable from a shipping binary (this is the line that makes `OriginRail` admissible)                                                                                                        |
| `verify-status`             | ok — 1 unimplemented item(s), all declared in docs/status.md and all still in shipping code                                                                                                                       |
| `verify-errors`             | ok — 20 error type(s), all classified; 17 `#[from]` variant(s) delegate every `Classify` method they match on; anyhow confined to binaries                                                                        |
| `verify-sdk-parity`         | ok — 767 proving test(s) named in docs/sdks/parity.md all exist, 45 dated gap(s), 35 SDK method(s) enumerated across 40 row(s)                                                                                    |
| `verify-links`              | ok — 2209 repository link(s) in 444 tracked markdown file(s) resolve to a tracked path                                                                                                                            |
| `verify-npm-scope`          | ok — 3 publishable package(s) under sdks/, 1 private one(s) declaring no publishConfig, and no retired package name outside the archive                                                                           |
| `verify-serde`              | ok — 104 serialisable type(s) spell the workspace's wire convention, 17 exempted with a reason in docs/adr/0016-engineering-standards.md                                                                          |
| `verify-repositories`       | ok — 4 concrete implementation(s) in backends/crates/vpay-db, named by none of the 85 source file(s) outside it, and no generated schema module is exported                                                       |
| `verify-toolchain`          | ok — rust-toolchain.toml pins 1.98.0 and all 1 `FROM rust:` instruction(s) in backends/Dockerfile name it                                                                                                         |
| `verify-migrations`         | ok — 49 migration file(s) in backends/migrations/ all match their entries in backends/migrations/MANIFEST.sha256                                                                                                  |
| `verify-versions`           | ok — 24 version references all say 0.6.0                                                                                                                                                                          |
| `verify-privacy-inventory`  | ok — 307 database column(s) classified in schemas/privacy-inventory.yaml across 26 elements, checked in both directions against 307 column(s) derived from backends/migrations; 24 debug_protection(s) registered |
| `verify-doc-counts`         | ok — 13 documented count(s) in 11 of 282 markdown file(s) agree with what this tree measures (the `75` on [README.md](../README.md) is one of them)                                                               |
| `verify-ui`, `check-schema` | **NOT RUN.** Both are `bash` recipes of the `justfile` with no `cargo xtask` form, and `just` is not installed here; neither reads anything this change touched (no frontend file, no `schemas/vpay.cstack`).     |
| `verify-docs` (advisory)    | ran; the report is unchanged in kind — the `#[allow]`/`#[expect]` list and the long-function list name no file this change touched                                                                                |

`RUSTDOCFLAGS="-D warnings" cargo doc -p vpay-provider --no-deps` reports
**4 warnings, all pre-existing** (`RedactedReqwest` at `http.rs:293`, `:296`
and `lib.rs:959`; `vpay-adapter-orange-money` at `lib.rs:348`) and none in the
two doc sections this change added; `cargo doc` is not a gate, and this is
recorded so nobody reads those four as new.

## What is left

- Steps 2–7 of RFC-0010's delivery, every one of them. No `vpay-adapter-stripe`
  crate, mapping, stub or configuration exists, and nothing has called
  Stripe from this repository.
- § 8's three conformance suite changes (`callback_url_pattern` as `Option`,
  the payee-addressed refund cases gated on `Required`, the hosted-page walk
  parameterised over every `Redirect` rail) wait on a shipping `Origin` rail
  to need them.
- The port still has no refund status read; every refund any rail accepts
  stays `pending` (RFC-0003 open question 8, open).
- `docs/status.md`'s machine-checked declaration did not move: no
  `NotImplemented` token was added or retired.

The skills changed in the same piece of work:
`vpay-provider-adapters/SKILL.md`, its `references/conformance.md` and
`references/adding-a-rail.md` in vaam-apps/vpay-skills, where
`tools/verify-coverage.mjs` is what fails if the case names above and the
skill disagree.
