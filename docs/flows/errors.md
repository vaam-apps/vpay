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
| `ProviderError` | `Transport`, `Malformed` | `Rail` | retried by the poll ladder |
| | `Rejected { code, .. }` | `Conflict` with `Retry::NewAttempt`; envelope `code` is the constant `charge_declined`; severity from the `FailureCode`'s own policy (`provider_account_blocked` pages, `provider_unavailable`/`provider_error` warn, the rest info) | a rail *decision*, not a rail failure — see below. The `FailureCode` itself is in the public message, never reused as the envelope `code`, because `provider_unavailable` already means "502, retrying" when `Transport` emits it |
| | `Config` | `Configuration` | |
| | `Unsupported` | `Conflict`, severity `Error` | 409 because the request cannot proceed; logged at `Error` because reaching it means the core skipped the capability check it is supposed to branch on |
| | `NotImplemented(..)` | `NotImplemented` | |
| `AuthRejection` | all three | `Authentication` | one category, one status, one `type`; the three codes/messages say only whether a header was present and well-formed, never anything about the token — not an oracle |

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
| A handler hand-builds an envelope with the wrong status | the renderers are `pub(crate)` to `vpay-api`, so code outside the crate cannot call them at all; inside the crate, `error_envelope_with_param` has one production caller (`ApiError::into_response`) and a second is a review finding | one status per category |
| A leaf's `Display` includes a secret | the existing redaction tests (`Debug` on `ProviderHost`, `CommonArgs`, SDK `Credentials`) — extend them when adding a payload that could carry one | secrets never reach a log |
| A `public_message()` override leaks a table or host name | test in `vpay-core` for the generic messages; add one per override | merchants see nothing internal |
| Two boundaries disagree on retry | impossible by construction — both read `Classify::retry` | one policy |

## How to add an error

1. Add the variant to the crate's existing enum, or a new `thiserror` enum
   if it is a new concern. Keep `#[source]` on wrapped errors.
2. Classify it: extend the crate's `impl Classify`. If the category's
   defaults are wrong for it, override `code`/`retry`/`severity`/
   `public_message` with a comment saying why.
3. If a composite needs to carry it, add a `#[from]` variant there and
   delegate in its `Classify` impl.
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
`vpay-api` runs **160 tests** as of 2026-09-03 (`cargo nextest run -p
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

**Not implemented, and not implied by anything above:** no job loop exists
to call `JobError::decision()`. `Category::Rail`'s `502`
`provider_unavailable` and `ProviderError::Rejected`'s `charge_declined`
have never been produced by an actual rail — no HTTP call to a rail has ever
been made — so every 5xx a `/v1` caller can currently provoke comes from
vpay's own storage or from the honest `501`. The `NotImplemented` category
is not a placeholder here: it is the answer `confirm` gives.
