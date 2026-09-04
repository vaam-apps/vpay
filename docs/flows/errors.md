# Errors

How a failure travels from where it happens to where it is acted on, and
what stays true along the way. Decision record:
[ADR-0011](../adr/0011-error-modelling.md).

## The invariant

> **Every error is classified exactly once, by the crate that raises it, and
> every boundary — HTTP envelope, worker retry, process exit code, log
> line — is derived from that classification, never re-decided.**

Two errors of the same kind therefore always get the same status, the same
retry policy and the same severity, whichever handler or job they surface
through.

## The three tiers

```
 rail / Postgres / YAML / caller input
          │
          ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ TIER 1 — leaf errors, one thiserror enum per crate concern    │
 │  MoneyError · LedgerError · ConfigError · DbError             │
 │  ProviderError · AuthRejection · UnknownCurrency              │
 │  each: impl Classify { fn category(&self) -> Category }       │
 └──────────────────────────────────────────────────────────────┘
          │ #[from] / #[source]
          ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ TIER 2 — composite errors, one per layer                      │
 │  vpay_api::ApiError      (HTTP)    → Classify delegates       │
 │  vpay_worker::JobError   (jobs)    → Classify delegates       │
 └──────────────────────────────────────────────────────────────┘
          │ consumed, never re-classified
          ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ TIER 3 — boundaries                                            │
 │  IntoResponse → Stripe envelope   (status, type, code, msg)   │
 │  JobError::decision → RetryAfter{alert} / Terminal / DeadLetter│
 │  main(): ExitCode from the anyhow chain (.context at each step)│
 │  tracing level from Severity                                  │
 └──────────────────────────────────────────────────────────────┘
```

`anyhow` lives only in the bottom box, and only in `backends/apps/*`.

## The classification

`vpay_core::error::Category` is the whole policy table. Everything else is
derived from it unless a leaf error overrides one column with a comment
saying why.

| Category | Whose problem | HTTP | Stripe `type` | default `code` | Retry | Severity | Exit |
|---|---|---|---|---|---|---|---|
| `InvalidRequest` | caller | 400 | `invalid_request_error` | `invalid_request` | never | info | 64 |
| `Authentication` | caller | 401 | `authentication_error` | `invalid_token` | never | info | 77 |
| `Forbidden` | caller | 403 | `invalid_request_error` | `forbidden` | never | info | 77 |
| `NotFound` | caller | 404 | `invalid_request_error` | `resource_missing` | never | info | 1 |
| `Conflict` | caller (state) | 409 | `invalid_request_error` | `invalid_state` | never | info | 1 |
| `Idempotency` | caller | 400 | `idempotency_error` | `idempotency_key_in_use` | never | info | 64 |
| `RateLimited` | caller (pace) | 429 | `rate_limit_error` | `rate_limit` | after backoff | warn | 1 |
| `Rail` | the rail | 502 | `api_error` | `provider_unavailable` | after backoff | warn | 69 |
| `Storage` | us (Postgres) | 503 | `api_error` | `service_unavailable` | after backoff | error | 69 |
| `Configuration` | operator | 500 | `api_error` | `misconfigured` | never | error | 78 |
| `NotImplemented` | us (honest stub) | 501 | `api_error` | `not_implemented` | never | error | 1 |
| `Internal` | us (a bug) | 500 | `api_error` | `internal_error` | never | **page** | 1 |

The `default code` column is the *default*, and one category now has two
codes in practice. **`Category::Idempotency` carries
`idempotency_key_in_use` (a key replayed with a different body) and
`idempotency_key_in_flight` (a key whose first request has not finished).**
Both are `400`/`idempotency_error` because the status comes from the
category and never from a variant — Stripe answers `409` for the second,
which this policy table cannot express without splitting the category. That
is an ADR-level change and is deliberately left as a maintainer decision;
`ApiError::IdempotencyKeyInFlight`'s doc comment records it, and
`a_key_still_in_flight_is_a_different_code_from_a_key_reused_and_from_a_conflict`
pins the three answers apart. Do not edit the table's cell for this: the row
is transcribed literally into `vpay-core`'s own test, and the *default* is
still `idempotency_key_in_use`.

**Three categories that were unreachable are now reachable from a real
request path** (2026-09-03, Step 2): `NotFound` → `resource_missing` (an
unknown `pi_…`, *and* another merchant's — byte-identical, so the API is not
an id oracle), `Conflict` → `invalid_state` (a confirm or cancel the
lifecycle forbids), and `Forbidden` → `forbidden` (a token without
`payments:read`/`payments:write`). `NotImplemented` → `not_implemented`
(`501`) is reachable too, from `confirm` reaching a rail adapter. See
`docs/api/README.md` for the full list of codes a `/v1` caller can receive.

`Retry` has a third value, `NewAttempt`, for operations that must not be
repeated as-is but may be started over (a failed charge: retry means a new
`PaymentIntent`, [payment-lifecycle.md](payment-lifecycle.md)). No category
defaults to it; a leaf sets it explicitly.

Exit codes follow `sysexits.h` where one fits (`EX_CONFIG` 78,
`EX_UNAVAILABLE` 69, `EX_USAGE` 64, `EX_NOPERM` 77).

## How each leaf classifies itself

| Leaf | Variant(s) | Category | Note |
|---|---|---|---|
| `MoneyError` | `Negative`, `CurrencyMismatch` | `InvalidRequest` | the amount or currency came from a caller |
| | `Overflow` | `Internal` | integer minor units overflowing `i64` is a bug, not a request |
| `UnknownCurrency` | | `InvalidRequest` | |
| `LedgerError` | `Unbalanced`, `TooFewEntries` | `Internal` | the core builds transactions; an unbalanced one is our bug |
| | `Money(..)` | delegates | |
| `ConfigError` | all | `Configuration` | `MissingPath` too: an operator forgot `--config` |
| `DbError` | `Connect`, `Healthcheck`, `Query` | `Storage` | |
| | `Migrate` | `Configuration` | a broken migration is a deploy problem, not a transient one |
| | `UniqueViolation` (SQLSTATE `23505`) | `Conflict`, code `resource_conflict` | **not** `invalid_state`: the object is not in a forbidden *state*, it already exists. A merchant must be able to tell "you already did this" from "this intent cannot be cancelled now" (`integrity_violations_are_the_callers_problem_not_a_storage_outage`) |
| | `ForeignKeyViolation` (SQLSTATE `23503`) | `InvalidRequest`, code `invalid_reference` | the request named a currency, provider or object that does not exist; no retry of the same request can succeed |
| | `WriteMatchedNoRow` | `Internal` | a compare-and-swap this crate's own caller set up matched nothing — nobody outside vpay can cause it (`the_two_invariant_refusals_classify_as_ours_not_the_callers`) |
| `ProviderError` | `Transport { context, source }`, `Malformed { context, source }` | `Rail` | retried by the poll ladder. **Changed 2026-09-03 (Step 7): both are struct variants** carrying the adapter's own sentence in `context` and the library error underneath as a real `#[source]` (`RailFailure::Http(reqwest::Error)` / `RailFailure::Body(HttpBodyError)`), instead of one `String` both adapters used to `format!` the foreign error into. `Display` still renders `context` alone, so the body cap and the rail's name stay in the one-line message; the leaf reaches a log through `vpay_core::error::source_chain` and a caller through `Error::source()` — see below |
| `RailFailure` | `Http(reqwest::Error)`, `Body(HttpBodyError)` | `Rail` | the cause a `Transport`/`Malformed` was raised from; classified so `verify-errors` can check it, never consulted on its own. `Http` is a send that never completed, `Body` a read that failed or ran past `MAX_RAIL_BODY_BYTES` — the two the port's own `read_rail_body` distinguishes |
| | `Rejected { code, .. }` | `Conflict` with `Retry::NewAttempt`; envelope `code` is the constant `charge_declined`; severity from the `FailureCode`'s own policy (`provider_account_blocked` pages, `provider_unavailable`/`provider_error` warn, the rest info) | a rail *decision*, not a rail failure — see below. The `FailureCode` itself is in the public message, never reused as the envelope `code`, because `provider_unavailable` already means "502, retrying" when `Transport` emits it |
| | `Config` | `Configuration` | |
| | `Unsupported` | `Conflict`, severity `Error` | 409 because the request cannot proceed; logged at `Error` because reaching it means the core skipped the capability check it is supposed to branch on |
| | `NotImplemented(..)` | `NotImplemented` | |
| `AuthRejection` | all three | `Authentication` | one category, one status, one `type`; the three codes/messages say only whether a header was present and well-formed, never anything about the token — not an oracle |

**A rail failure keeps its cause.** Before Step 7, `ProviderError::Transport`
and `Malformed` were `Transport(String)`/`Malformed(String)`, so every
adapter flattened `reqwest`'s error with `format!`. `reqwest`'s own `Display`
for a timeout is *"error sending request for url (…)"* — the word *timeout*
is one link further down the chain — so MTN's log line named the URL and not
the fault. Orange had noticed and hand-walked `Error::source()` into a
`String`, which is the same information rebuilt by hand at one of the two
adapters. Both now attach the error itself:

```rust
ProviderError::transport_from("mtn_momo: the request to the rail failed", error)
```

and the chain is rendered once, at the boundary that logs it
(`ApiError::log`'s `source_chain` field, and `vpay_worker`'s
`jobs.last_error`), through `vpay_core::error::source_chain`. What an
operator sees for a timeout is now
`sending the request: error sending request for url (…): operation timed
out`. `a_transport_failures_source_chain_reaches_the_reqwest_error`
(`vpay-adapter-mtn-momo`) fails if anyone goes back to `format!`.

A `serde_json` parse failure is deliberately *not* attached as a source: its
own text is the whole diagnostic and belongs in `context`, where a one-line
log shows it.

Two constructors per variant, and which one a call site reaches for is the
whole of the decision: `transport`/`malformed` when the rail *answered*
(badly) and there is no library error to attach, `transport_from`/
`malformed_from` when there is. All four carry a doctest asserting that
`Display` renders `context` alone while the cause stays reachable through
`Error::source()` — the property that would silently die the day someone goes
back to `format!`.

**Which variant each port operation may raise is a table, in the port's own
rustdoc.** `ProviderAdapter`'s trait doc carries it and each of the four
methods carries an `# Errors` section naming its own set;
`#![warn(clippy::missing_errors_doc)]` on `vpay-provider` makes a method that
loses one fail `cargo clippy -- -D warnings` (it covers trait *definition*
methods, verified by deleting one). The three rows worth knowing without
reading it:

- `parse_callback` raises `Malformed` and nothing else. It touches no
  network, holds no credential and reads no configuration, so there is no
  transport to fail and no decision to relay.
- `query_status` raises `Rejected` **only** when the rail refuses *our*
  partner credentials. A declined charge is `Ok(ChargeStatus::Failed)` and a
  rail with no record is `Ok(ChargeStatus::NotFound)` — neither is an error.
- `submit` never raises `Unsupported`: a rail that cannot take a payment is
  not a rail. `Unsupported` belongs to `refund` alone, where it is the
  permanent capability answer rather than unbuilt work.

There is deliberately **no** `ProviderError::retryable()`. Retry policy is
`Classify::retry` and a second oracle beside it is what ADR-0011 exists to
prevent; the worker reads `Classify` exclusively.

**A body that fails mid-stream now says so.** Both adapters used to map
`HttpBodyError::Read(reqwest::Error)` onto `RailFailure::Http`, whose
`Display` is *"sending the request"* — describing a stage that had already
succeeded. `vpay_provider::http::read_rail_body`, which is now the one
bounded read both adapters call, keeps the `HttpBodyError` and the chain
reads *"reading the response"*. Both classify `Category::Rail`; only the text
an operator reads changed.

**`ProviderError::Rejected` is the seam between system errors and business
outcomes.** A rail declining a charge is not a system failure: the worker
records it as the charge's `failure_code` and the intent returns to
`requires_payment_method` with `last_payment_error` populated. It is
classified here only so that a path which *does* surface it as an error
(an adapter conformance test, a synchronous refund) answers coherently.

## Composite errors

**`vpay_api::ApiError`** wraps every leaf the HTTP layer can meet
(`DbError`, `ProviderError`, `MoneyError`, `UnknownCurrency`, `LedgerError`,
`ConfigError`, `AuthRejection`) and adds its own variants (`UnknownRoute`,
`InvalidParam`, `IdempotencyKeyReused`, `Internal`). axum's own extractor
rejections (`Form`, `Json`, `Path`, `Query`) convert into `InvalidParam`
with a curated sentence, so a malformed body gets the envelope rather than
axum's plain-text 400. `IntoResponse` does exactly this and nothing else:

```
status  = err.category().http_status()
body    = { "error": { "type": category.stripe_type(), "code": err.code(),
                       "message": err.public_message(), "param"?: <if the variant names one> } }
log     = at err.severity() (alert=true when Page), with the full Display + source chain
```

The full chain goes to the log; only `public_message()` goes to the
merchant. The two envelope renderers are `pub(crate)`, so a handler
*cannot* build one by hand — "one renderer" is structural, not a
convention. (ADR-0011 names the renderer `error_envelope`; the production
one is its sibling `error_envelope_with_param`, and `error_envelope`
survives only as a test-side wrapper. The ADR's claim — one renderer,
called from `IntoResponse` and nowhere else — holds for the family.) `InvalidParam.message` and `UnknownCurrency`'s echoed code are
length-bounded at render time so a caller cannot reflect a megabyte back
into the envelope.

**`vpay_worker::JobError`** wraps `DbError`, `ProviderError`, `MoneyError`
and `LedgerError` and adds job-level variants (`Poisoned`, `Exhausted`).
Its `decision(attempt)` is derived from `Classify::retry` and
`Classify::severity` alone:

| `retry()` | Decision |
|---|---|
| `AfterBackoff` | `RetryAfter { delay, alert }`: re-run after `poll_delay(attempt)` ([reconciler.md](reconciler.md)) — or after `UNRESOLVED_POLL_INTERVAL` (one hour) for `Exhausted` — with `alert = true` when severity is `Error` or above, so a human is paged while the loop keeps going |
| `NewAttempt` | `Terminal`: this job is over; the intent's own state machine decides what a new attempt means |
| `Never` | `DeadLetter`: nothing the loop can do will change the outcome — park it for a human |

`Exhausted` is the reconciler's `unresolved` state: the 24-hour horizon
passed with no terminal answer. Per [reconciler.md](reconciler.md) that is
**alert and keep polling hourly**, never a silent failure and never a
dead-letter — a late success at hour 30 is a normal transition.

## Boundaries

**HTTP.** Handlers return `Result<_, ApiError>` and use `?`. They never
construct an envelope, choose a status, or format a message for a merchant.

**Worker.** Jobs return `Result<_, JobError>`; the loop calls `decision()`
and logs at `severity()`. The loop does not inspect variants.

**Binaries.** `main` returns `ExitCode` and wraps an `async fn run() ->
anyhow::Result<()>` in which every fallible startup step gets
`.context("what we were doing")`. On `Err`, `main` prints the full chain to
stderr (`eprintln!("{e:#}")` — `tracing` may not be initialised yet when
configuration fails), finds the first classifiable leaf in the chain
(`find_in_chain::<ConfigError>` first, then `DbError` — a config naming a
dead database is still a config problem), and exits with
`category().exit_code()`, `Internal`/`1` if nothing matched.

## What can go wrong

| Failure | Where it surfaces | What holds |
|---|---|---|
| A new error type forgets `impl Classify` | `cargo xtask verify-errors` fails `just verify`: it finds every `pub` type in `backends/crates` that derives `thiserror::Error` **or** is named `*Error`/`*Rejection`, outside `#[cfg(test)]` blocks and `tests/` directories, and requires an impl in the same crate that is itself outside test code | nothing unclassified reaches a boundary — within that scan; the SDKs and `backends/apps` are outside it by design |
| A library crate adds `anyhow` to `[dependencies]` | same check | `anyhow` stays at the edge |
| A composite grows a `#[from]` leaf and an existing `_ =>` arm answers for it | same check, extended 2026-09-03 (Step 7): for every `#[from]` variant, each `Classify` method that *discriminates* on `self` must name `Self::<Variant>` explicitly. Five spellings count as discriminating — `match self`, `match *self`, `match &self`, `if let Self::`, `matches!(self` — because searching only for `match self` made the rule opt-out: an `if let` ladder's trailing `else` answers for an unnamed leaf exactly as a `_ =>` arm does. Proven live by deleting `ApiError`'s `Self::Db(e) => e.code()` arm and watching `verify-errors` refuse, and by `a_from_variant_swallowed_by_an_if_let_ladder_is_reported`, which fails if the list is narrowed back | composites do not re-classify — the leaf's own `code`/`retry`/`severity`/`public_message` reach the boundary instead of the category default |
| A handler hand-builds an envelope with the wrong status | the renderers are `pub(crate)` to `vpay-api`, so code outside the crate cannot call them at all; inside the crate, `error_envelope_with_param` has one production caller (`ApiError::into_response`) and a second is a review finding | one status per category |
| A leaf's `Display` includes a secret | the existing redaction tests (`Debug` on `ProviderHost`, `CommonArgs`, SDK `Credentials`) — extend them when adding a payload that could carry one | secrets never reach a log |
| A `public_message()` override leaks a table or host name | test in `vpay-core` for the generic messages; add one per override | merchants see nothing internal |
| Two boundaries disagree on retry | impossible by construction — both read `Classify::retry` | one policy |

## How to add an error

1. Add the variant to the crate's existing enum, or a new `thiserror` enum
   if it is a new concern. Keep `#[source]` on wrapped errors — a foreign
   error is attached, never `format!`ed into the message, so the leaf is
   still reachable from the boundary that logs it
   (`vpay_core::error::source_chain`).
2. Classify it: extend the crate's `impl Classify`. If the category's
   defaults are wrong for it, override `code`/`retry`/`severity`/
   `public_message` with a comment saying why.
3. If a composite needs to carry it, add a `#[from]` variant there and
   delegate in its `Classify` impl — in **every** method that discriminates
   on `self` (`match`, `if let Self::`, `matches!`), which `verify-errors`
   now checks rather than trusting.
4. `just verify` — `verify-errors` checks the impl exists; `verify-status`
   still checks any `NotImplemented` token.
5. Test the classification if it overrides a default — the override is the
   decision worth pinning.

## Status

**Implemented:** `vpay_core::error` (`Category`, `Retry`, `Severity`,
`Classify`, `find_in_chain`), with invariant tests over every category
*and* a literal transcription of the table above as a test, so this
document and the code fail together; `impl Classify` on every leaf listed
above; `vpay_api::ApiError` with `IntoResponse` deriving the envelope, the
existing 404 fallback routed through it, and `AuthRejection` classified
and rendered through it; `vpay_worker::JobError` with `decision()`; both
binaries exiting with `Category::exit_code()`; `cargo xtask verify-errors`
in `just verify` and CI. See [../status.md](../status.md) for the
row-by-row proof, including which of these are proven by a test that would
fail if they broke.

**New 2026-09-03 (Step 2), and it changes what the paragraph below can
say.** `ApiError` gained four variants — `NotFound { resource, id }`,
`Conflict { message }`, `Forbidden`, and `IdempotencyKeyInFlight { key_hint }`
— and, more importantly, **`/v1/payment_intents` handlers now return them
from a real request path**. A running `vpay-server` can now answer `400`,
`403`, `404 resource_missing`, `409 invalid_state`, `409 resource_conflict`,
`501 not_implemented` and `503` through this machinery, not only the 404
fallback and the 401 envelope. `vpay_api::form`'s `VpayForm`/`VpayQuery`
replaced axum's own extractors on `/v1` for exactly this reason: axum's
`FormRejection` renders plain text, and every rejection must be the Stripe
envelope naming the part of the request it came from
(`a_form_rejection_is_answered_with_the_envelope_not_axums_plain_text`,
`every_extractor_rejection_names_the_part_of_the_request_it_came_from`).
`vpay-api` ran **160 tests** when this paragraph was written on 2026-09-03
(it is 165 after Step 3 — see the note below) (`cargo nextest run -p
vpay-api`, measured): 34 `op`, 25 `form`, 21 `v1`, 20 `error`, 20
`resource_auth`, 15 crate-level, 10 `model`, 9 `idempotency`, 4
`jwks_cache`, 2 `http_client`.

Three properties worth naming because they are what keeps an error envelope
from becoming a leak: a foreign object and a missing object are **byte
identical** (`a_foreign_object_and_a_missing_object_are_byte_identical`); a
storage error's leaf text reaches the log and never the body
(`a_storage_errors_leaf_text_reaches_the_log_and_never_the_body`); and an
idempotency key is never echoed past an 8-character hint, in the log only
(`an_idempotency_key_is_never_echoed_past_its_hint`).

**Updated 2026-09-03 (Step 3): two of these codes are now produced by a
rail.** `POST …/confirm` calls a real adapter over real HTTP, so:

- **`charge_declined` (409)** is what a rail's decision renders as.
  `ProviderError::Rejected`'s `code()` is that constant on purpose, *not*
  the `FailureCode`'s own string: `FailureCode::ProviderUnavailable` renders
  `provider_unavailable`, which is also `Category::Rail`'s default code, and
  a merchant branching on the envelope would otherwise see one token for
  "the rail is down, we are retrying" (502) and "your charge was declined,
  start a new intent" (409). The specific `FailureCode` reaches the merchant
  through the charge's `failure_code` and through the public message
  (`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read`,
  `backends/tests/integration/tests/confirm_rails.rs`).
- **`provider_unavailable` (502)** is what an unreachable rail renders as,
  and the accompanying `409` on a retry tells the merchant to poll rather
  than to open a second PaymentIntent
  (`an_unreachable_rail_leaves_the_charge_where_recovery_expects_it`).
- `ProviderError`'s severity table is exercised in `vpay-provider`'s own 11
  tests (`a_declines_severity_follows_the_failure_codes_own_policy`,
  `an_unsupported_operation_answers_409_but_is_logged_as_our_bug`).

**Still not implemented, and not implied by anything above:** no job loop
exists to call `JobError::decision()`. Every rail response that produced a
`charge_declined` or a `502` above came from a **WireMock** host, not from
MTN or Orange — the codes are real, the rails behind them are stubs. And
`Category::NotImplemented`/`501` is no longer what `confirm` answers: the
one remaining `NotImplemented` token is `mtn_momo::refund`, on a route
(`POST /v1/refunds`) that does not exist, so **no `/v1` caller can currently
provoke a `501` at all**. `vpay-api` runs **165 tests, 165 passed, 0 skipped** as of 2026-09-03
(`cargo nextest run -p vpay-api`, measured).

**Updated 2026-09-03 (Step 7, Phase A): the rail failures carry their
cause.** `ProviderError::{Transport, Malformed}` are struct variants with a
`context: String` and an optional `#[source] RailFailure`; the two adapters
attach the `reqwest`/body error instead of `format!`ing it into a message,
and Orange's hand-rolled `Error::source()` walk is gone — the walk is the
logger's now (`vpay_core::error::source_chain`, used by `ApiError::log` and
by `vpay_worker`'s job settlement, so `jobs.last_error` keeps the leaf it
would otherwise have lost). ~~`verify-errors` counts **14 error types, all classified** (13 until Step 7's
lane 3 added `vpay_api::BootError`)~~ **— corrected 2026-09-04: it counts 15,
Step 8's lane B having added `vpay_worker::ssrf::EgressRefusal`** — and it
additionally checks that each of the **14 `#[from]` variants** is named
explicitly in every `Classify` method that discriminates on `self` — a count that is now derived from the variants that passed rather
than from per-file arithmetic. Nothing about the wire changed: the 26 conformance cases still pass,
and three of their assertions were adapted to the struct shape (Step 7
decision 14) without changing what they assert.
