<!-- Per-lane notes for Step 9 (docs/plans/2026-09-04-step9-hosted-checkout.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. Everything in §6 and §7 below is written so it can be applied verbatim. -->

# Step 9, lane 1 — the session object and both surfaces

Branch `claude/step9-lane-1-sessions`, on top of `f186d67` (master + the Step 9
plan). Two commits: `30341ee` (the object) and `8c11c17` (the suite and the
counters), plus this note.

## 1. What landed

| # | Thing | Where |
|---|---|---|
| 1 | Migration `0028` — `checkout_sessions`, three closed vocabularies, `urls_match_ui_mode`, both credential-length CHECKs, the partial unique index and three read indexes | `backends/migrations/0028_create-checkout-sessions.sql` |
| 2 | `vpay_core::ids` — `CHECKOUT_SESSION_PREFIX`/`checkout_session_id()`, `return_token()`, and `secret_body()` shared with `client_secret_suffix()` | `backends/crates/vpay-core/src/ids.rs:46` (prefix), `:196` (id), `:283` (`secret_body`), `:339` (`return_token`) |
| 3 | `vpay_db::checkout_sessions` — `CheckoutSessionRow` (redacting `Debug`), `NewCheckoutSession`, `SessionListPage`, the `CheckoutSessions` trait and its `PgRepositories` impl | `backends/crates/vpay-db/src/checkout_sessions.rs`; registered at `backends/crates/vpay-db/src/lib.rs:20`, added to the `Repositories` umbrella at `backends/crates/vpay-db/src/repository.rs:437` |
| 3b | `CheckoutSessionRow::publishable_key` (a column, migration `0028`) and `CheckoutSessionRow::return_page_url(checkout_base)` — the return-page URL lane 2 hands a rail, built in one place | `backends/crates/vpay-db/src/checkout_sessions.rs:115` (the method), `:214` (the field) |
| 4 | The settlement flip — `checkout_sessions::settle_for_intent(tx, intent_id, paid)`, `pub(crate)`, called **inside** the settlement transaction | `backends/crates/vpay-db/src/checkout_sessions.rs:303` (the write), `backends/crates/vpay-db/src/settlement.rs:166` (`flip_session`), called at `:411` and `:466` |
| 5 | `/v1/checkout/sessions` — create, retrieve, list, expire; every URL rule; `Idempotency-Key` through the shared `PostRequest` | `backends/crates/vpay-api/src/v1/checkout_sessions.rs`; `V1_ROUTES` at `backends/crates/vpay-api/src/v1/mod.rs:170` |
| 6 | `/v1/browser/checkout/{sessions/{id},sessions/{id}/return,origins}` | `backends/crates/vpay-api/src/browser/checkout_sessions.rs`; `BROWSER_ROUTES` at `backends/crates/vpay-api/src/browser/mod.rs:136` |
| 7 | `CheckoutSessionObject`, `CheckoutSessionWithSecret` (redacting `Debug`, **including the `url`**), `ExpandableIntent`, `CheckoutSessionTag` | `backends/crates/vpay-api/src/model.rs:74` (tag), `:388` (`ExpandableIntent`), `:452` (object), `:566` (with-secret) |
| 8 | `ApiError::CheckoutNotConfigured`, code `checkout_not_configured` | `backends/crates/vpay-api/src/error.rs:381` |
| 9 | `ResourceConfig::checkout_public_base_url()`, `checkout_origins_for()`, `publishable_keys_for()` | `backends/crates/vpay-api/src/v1/mod.rs:453` (fields), `:640` (accessors) |
| 10 | `vpay_config::CheckoutConfig`, `MerchantClient::checkout_origins`, six named `ConfigError` variants, two validators, six fixtures | `backends/crates/vpay-config/src/config.rs:388` (`CheckoutConfig`), `:1204` (`validate_checkout_base_url`), `:1303` (`validate_checkout_origins`); `backends/crates/vpay-config/src/oauth.rs:333` (the field); `backends/crates/vpay-config/src/lib.rs:585` (the variants); `backends/crates/vpay-config/tests/fixtures/checkout-*.yml` |
| 11 | The samples | `config/application.yml` (`checkout:` block, `checkout_origins`), `config/application-sandbox.yml` |
| 12 | Integration suite — 11 cases | `backends/tests/integration/tests/checkout_sessions.rs` |
| 13 | Reference pages | `docs/reference/vpay-db.md` (new §`checkout_sessions`), `docs/reference/vpay-api.md` (new §"Checkout Sessions"), `docs/reference/vpay-config.md` (new §"The `checkout:` block"), `docs/reference/vpay-core.md` (new §"`return_token` is a second *capability*") |
| 14 | Counters | `justfile`: `expected_suites` 41 → 42, `min_tests` 1000 → 1050 |

## 2. Decisions taken in this lane, and why

- **The settlement flip lives in `vpay-db`'s transaction, not in
  `vpay-worker/src/handlers.rs`.** The plan places "the worker hook" in
  `handlers.rs`. The *decision* is the worker's — `settle_succeeded` or
  `settle_failed` — but the settlement transaction itself is
  `vpay_db::settlement::apply_{succeeded,failed}`, and a write made after
  that commit would leave a window in which the intent is `succeeded` and the
  session still `open`/`unpaid`, permanently if the process died in it. There
  is no job that would notice and D10 adds none. `settle_for_intent` is
  therefore `pub(crate)` in `vpay-db` and reachable only from `settlement`,
  the same device `payment_intents::succeed_after_submission` uses. **No file
  under `vpay-worker/` was changed.**
- **`payment_intent` is expanded in place on the browser reads**
  (`ExpandableIntent`, `#[serde(untagged)]`), not carried beside the session
  in an envelope. The integrator's binding clarification asked for the
  expanded object on those two routes and the id on `/v1`; expansion *on the
  field* is Stripe's own `expand` shape, avoids two keys named
  `payment_intent` at two nesting levels, and makes "with or without the
  intent's credential" a choice of enum variant rather than a field a handler
  clears.
- **Neither browser read renders the session's `url`.** It carries the
  session's `client_secret` in its fragment, and the return read is authorised
  by the *weaker* `return_token`. Echoing it there would be a three-step
  escalation out of a query-string value: `return_token` → session secret →
  the session read → the intent's `client_secret` → `confirm`. It costs the
  page nothing.
- **`expire` guards on a live charge inside the `UPDATE`**, over the shared
  `LIVE_CHARGE_STATES`, exactly as `payment_intents::cancel` does. A session
  expired while a rail holds a live payment would then be contradicted by the
  settlement transaction flipping the same row to `complete`/`paid`.
- **The session pins a publishable key, and it is a column.** Every URL vpay
  mints carries `?key=`, because all three browser routes authenticate by it
  and the return page cannot use a fragment. `create` takes an optional
  `publishable_key`, defaults to the tenant's *first configured* key, refuses
  one that is not theirs with a `400`, and answers `checkout_not_configured`
  (a second sentence under the same code) for a tenant with none. It is
  **stored** rather than derived at render time so a key rotation cannot
  strand a payer already on a rail's page — the full argument is on the column
  in `0028` and in `docs/reference/vpay-db.md`.
- **`return_page_url` is a method on the row, not a `format!` in `vpay-api`.**
  Two callers build that URL and a *rail* holds the copy that matters, so the
  two must agree byte for byte. It carries a compiled doctest.
- **`get_for_merchant` call sites are now fully qualified.** Two traits offer
  the name, so `PaymentIntents::get_for_merchant(repos, …)` at three sites in
  `v1/payment_intents.rs` and four in `vpay-db/tests/repositories.rs`. That
  file is lane 2's; the change is three mechanical lines and matches the
  style the file already uses for `PaymentIntents::list_page`.
- **`PostRequest`/`ClaimOutcome` widened to `pub(crate)`** rather than copied.
  A second claim/finish/release dance is how one of the two POSTs ends up
  leaving a merchant's key stuck `in_flight`.
- **The `it names no host` branch in both config validators is unreachable**
  and is kept anyway, with `a_hostless_http_url_never_reaches_the_host_branch`
  proving it — a tripwire on `url`, not on vpay.

## 3. Where this deviates from the brief, and what is not done

### 3a. `checkout_not_configured` answers **500**, not 503

The `code` is what the plan asked for. The **status is not**, deliberately.

ADR-0011 derives the status from the `Category` and never from a call site,
and `Category::Storage` is the only category that answers `503`. Classifying
"this deployment configures no checkout page" as storage would tell an
operator Postgres was unreachable — and, worse, `Category::Storage`'s
`Retry::AfterBackoff` would tell a merchant's SDK to retry a request that
cannot succeed until someone deploys. `Category::Configuration` says exactly
what is true and its status is `500`.

A truthful `503` needs either a new `Category` or `Category::Configuration`
moving to `503`. Both are ADR-level changes touching every error in the
workspace. **That is a maintainer's decision and is not taken here.** The full
argument is on the variant (`error.rs`) and in
`docs/reference/vpay-api.md` §"`checkout_not_configured` answers 500".

### 3b. The merchant's display name is **not** rendered — not built

The wire contract says the browser session read answers "the session plus its
intent … **and the merchant's display name**". There is no display name
anywhere in `vpay-config`: `MerchantClient` has `client_id` and `merchant_id`,
and neither is a name a payer should see. My brief's config list is
`checkout.public_base_url` and `checkout_origins` and nothing else, so I did
not add a third field, and I did **not** render `merchant_id` under a
`display_name` key — a tenant slug shown to a payer as "who you are paying" is
exactly the plausible-looking fabrication AGENTS.md's second rule forbids.

**Lane 3 will need it.** The smallest honest fix is
`merchant_clients[].display_name: "Acme Cameroon"` in `vpay-config`, projected
through `ResourceConfig` and added to the two browser reads. It is one field,
one validator (non-empty, bounded) and one fixture. Flagged for the
orchestrator rather than invented.

### 3c. Not done, and out of lane

- `ChargeRef::return_url` and populating it from `find_open_by_intent` — lane
  2. The trait method exists and its contract is documented in
  `docs/reference/vpay-db.md` for exactly that, and
  `CheckoutSessionRow::return_page_url(checkout_base)` builds the URL so
  neither caller has to. Lane 2's `v1/return_trip.rs` `session` branch, which
  the integrator says currently answers `Ok(None)` with a doc comment naming
  `find_open_by_intent`, should read:
  `Ok(session.map(|row| row.return_page_url(base)))` where `base` is
  `ResourceConfig::checkout_public_base_url()`. **Nothing in this lane wires
  that branch** — the file is not on this branch.
- `frontends/apps/checkout` — lane 3. Nothing here serves HTML or sets a CSP
  header; the origins route answers the list a middleware turns into one.
- The SDKs' `checkout.sessions.*` — lane 5. The integration suite drives raw
  HTTP for the merchant half and says so in a comment, because `vpay-sdk`
  models no such resource yet.
- **No expiry sweep.** `expires_at` is stored and rendered, and nothing reads
  it: a session past its horizon still says `open` until something expires it.
  D10 names the 24-hour horizon but no job, and I did not add one. See §6 for
  the status row.
- `docs/status.md`, `docs/roadmap.md` and `docs/flows/*` are untouched — §6
  and §7 carry the text.

## 4. Measured, on this branch

Host: the lane worktree, `CARGO_BUILD_JOBS=4`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`.

| Gate | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-config -p vpay-core -p vpay-worker --retries 2 -j 1` | **560 run, 560 passed, 0 skipped** (156.8 s) |
| `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(browser_checkout) \| binary(confirm_rails)' --retries 2 -j 1` | **30 run, 30 passed, 0 skipped** — 13 + 10 + 7 |
| `just verify` | ok — `verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity` all pass; `verify-docs` is the advisory report |
| `just verify-ignored` | **0 ignored (expected 0), 42 test binaries (expected 42), 1098 total (minimum 1050)** |
| `just test-doc` | **82 doctests, 82 passed, 0 failed** |

`cargo xtask verify-errors` is green with `ApiError::CheckoutNotConfigured`
added: the variant is on an enum that already `impl Classify`, and the six new
`ConfigError` variants are on one that does too.

No container-start flakes were observed on this host across the runs above;
every integration run went green on the first try.

`verify-docs` reports one new production function of 80 lines or more —
`v1/checkout_sessions.rs:382 fn validate_create`, 86 lines. It is one
`match ui_mode` over two arms, each five short calls; splitting it would put
"which URLs belong to which mode" in two places. Recorded rather than
suppressed; the report is advisory and fails nothing.

## 5. Guard-failure proofs

Each mutation was applied, the named test run, and the file restored from a
copy taken first. `git status --porcelain` is empty after all six.

| # | Mutation | Test | Observed |
|---|---|---|---|
| 1 | Delete the `session.merchant_id != merchant_id` arm from `browser::checkout_sessions::authenticate` | `every_credential_failure_on_the_checkout_surface_is_the_identical_404` | **FAIL** — "another merchant's publishable key, valid and registered" answered `200` instead of `404`, rendering merchant A's session *and the intent's `client_secret`* to merchant B's key |
| 2 | Change `retrieve_for_return` to build `ExpandableIntent::ExpandedWithSecret` | `the_session_read_carries_the_intents_secret_and_the_return_read_does_not` | **FAIL** — the return read rendered `client_secret` on the expanded intent |
| 3 | Remove the `flip_session` call from `vpay_db::settlement::apply_succeeded` | `the_settlement_transaction_flips_the_session_with_the_intent` | **FAIL** — intent `succeeded`, session still `("open", "unpaid")`; expected `("complete", "paid")` |
| 4 | Remove the `NOT EXISTS` live-charge clause from `checkout_sessions`' `expire` | `expiring_a_session_is_a_compare_and_swap_and_a_live_charge_refuses_it` | **FAIL** — a session whose intent had a live charge expired with `200` instead of `409` |
| 5 | `CREATE UNIQUE INDEX` → `CREATE INDEX` for `checkout_sessions_one_open_per_intent` in migration `0028` | `an_intent_may_have_only_one_open_session` | **FAIL** — the second insert succeeded, leaving one intent with two open sessions and two live payer links |
| 6 | Delete the `registered.iter().any(...)` check from `chosen_publishable_key`, accepting any key the merchant names | `a_session_pins_the_tenants_first_key_unless_the_merchant_names_another` | **FAIL** — merchant A minted `…/c/cs_…?key=pk_test_betadoualasandbox0001#…`, a link whose page would authenticate against **merchant B's** tenant and answer the uniform 404 to every payer. `201` instead of `400` |

Proofs 1, 2, 5 and 6 are the security-relevant ones. 1 and 2 are the two the
brief named; 5 is the one that would otherwise look like it was enforced by
the `find_open_by_intent` pre-check, which is not a guard; 6 is the one the
publishable-key ruling introduced.

## 6. `docs/status.md` — rows for lane E

### 6a. New row (Payments / API surface table)

| Checkout Sessions — `/v1/checkout/sessions` and `/v1/browser/checkout/*` (`vpay_api::v1::checkout_sessions`, `vpay_api::browser::checkout_sessions`) | 🟡 | **New 2026-09-04 (Step 9 lane 1).** A `checkout.session` (`cs_…`, migration `0028`) a merchant creates from its server against an existing `pi_…` — `create`/`retrieve`/`list`/`expire` on `/v1` (token-authenticated, `Idempotency-Key`, tenant-scoped), and three `GET`s on `/v1/browser/checkout` for the page. **Two payer credentials, not one** (D6): `client_secret` rides in the hosted `url`'s *fragment* and buys the intent's own `client_secret`; `return_token` rides in the return page's *query string* — it must, a fragment does not survive a rail's redirect — and buys the session and its intent **without** that credential. Every failure on both browser reads is the byte-identical uniform 404, including the tenancy case, and neither read renders the `url` (it carries the stronger credential in its fragment). `create` refuses an intent that is not `requires_payment_method`, one that already has a charge, and one that already has an open session — the last enforced by a **partial unique index**, not only by the pre-check. `expire` is a compare-and-swap with a `NOT EXISTS` live-charge guard in the same statement, so a session cannot be marked abandoned while a rail may still take the payment. The settlement transaction (`vpay_db::settlement`) flips `payment_status`/`status` in the **same commit** as the intent — `paid`/`complete` on success, `failed`/`expired` on a terminal decline. Proven by `backends/tests/integration/tests/checkout_sessions.rs` — 11 container-backed cases against real Postgres, the real WireMock MTN rail and the shipping worker loop, with five recorded guard-failure proofs (`docs/plans/step9-notes/lane-1.md` §5). Still 🟡 and not ✅: **no page exists yet** (lane 3), the return trip through the port is not wired (lane 2), no SDK models the resource (lane 5), and no expiry sweep runs — see the two rows below. |

### 6b. New row (Configuration table)

| `checkout.public_base_url` and `merchant_clients[].checkout_origins` (`vpay_config`) | ✅ | **New 2026-09-04 (Step 9 lane 1).** `checkout.public_base_url` is the origin (optionally with a path prefix) every payer link is built on; **absent is a complete answer** — a deployment that omits it serves no checkout page and `POST /v1/checkout/sessions` answers `checkout_not_configured` rather than minting a `url` that resolves to nothing. `checkout_origins` is the per-merchant list `frame-ancestors` is derived from; an empty list (the default) means no site may embed. Six named `ConfigError` variants with one fixture each under `backends/crates/vpay-config/tests/fixtures/checkout-*.yml`: `MalformedCheckoutBaseUrl`, `InsecureCheckoutBaseUrl`, `MalformedCheckoutOrigin`, `InsecureCheckoutOrigin`, `DuplicateCheckoutOrigin`, `CheckoutOriginsWithoutBaseUrl`. Both are `https`-only under `deployment.livemode`, checked at boot. `config/application.yml` carries a worked example of each. |

### 6c. New row (the honest gap)

| Checkout session expiry sweep | ⛔ | **Not built.** `checkout_sessions.expires_at` is stored at create (24 h, D10) and rendered on every response, and **nothing reads it**: a session past its horizon still reports `status: open` until a merchant calls `POST /v1/checkout/sessions/{id}/expire` or the intent settles. D10 fixes the horizon and names no job, and Step 9 lane 1 added none. The consequence is bounded — an expired-but-`open` session's `url` still works, and the payment behind it is still guarded by `one_charge_per_intent` — but a merchant reading `status` cannot tell "still payable" from "long abandoned". A `sweep_expired`-style job over `checkout_sessions_open_by_intent_idx` is the shape; it needs a new `jobs.kind` and therefore a migration. |

### 6d. Amendments to existing rows

- **Migration count.** `backends/migrations` now holds **28** files.
  `postgres_smoke.rs`'s assertion was moved 27 → 28 in the same commit, and
  `checkout_sessions` was added to its queryable-tables list. Measured: that
  binary is **14 run, 14 passed, 0 skipped** on this branch.
- **Test counts.** `cargo nextest list --workspace`: **1098 total, 42 test
  binaries, 0 ignored** (was 1059 / 41 / 0). `justfile`'s `expected_suites`
  → `42` and `min_tests` → `1050`, both bumped in this lane's commit with the
  reasoning in the recipe's own comment.
- **Doctest count.** `cargo test --doc --workspace`: **82 passed** (four new:
  `CheckoutConfig` twice, `ids::return_token`,
  `CheckoutSessionRow::return_page_url`).
- **`BROWSER_ROUTES` is five entries, not two.** Any status text saying "two
  routes" about `/v1/browser` is now wrong. The property that replaced the
  count: *exactly one entry answers a non-`GET` method, and it is the confirm
  that has been there since Step 5c* — Step 9 added no second way to move
  money.

## 7. Flow-doc lines to retire (lane E)

Each is quoted verbatim as it stands today, with the replacement.

### `docs/flows/browser-checkout.md`, D2 (line 36)

**Retire:**
> - **D2 — `client_secret` is rendered only by `create`, `retrieve`, and the two
>   browser routes**, through a wrapper type that never touches the object
>   every other response renders:

**Replace with:**
> - **D2 — `client_secret` is rendered only by `create`, `retrieve`, the two
>   payment-intent browser routes, and — since Step 9 — the checkout
>   **session** read** (`GET /v1/browser/checkout/sessions/{id}`, which hands
>   the page the intent's credential once it has proved it holds the
>   *session's*). It is rendered through a wrapper type that never touches the
>   object every other response renders:

### `docs/flows/browser-checkout.md`, "The routes" table (line ~131)

**Retire the table's introduction and add a row.** After the two existing
rows, append:

> | GET | `/v1/browser/checkout/sessions/{id}` | `key`, `client_secret` (query) | The `checkout.session`, with `payment_intent` **expanded and carrying the intent's own `client_secret`** — what vpay's own page reads before it can paint. Step 9. |
> | GET | `/v1/browser/checkout/sessions/{id}/return` | `key`, `t` (query) | The same, with `payment_intent` expanded **without** the intent's secret. Where a redirect rail sends the payer back. Step 9. |
> | GET | `/v1/browser/checkout/origins` | `key` (query) | `{"origins": [...]}` for the key's tenant, with no secret at all — an origin is the merchant's own public website. Step 9. |

### `docs/flows/browser-checkout.md`, the mounting paragraph (line ~135)

**Retire:**
> (its own table is `BROWSER_ROUTES`, exactly two entries)

**Replace with:**
> (its own table is `BROWSER_ROUTES`, five entries since Step 9 — and the
> property that pin defends is not the count but that **exactly one of them
> answers a non-`GET` method**, the confirm that has been there since Step 5c)

### `docs/flows/browser-checkout.md`, "There is no `create`…" (line ~137)

**Retire:**
> There is no `create`, no `list`, and no `cancel` here — proved by
> `the_browser_surface_has_no_create_no_list_and_no_cancel` — and no route
> answers `401` (`every_browser_route_is_reachable_without_a_merchant_token`).

**Replace with:**
> There is no `create`, no `list`, and no `cancel` here — proved by
> `the_browser_surface_has_no_create_no_list_and_no_cancel` — and no route
> answers `401` (`every_browser_route_is_reachable_without_a_merchant_token`,
> which also pins the table's contents and that only the confirm writes).
> Step 9's three additions are all reads, and vpay's own checkout page
> confirms through the *same* `POST /v1/browser/payment_intents/{id}/confirm`
> a merchant's page does.

### `docs/flows/browser-checkout.md`, "Every failure is the same 404" (line ~114)

**Append after the existing paragraph:**
> Step 9 added a second uniform 404 on the same surface, for the checkout
> session (`browser::checkout_sessions`): five ways to refuse — unknown key,
> session not found, session belongs to a different merchant, wrong
> credential, missing credential — and one byte-identical
> `ApiError::NotFound { resource: "checkout session" }`. The noun differs from
> `/v1`'s `checkout.session` deliberately, for the reason the payment-intent
> one says `payment intent` with a space. Proven byte-for-byte by
> `every_credential_failure_on_the_checkout_surface_is_the_identical_404`,
> which also asserts that neither of the session's two credentials is accepted
> where the other belongs.

### `docs/flows/payment-lifecycle.md`, "What still has never happened" (line ~172)

**Append to the paragraph, before "See ../status.md":**
> Since Step 9 the settlement transaction also moves a **checkout session**
> when one drove the payment — `paid`/`complete` on success, `failed`/`expired`
> on a terminal decline, in the same commit as the intent's own status, so the
> two can never be observed disagreeing
> (`vpay_db::checkout_sessions::settle_for_intent`, called from
> `vpay_db::settlement`). Nothing else about the lifecycle changed: a session
> is a *view* of one checkout attempt and moves no money.

## 8. What I did not do

- Did not touch `PaymentIntentObject`'s twelve keys; its tripwire test is
  unchanged and still asserts exactly twelve.
- Did not touch the CORS layer, any adapter, `vpay-provider`, `docs/status.md`
  or `docs/flows/*`.
- Did not touch `vpay-worker` at all — see §2 for why the settlement hook is
  in `vpay-db` instead.
- Did not add a merchant display name (§3b), an expiry sweep (§6c), or a `503`
  status for `checkout_not_configured` (§3a).
- Did not run `just ci`, `just demo` or the e2e suite: the plan puts the full
  gate in the `vpay-ci` VM on the merged branch, and lane 4 has not built the
  checkout service yet.
- Did not run `just ci` end to end, `just demo`, the conformance suite or the
  Cypress specs — see the row above.
