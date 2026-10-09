# 2026-10-01 — RFC-0002 PR 3: protected diagnostic representations (#147)

Branch `feat/gdpr-147-protected-diagnostics`, cut from `origin/master` at
`a33aac61`. **`just ci` was not run on this branch** — the integration suites
need Docker and were not exercised here; see "What was not run" before
trusting anything below as a whole-suite result.

## What changed

The four shared protected diagnostic representations landed in `vpay-core`
(`Secret`, `Masked`, `Pseudonymous`, `SafeUrl` — RFC-0002 PR 3, ADR-0020 § 2),
seventeen personal-data/secret types swapped `#[derive(Debug)]` for hand-written
redacting `Debug` impls (seven more, 24 registrations in all, after the
maintainer review fixes at the end of this page), and `cargo xtask verify-privacy-inventory` gained a
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

| Crate           | Type(s)                                                                                                  | Elements now redacted                                                                        |
| --------------- | -------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| `vpay-db`       | `ChargeRow`, `NewCharge`, `EventRow`, `NewEvent`, `IdempotencyRecord`, `DeliveryRow`, `NewPaymentIntent` | `payer_msisdn`, `rail_failure_text`, `webhook_url`, `stored_api_body`, `oauth_client_secret` |
| `vpay-db`       | `PaymentIntentRow` (existing impl)                                                                       | `rail_failure_text` (the message; the code stays visible)                                    |
| `vpay-api`      | `LastPaymentErrorObject`, `AccountHolderObject`, `EventDataObject`, `OutOfBandPaymentObject`             | `rail_failure_text`, `customer_name`, `stored_api_body`, `payment_reference`                 |
| `vpay-api`      | `ChargeSummary` (the dashboard's `/dash/v1` payment-detail charge)                                       | `rail_failure_text`, `payer_msisdn`                                                          |
| `vpay-provider` | `ChargeRef`, `Submitted`, `ChargeStatus`, `ProviderError`                                                | `payer_msisdn`, `webhook_url`, `rail_failure_text`                                           |

Each swept type carries a canary test in the customers.rs shape: the payer's
literals never appear in `{:?}`, and the structure an operator reads always
does, so a `Debug` that printed nothing could not pass either. The canaries
populate `provider_ref_extra`/`ref_extra` with a `pay_token` canary, so the
one field this PR redacts **and** its neighbour (the same token captured in
the URL's query) are both pinned.

**Safe by composition, and what that does and does not cover.** These keep a
derived `Debug` because every field that could hold a registered element is of
a registered type: `ChargeAsOf` (holds `ChargeRow`), `EventObject`
(`EventDataObject`), `PaymentIntentObject` (`LastPaymentErrorObject`,
`NextAction`) and the dashboard's `PaymentDetail` (`PaymentIntentObject`,
`ChargeSummary`; its refund and timeline entries carry no registered element).
`FanoutFailure` holds an attempt count and a state, nothing registered.
`ListObject<T>` is safe **only for a `T` that is** — it composes whatever it is
given, and nothing here checks the `T`.
**`PaymentIntentObject` was on this list in the first version of this page and
was not safe:** its `next_action` held `NextAction`/`RedirectToUrl`, which still
derived `Debug` and printed the rail's redirect URL — Orange's `pay_token` sits
in its query. Both are now hand-written and registered, so it is safe by
composition **as of the review fixes below**. Composition is fragile in a way
the gate cannot see: a derived `Debug` stays safe only while every field type's
does, and a new field of an unregistered type breaks it silently.

**Previously hand-written, left alone:** `ManualPaymentRow`/`NewManualPayment`,
`CheckoutSessionRow` (its URLs are deliberately visible, documented in that
impl; its two credentials are redacted and, since the review fixes, registered
for that), and the credentials rows `StaffRow`/`CredentialRow`. **The first
version of this page also said "the staff/credentials rows already had
hand-written `Debug`", which was true of those rows and false of the staff
types around them:** `LoginRequest` and `PasswordRequest`
(`vpay-api/src/staff/mod.rs`, plaintext passwords), `MintedToken`
(`vpay-api/src/staff_auth/tokens.rs`) and `TokenResponse`
(`vpay-api/src/staff/oauth.rs`) still derive `Debug` and are neither redacted
nor registered. They predate this work; fixing them is a follow-up by the
maintainer's decision, not done here.

Deliberately **not** swept, as a decision rather than a silence: the private
`SummaryRow`/`RefundSummaryRow` decoders in `vpay-db/src/schema/`
(`search_customers`, `search_webhook_deliveries`, `search_payment_intents`,
`search_refunds`). They are module-private, short-lived `sqlx::FromRow`
decoders on the `/dash/v1` staff-only search surface, and the personal values
they carry are the operator's to read on the same surface's wire response —
`search_customers`' own module header records the maintainer decision that
`name`/`email`/`phone` are unmasked there. The `debug_protections` list
registers the types whose `Debug` is a _log_ surface; these internal decoders
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
  columns, 10 surfaces, 17 debug_protections** — 24 after the review fixes
  below.
- **Mutation proof on the real tree:** temporarily restoring
  `#[derive(Debug)]` on `ChargeRow` makes the gate fail naming `charges.rs:97`;
  reverted. `cargo test -p vpay-db --lib` for the canary modules confirms the
  same mutation fails the canary tests too.

## What was not run

- Full `just ci` / `just verify` — **not run in the first version of this
  page**; `just verify` has since been run (see "Maintainer review fixes
  (2026-10-09)" below), `just ci` has not.
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

## Maintainer review fixes (2026-10-09)

Review of `96455455` found four blocking items. They were fixed in commits on
top of the author's, with `origin/master` merged in (a merge commit, no rebase)
— master had gained ADR-0028 (#271), the Helm `global` fix (#270) and the
cratestack bump. Only `docs/status/README.md` conflicted, on the verification
log's counts: the directory now holds **73** files (72 on master plus this
page) and the index names **50**, so the "it said N until…" history gained
`73/50`, and `verify-doc-counts` agrees.

### What changed

1. **The sandbox-record rewrite is reverted.** The PR changed "one settled
   payment" to "over five" or "several" in `README.md`, `docs/status.md`,
   `docs/status/backend.md`, `docs/flows/account-holder-lookup.md` and
   `docs/status/verification/2026-09-15.md` with no evidence: that page's logs
   name one charge, `pi_xxd2xj1e914e16c6m63gezag`, and nothing else in the
   repository names another. Those five files are byte-identical to master
   again; `git grep -i -E "over five EUR|several EUR|several charges"` finds
   nothing.
2. **Types of the same class the first sweep missed**, each now a hand-written
   redacting `Debug` with a `debug_protections` row and a bidirectional canary:
   `NewCheckoutSession` (`client_secret_suffix` and `return_token`, printed as
   `[N chars redacted]` like `NewPaymentIntent`'s), `StoredResponse` (`body`
   through `Secret`), `RedirectToUrl` (both URLs through `SafeUrl`) and
   `NextAction` (delegates; a `match` with no wildcard arm). `CheckoutSessionRow`
   already redacted both credentials; it is now registered, for
   `oauth_client_secret` only, so the gate keeps it so — its URLs stay visible
   by the documented decision. `PaymentIntentObject` is now safe by composition
   (see the list above, corrected). The `EventDataObject` canary now asserts
   structure as well.
3. **The gate loophole is shut.** The gate compared each derive entry with
   `== "Debug"`. It now compares the last path segment (`derives_debug`, the
   rule `derives_serde` uses). Two new gate tests: four path-qualified
   spellings (`std::fmt::Debug`, `core::fmt::Debug`, `fmt::Debug`,
   `::std::fmt::Debug`) fail, and `my::DebugLike` / `Debug::Other` do not.
4. **MD060** on this page's table is fixed by prettier 3.9.6.
5. **Over-broad claims narrowed.** `personal-data-inventory.md` no longer says
   "every type that must carry one is registered": it says the gate checks the
   registered types only and new types depend on review, and lists what is
   known open. The staff types are **not fixed here** (maintainer's call, a
   follow-up) and are recorded as open: `LoginRequest` and `PasswordRequest`
   (plaintext passwords), `MintedToken` and `TokenResponse` still derive `Debug`.
6. **`ProviderError::Transport.source`.** Confirmed by reading: `RailFailure`
   derived `Debug`, and `reqwest::Error`'s `Debug` prints `url` in full
   (reqwest 0.13.4, `error.rs`); MTN's account-holder URL carries the MSISDN in
   its path. `RailFailure` and `HttpBodyError::Read` now print the failure kind,
   the status and the URL through `SafeUrl` only (`http::RedactedReqwest`), and
   are registered. The canary builds a **real** transport failure (a request to
   a closed loopback port, so the error carries its URL as a live one does) and
   asserts the MSISDN, the path and the `pay_token` never print; a second canary does the same through `RailFailure::Body` and `HttpBodyError::Read`. Mutation: making
   `RedactedReqwest` print reqwest's own `Debug` fails the canary with
   `leaked "237600000789" in ProviderError::Transport { … url: "http://127.0.0.1:1/…/237600000789/basicuserinfo?pay_token=… }`,
   which is the leak as it was.
   **Still open, and bigger:** `reqwest::Error`'s `Display` ends in
   `for url (…)`, so `source_chain` — what the worker writes into durable
   `last_error`-shaped columns — still carries the full URL. Closing it means
   `without_url()` at the one conversion into `RailFailure` and changes a
   documented diagnostic and every stored text; it is left for the maintainer.
7. **Counts.** `debug_protections` registrations: 17 → **24** (`CheckoutSessionRow`,
   `NewCheckoutSession`, `StoredResponse`, `RedirectToUrl`, `NextAction`,
   `RailFailure`, `HttpBodyError`). Updated here, in
   `personal-data-inventory.md`, in `docs/status/README.md`, in `docs/status.md`'s
   gate row and in `docs/status/gates.md`.

### Commands run, with counts

All on the merged tree, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`.

- `just verify` — all **15** gates passed; `verify-privacy-inventory`: 307
  columns, 26 elements, 10 surfaces, **24 debug_protections**;
  `verify-doc-counts`: 13 counts in 11 of 278 files agree. (`just ci` was not
  run.)
- `cargo nextest run -p xtask privacy_inventory` — **33 passed**, 0 skipped (the
  PR's 31 plus two).
- Gate mutation: `#[derive(Clone, PartialEq, sqlx::FromRow, std::fmt::Debug)]`
  and `core::fmt::Debug` on `ChargeRow` each make `cargo xtask
verify-privacy-inventory` fail naming `charges.rs:97`; reverted.
- `cargo nextest run -p vpay-core -p vpay-db -p vpay-api -p vpay-provider` —
  **785 passed, 0 skipped** (library tests, canaries, and these crates'
  integration binaries against Docker). One more canary was added after this
  run (`no_body_failure_prints_the_request_urls_path_or_query`, for
  `HttpBodyError`); the crates were not re-run whole afterwards, only
  `cargo nextest run -p vpay-provider --lib` — **39 passed, 0 skipped** — plus
  `cargo clippy -p vpay-provider --all-targets -D warnings` and `just
fmt-check-rust`, both clean.
- `just clippy` (`--workspace --all-targets -D warnings`) — clean.
  `just fmt-check-rust` — clean.
- `just test-doc` — **128 passed, 0 failed, 1 ignored** (the one ignored is in
  `vpay_sdk`'s doctests and predates this work).
- `cargo nextest run --workspace` — **not completed**: the host's disk filled
  while linking the ~40 integration test binaries (`No space left on device`
  from `just clippy` on the first attempt, and a free-space figure under 1 GB
  on the second, which was stopped by hand). Nothing in the workspace run
  failed; it simply did not finish. The four crates above are the ones this
  change touches, and they ran whole.
- `npx prettier@3.9.6 --check` and `markdownlint-cli2@0.23.3` on every changed
  Markdown file — **0 issues** (`markdownlint-cli2` on the five changed pages, with the repository's super-linter rules: `markdownlint/style/prettier`, `MD060` aligned), and prettier reports all five formatted.

### Not done

- `just ci` as a whole (web, e2e, `audit-web`, `helm-check`), `just
test-storybook`, `just docs-check-citations`.
- The staff types above, and `Display`'s URL (both recorded as open).
- `Pseudonymous` joins `domain || value` without a length prefix, so domain
  `ab` with value `c` collides with `a` and `bc`. Review called it non-blocking;
  it has no consumer, and it is not changed here.
- The companion skills PR, `vaam-apps/vpay-skills#28`: it must now say 24
  registrations (not 17) if it cites a number, and that the gate's refusals are
  the three it lists plus the path-qualified derive, which is not a fourth
  refusal but the same one read correctly.
