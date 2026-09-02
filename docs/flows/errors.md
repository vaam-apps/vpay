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
 │  JobError::decision → Retry now / poll ladder / dead-letter   │
 │  main(): anyhow::Result + .context(..) → exit code            │
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
| `ProviderError` | `Transport`, `Malformed` | `Rail` | retried by the poll ladder |
| | `Rejected { code, .. }` | `Conflict` with `Retry::NewAttempt` | a rail *decision*; `code` (the `FailureCode`) is the merchant-facing signal — see below |
| | `Config` | `Configuration` | |
| | `Unsupported` | `Conflict` | capabilities said no; the core should have checked first |
| | `NotImplemented(..)` | `NotImplemented` | |
| `AuthRejection` | all three | `Authentication` | one category, one message: never an oracle |

**`ProviderError::Rejected` is the seam between system errors and business
outcomes.** A rail declining a charge is not a system failure: the worker
records it as the charge's `failure_code` and the intent returns to
`requires_payment_method` with `last_payment_error` populated. It is
classified here only so that a path which *does* surface it as an error
(an adapter conformance test, a synchronous refund) answers coherently.

## Composite errors

**`vpay_api::ApiError`** wraps every leaf the HTTP layer can meet and adds
its own variants (`UnknownRoute`, request-shape failures). `IntoResponse`
does exactly this and nothing else:

```
status  = err.category().http_status()
body    = error_envelope(err.category().stripe_type(), err.code(), err.public_message())
log     = at err.severity(), with the full Display + source chain
```

The full chain goes to the log; only `public_message()` goes to the
merchant. `error_envelope` is called from here and from nowhere else in
production code.

**`vpay_worker::JobError`** wraps `DbError` and `ProviderError` and adds
job-level variants. Its `decision()` is derived from `Classify::retry`:

| `retry()` | Decision |
|---|---|
| `AfterBackoff` | re-run the job after `poll_delay(attempt)` ([reconciler.md](reconciler.md)) |
| `NewAttempt` | terminal for this job; the intent's own state machine decides what a new attempt means |
| `Never` | dead-letter, at `severity()` — a human looks |

## Boundaries

**HTTP.** Handlers return `Result<_, ApiError>` and use `?`. They never
construct an envelope, choose a status, or format a message for a merchant.

**Worker.** Jobs return `Result<_, JobError>`; the loop calls `decision()`
and logs at `severity()`. The loop does not inspect variants.

**Binaries.** `main` returns `anyhow::Result<()>`; every fallible startup
step gets `.context("what we were doing")`. On `Err`, `main` finds the
first classifiable leaf in the chain (`find_in_chain::<ConfigError>`, then
`DbError`), logs the full chain, and exits with `category().exit_code()` —
`Internal`/`1` if nothing matched.

## What can go wrong

| Failure | Where it surfaces | What holds |
|---|---|---|
| A new error enum forgets `impl Classify` | `cargo xtask verify-errors` fails `just verify` | nothing unclassified reaches a boundary |
| A library crate adds `anyhow` to `[dependencies]` | same check | `anyhow` stays at the edge |
| A handler hand-builds an envelope with the wrong status | review — `error_envelope` has one production caller and grep finds a second | one status per category |
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
`Classify`, `find_in_chain`) with exhaustive tests of the policy table;
`impl Classify` on every leaf listed above; `vpay_api::ApiError` with
`IntoResponse` deriving the envelope, and the existing 404 fallback and
`AuthRejection` routed through it; `vpay_worker::JobError` with
`decision()`; both binaries exiting with `Category::exit_code()`;
`cargo xtask verify-errors` in `just verify` and CI. See
[../status.md](../status.md) for the row-by-row proof, including which of
these are proven by a test that would fail if they broke.

**Not implemented, and not implied by anything above:** no `/v1` handler
exists to return an `ApiError` from, and no job loop exists to call
`JobError::decision()`. The types are the contract Phase 3 and Phase 5
build against; they move no money and serve no request today.
