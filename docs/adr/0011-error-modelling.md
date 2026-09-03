# ADR-0011: Errors are typed at the leaves, composed per layer, classified once, and `anyhow` only at the edge

- **Status:** Accepted
- **Date:** 2026-09-02
- **Deciders:** vpay maintainers

## Context

By this date the workspace had ten error types across seven crates, every
one a `thiserror` enum, and `anyhow` correctly confined to the two binaries
(`AGENTS.md`: "`thiserror` for library crates, `anyhow` only in binaries").
What it did not have was any agreement on what happens to an error *between*
the crate that raises it and the place it is finally acted on:

- The HTTP layer built its Stripe-shaped envelope by hand at every call site
  (`error_envelope("invalid_request_error", "unknown_route", ...)`), so the
  status, `type` and `code` for a given failure were whatever the nearest
  handler chose. Two handlers could — and eventually would — answer the same
  `DbError` with different statuses.
- Nothing said whether an error was retryable, by whom, or how loudly to log
  it. The worker (Phase 5) will have to decide retry-vs-dead-letter for every
  `ProviderError` and `DbError`; without a shared answer it would grow its
  own `match`, drifting from the API's.
- Binaries exited `1` for everything, so a supervisor could not tell "fix the
  YAML" from "Postgres is down".
- `ADR-0007` denies `unwrap`/`expect`/`panic` in production code, which makes
  error handling explicit and verbose. Verbosity without structure is how
  `Box<dyn Error>` and stringly-typed catch-alls creep in.

The merchant-facing side already had the right shape: `FailureCode`
(`docs/flows/failures.md`) is a closed vocabulary owned by the core, into
which every adapter maps its rail's errors, and it carries its own policy
(`payer_actionable`, `merchant_actionable`). This ADR applies the same idea
to the system's own errors.

## Decision

Errors are organised in **three tiers**, with one **cross-cutting
classification** that every tier implements or consumes.

### Tier 1 — leaf errors (`thiserror`, one per crate concern)

- Each library crate defines closed `thiserror` enums for its own failures
  (`MoneyError`, `LedgerError`, `ConfigError`, `DbError`, `ProviderError`,
  `AuthRejection`, ...). One enum per *concern*, not per function.
- Foreign errors are wrapped with `#[source]` (or `#[from]` when the wrap is
  one-to-one and adds no information), never flattened to a `String` and
  never boxed as `Box<dyn Error>`. A `String` payload is allowed only for
  data that was never an error type to begin with (a rail's raw reason, a
  field name).
- `Display` is for operators and logs and may name hosts, tables and library
  text. It **must not** contain a secret: no credential value, no PEM, no
  token. Hand-written `Debug` where a payload could carry one.
- `#[non_exhaustive]` is used on errors that cross a *published* boundary
  (the SDKs) and not on workspace-internal ones, where exhaustive `match` is
  the point.
- `ProviderError::NotImplemented("<crate>::<fn>")` stays exactly as it is —
  `cargo xtask verify-status` scans for that literal — and classifies as
  `Category::NotImplemented`.

### Cross-cutting — `vpay_core::error::Classify`

`vpay-core` (which depends on no framework) owns `Category`, `Retry`,
`Severity` and the trait `Classify`. Every leaf error implements `Classify`;
the only required method is `category()`, and the category derives an HTTP
status, a Stripe-shaped `type`, a default `code`, a retry policy, a log
severity, a public message and a process exit code. A leaf overrides a
default only when it knows better, with a comment saying why.

`cargo xtask verify-errors` fails the build if a `pub enum ...Error` or
`...Rejection` in `backends/crates` has no `impl Classify`, or if a library
crate lists `anyhow` under `[dependencies]`.

### Tier 2 — composite errors (one per layer)

A layer that consumes several crates defines its own enum that `#[from]`s
the leaves it depends on and adds the layer's own variants; its `Classify`
impl delegates to the leaf. Today:

- `vpay_api::ApiError` — the HTTP boundary's error. Its `IntoResponse`
  derives status, `type`, `code` and message from `Classify`; no handler
  builds an envelope by hand. `AuthRejection` converts into it.
- `vpay_worker::JobError` — the job loop's error. Its retry decision
  (`retry now`, `retry after the poll ladder`, `dead-letter`) is derived
  from `Classify::retry`, so the worker and the API can never disagree on
  whether a `DbError` is transient.

Composites do not re-classify: a `DbError` is `Storage` whether it surfaces
through the API or the worker.

### Tier 3 — boundaries (`anyhow` at the edge only)

- `backends/apps/*` `main()` returns `anyhow::Result<()>` and uses
  `.context(..)` to say *what the process was doing* when a leaf failed. The
  leaf's `Classify` still decides the exit code: `main` finds the first
  classifiable error in the `anyhow` chain (`vpay_core::error::find_in_chain`)
  and exits with `Category::exit_code()` — `78` for configuration, `69` for
  an unreachable database, `1` otherwise.
- `anyhow` never appears in a library crate's `[dependencies]` and never in a
  public signature. It may appear in `[dev-dependencies]` for tests.
- The SDKs (`sdks/rust`, `sdks/nodejs`) model the *wire* — the envelope the
  API emits — not the system. They do not implement `Classify`.

## Alternatives considered

- **One global `VpayError` enum.** Rejected: every crate would depend on every
  other crate's failure vocabulary, and a match in the ledger would have to
  acknowledge JWT errors. Composition per layer keeps each crate's enum
  closed and small.
- **`anyhow` everywhere.** Rejected: a payment path needs to *branch* on
  errors (retry a rail timeout, never retry a rejected charge), and
  `anyhow::Error` can only be downcast by guessing types. It is the right
  tool exactly where nothing branches — process startup.
- **`Box<dyn Error + Send + Sync>` in library signatures.** Rejected for the
  same reason, and because it erases the `#[source]` chain's types.
- **`snafu` / `error-stack` / `eyre`.** Not adopted: `thiserror` already
  expresses everything needed and is what every crate uses; adding a second
  error framework would be a migration for no new capability. Revisit if
  span-attached context (`error-stack`'s strength) becomes a real need in
  the worker.
- **A `Classify` impl via derive macro.** Not now: the impls are a few lines
  each and the explicit `match` is where the "why is this `Storage` and not
  `Internal`" reasoning lives. A macro would hide it.

## Consequences

- Every new error enum costs one `impl Classify` — ten lines — and gets
  correct HTTP, retry, logging and exit-code behaviour for free. Forgetting
  it fails `just verify`.
- Handlers stop deciding statuses. `error_envelope` remains as the one
  function that renders an envelope, but it is called from `ApiError`'s
  `IntoResponse` and from nowhere else in production code.
- Retry policy has one home. When the worker's job loop lands (Phase 5), it
  consumes `Classify::retry` rather than inventing a table.
- Adding a `Category` variant is an ADR-level change: the `match`es in
  `vpay_core::error` are exhaustive by design, so every boundary is forced
  to decide what the new category means for it.
- `find_in_chain` is typed, so a binary must name the leaf types it knows
  how to classify. An error nothing names falls through to `Internal` — exit
  `1`, severity `Page` — which is the honest outcome for an unclassified
  startup failure in a payment binary.
- Migration cost now: `error_envelope` call sites in `vpay-api` move behind
  `ApiError`; `AuthRejection` gains a `Classify` impl and a conversion; the
  binaries' `main` gains an exit-code mapping; ten leaf enums gain a
  `Classify` impl each. No wire format changes.

---

## Amendment, 2026-09-03 (Step 7, Phase A) — the rail failures carry a typed cause

Appended rather than superseded: nothing above is reversed, and the shape
below is what the ADR already asked for, applied to the two variants that
were not following it.

`ProviderError::Transport(String)` and `ProviderError::Malformed(String)`
flattened `reqwest`'s and the bounded-body reader's errors with `format!` at
roughly forty call sites — the *"never flattened to a `String`"* rule, broken
in the one place a payment path most needs the leaf. They are now struct
variants:

```rust
Transport { context: String, #[source] source: Option<RailFailure> }
Malformed { context: String, #[source] source: Option<RailFailure> }
```

`RailFailure` is a closed enum (`Http(reqwest::Error)`, `Body(HttpBodyError)`)
rather than `Box<dyn Error>`, because this ADR forbids the box; it costs no
dependency, since both types are already in every adapter's graph. It carries
its own `impl Classify` (`Category::Rail`) so `verify-errors` can see it, and
it is never consulted on its own — a `ProviderError` is what a boundary
classifies.

`Display` renders `context` only. The chain is rendered where an operator
reads it, by the new `vpay_core::error::source_chain`, which `ApiError::log`
already did privately and `vpay_worker`'s job settlement now does too (so
`jobs.last_error` keeps the leaf).

`cargo xtask verify-errors` gains the check §"Tier 2" always implied: for
every `#[from]` variant, each `Classify` method whose body matches on `self`
must name `Self::<Variant>` explicitly, so a new leaf cannot be answered for
by an existing `_ =>` arm. Methods that do not match on `self` are exempt —
there is no wildcard to hide in.

**Not changed:** the three tiers, the `Classify` seam, `anyhow` at the edge,
`ProviderError::NotImplemented`'s literal, and every wire-visible status,
`type`, `code` and message.
