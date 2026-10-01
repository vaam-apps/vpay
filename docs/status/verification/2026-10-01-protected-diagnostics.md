# 2026-10-01 — RFC-0002 PR 3: protected diagnostic representations (#147)

Branch `feat/gdpr-147-protected-diagnostics`, cut from `origin/master` at
`a33aac61`. **`just ci` was not run on this branch** — the integration suites
need Docker and were not exercised here; see "What was not run" before
trusting anything below as a whole-suite result.

## What changed

The four shared protected diagnostic representations landed in `vpay-core`
(`Secret`, `Masked`, `Pseudonymous`, `SafeUrl` — RFC-0002 PR 3, ADR-0020 § 2),
seventeen personal-data/secret types swapped `#[derive(Debug)]` for hand-written
redacting `Debug` impls, and `cargo xtask verify-privacy-inventory` gained a
`debug_protections` registration list enforced in both directions.

### The wrappers — `backends/crates/vpay-core/src/privacy.rs`

- `Secret<'a, T>` — prints `[redacted]`, nothing of the value.
- `SafeUrl<'a>` — prints `scheme://host[:port]` only; query, fragment and
  userinfo never print (a URL's query is where a credential hides).
- `Masked<'a>` and `Pseudonymous<'a>` — built as **policy-free mechanisms with
  no shipping consumer**, ahead of their first use in PR 4 / #56. `Masked`
  hides values shorter than 8 characters entirely and keeps the last four of a
  longer one; `Pseudonymous` is a domain-separated HMAC-SHA256 that takes its
  key from the caller and has no unkeyed path. Which identifiers may appear
  masked or pseudonymised in which surface is RFC-0002 D3, OPEN; adopting
  either for a live surface before D3 answers is what the module doc forbids.

### The sweep — derived `Debug` → hand-written

| Crate           | Type(s)                                                                                                      | Elements now redacted                                              |
| --------------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------ |
| `vpay-db`       | `ChargeRow`, `NewCharge`, `EventRow`, `NewEvent`, `IdempotencyRecord`, `DeliveryRow`, `NewPaymentIntent`     | `payer_msisdn`, `rail_failure_text`, `webhook_url`, `stored_api_body`, `oauth_client_secret` |
| `vpay-db`       | `PaymentIntentRow` (existing impl)                                                                            | `rail_failure_text` (the message; the code stays visible)          |
| `vpay-api`      | `LastPaymentErrorObject`, `AccountHolderObject`, `EventDataObject`, `OutOfBandPaymentObject`                 | `rail_failure_text`, `customer_name`, `stored_api_body`, `payment_reference` |
| `vpay-api`      | `ChargeSummary` (the dashboard's `/dash/v1` payment-detail charge)                                            | `rail_failure_text`, `payer_msisdn`                                |
| `vpay-provider` | `ChargeRef`, `Submitted`, `ChargeStatus`, `ProviderError`                                                    | `payer_msisdn`, `webhook_url`, `rail_failure_text`                 |

Each swept type carries a canary test in the customers.rs shape: the payer's
literals never appear in `{:?}`, and the structure an operator reads always
does, so a `Debug` that printed nothing could not pass either. The canaries
populate `provider_ref_extra`/`ref_extra` with a `pay_token` canary, so the
one field this PR redacts **and** its neighbour (the same token captured in
the URL's query) are both pinned. `ChargeAsOf`, `FanoutFailure`, `EventObject`,
`PaymentIntentObject` and `ListObject<T>` are
safe **by composition** — their derived `Debug` formats through the swept
types' hand-written impls. `ManualPaymentRow`/`NewManualPayment`,
`CheckoutSessionRow` and the staff/credentials rows already had hand-written
`Debug` and were left alone; `CheckoutSessionRow`'s deliberate URL visibility
is documented in that impl and deliberately **not** registered.

Deliberately **not** swept, as a decision rather than a silence: the private
`SummaryRow`/`RefundSummaryRow` decoders in `vpay-db/src/schema/`
(`search_customers`, `search_webhook_deliveries`, `search_payment_intents`,
`search_refunds`). They are module-private, short-lived `sqlx::FromRow`
decoders on the `/dash/v1` staff-only search surface, and the personal values
they carry are the operator's to read on the same surface's wire response —
`search_customers`' own module header records the maintainer decision that
`name`/`email`/`phone` are unmasked there. The `debug_protections` list
registers the types whose `Debug` is a *log* surface; these internal decoders
are not in that class. If the `/dash/v1` search surface ever renders `{:?}`
of a row, that call site is PR 4's telemetry sweep, and the decoders would be
registered then.

### The gate — `debug_protections`

`schemas/privacy-inventory.yaml` grew a `debug_protections` list: each entry
names a type, its file and the inventory elements its `Debug` redacts. The
gate fails in both directions, like the rest of the inventory:

- a registered type that derives `Debug` → fail (protection undone);
- a registered type no longer declared in its file → fail (stale row);
- an entry naming an element not in the inventory → fail (misspelling cannot
  silently register the wrong protection).

## What was run

- `cargo check` on `vpay-core`, `vpay-db`, `vpay-api`, `vpay-provider` —
  clean.
- `cargo test -p vpay-core` (unit + doctests) — 11 unit tests and 4 doctests
  for the wrappers, all pass.
- `cargo test -p vpay-db --lib` canary modules — 4 tests, all pass
  (`charges::tests`, `events::debug_tests`, `idempotency::debug_tests`,
  `webhook_deliveries::debug_tests`), plus the `payment_intents::tests`
  canaries (suffix, message and `NewPaymentIntent`).
- `cargo test -p vpay-api --lib debug_tests` — 2 tests, all pass
  (`model::debug_tests`, `dash::debug_tests`).
- `cargo test -p vpay-provider --lib debug_tests` — 4 tests under the filter,
  all pass (3 in `debug_tests` plus the pre-existing
  `provider_config_debug_tests`).
- `cargo test -p xtask --bin xtask privacy_inventory` — 31 tests, all pass,
  including **five** new ones for `debug_protections` (a registered type with
  a hand-written `Debug` passes; a registered type that derives `Debug`
  fails; a stale registration fails; an unknown element name fails; a missing
  file fails).
- `cargo run -p xtask -- verify-privacy-inventory` on this tree — **ok — 307
  columns, 10 surfaces, 17 debug_protections**.
- **Mutation proof on the real tree:** temporarily restoring
  `#[derive(Debug)]` on `ChargeRow` makes the gate fail naming `charges.rs:97`;
  reverted. `cargo test -p vpay-db --lib` for the canary modules confirms the
  same mutation fails the canary tests too.

## What was not run

- Full `just ci` / `just verify` (the `verify` recipe's other gates were not
  re-run on this branch; `verify-privacy-inventory` itself was).
- The Postgres-backed integration suites (testcontainers/Docker) and the
  conformance suite.
- `just test-web`, `just test-e2e`, `helm-check`, `just audit-web`.
- `just docs-check-citations` (needs a GitHub token).

## Still true (unchanged by this work)

- `Masked` and `Pseudonymous` have no shipping consumer; RFC-0002 D3 remains
  open.
- No webhook/event payload projections (RFC-0002 D2) — those are PR 5.
- The `logs` non-database surface remains `enumerable: false` — this work
  does not add a static scanner over `tracing!` sites; it makes the types a
  scanner would care about safe by construction and registers them.

## Where the claims landed

- `backends/crates/vpay-core/src/privacy.rs` — the four wrappers + tests.
- `backends/crates/vpay-db/src/{charges,events,idempotency,webhook_deliveries,payment_intents}.rs`.
- `backends/crates/vpay-api/src/model.rs`.
- `backends/crates/vpay-provider/src/lib.rs`.
- `schemas/privacy-inventory.yaml` — the `debug_protections` section.
- `.xtask/src/main.rs` — the gate extension and its tests.
- `docs/reference/personal-data-inventory.md` — "How it is checked" and Status.
- This page.
