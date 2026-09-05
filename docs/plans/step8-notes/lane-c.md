<!-- Per-lane notes for Step 8 (docs/plans/2026-09-03-step8-production-gate.md).
Lane E — the orchestrator — is the only editor of docs/status.md,
docs/roadmap.md and docs/flows/*.md, and takes its edits from files like this
one. Everything in §4 and §5 below is written so it can be applied verbatim. -->

# Step 8, lane C — the rail callback route

Branch `claude/step8-lane-c-callback`, on top of `93c6a1c` (master `572a89f`
+ Step 7 + the Step 8 plan).

## 1. What landed

| # | Thing | Where |
|---|---|---|
| 1 | `vpay_api::provider_callback` — `POST /provider/{code}/callback`, mounted on its own nest | `backends/crates/vpay-api/src/provider_callback.rs`, mounted at `backends/crates/vpay-api/src/lib.rs:988` (`.nest(PROVIDER_NEST, provider)`) |
| 2 | `Charges::get_by_provider_reference(provider_code, reference)` — a new trait method beside its neighbours | `backends/crates/vpay-db/src/charges.rs:433` (trait), `:474` (impl) |
| 3 | `TxRepositories::pull_forward_in_tx(dedupe_key)` — `UPDATE jobs SET run_at = now()`, guarded three ways | `backends/crates/vpay-db/src/jobs.rs:165` (query), `backends/crates/vpay-db/src/repository.rs:222` (trait), `:321` (impl) |
| 4 | Migration `0027`: `charges_provider_reference_idx ON charges (provider_code, provider_reference_id)` | `backends/migrations/0027_charges-provider-reference-idx.sql` |
| 5 | `POLL_CHARGE_KIND` / `poll_dedupe_key` widened to `pub(crate)` so the callback route spells the ladder's key once, not a third time | `backends/crates/vpay-api/src/v1/payment_intents.rs:1447`, `:1556` |
| 6 | The WireMock tree now **requires** the callback URL on every accepted submit (MTN header, Orange body field) | `backends/tests/conformance/wiremock/mtn/mappings/requesttopay.json`, `.../requesttopay-scenario.json`, `.../orange/mappings/webpayment.json` |
| 7 | Conformance case `the_submit_tells_the_rail_where_to_call_back` (×2 rails) | `backends/tests/conformance/tests/adapter_conformance.rs:587` |
| 8 | Integration suite `provider_callback.rs` — 9 cases | `backends/tests/integration/tests/provider_callback.rs` |
| 9 | Reference docs | `docs/reference/vpay-api.md` (new §"The rail callback route"), `docs/reference/vpay-db.md` (new §"`pull_forward_in_tx` is the exception"), `docs/reference/rails.md` (new §"The callback URL is a contract the mappings hold") |

**Item 2 of the plan (`X-Callback-Url`/`notif_url` sent on `submit`) was
already built.** `vpay_config::ProviderHost::effective_callback_url` has
derived `{public_base_url}/provider/{code}/callback` since Step 3 and both
adapters have been sending it (`backends/crates/vpay-adapter-mtn-momo/src/lib.rs:436`,
`backends/crates/vpay-adapter-orange-money/src/lib.rs:375`). What was missing was the route it
pointed at and any assertion that it kept being sent. Both are here; no
adapter behaviour changed.

## 2. Decisions taken in this lane, and why

- **`/provider`, not `/v1/provider`.** Not part of the merchant API, carries
  no resource version, and mounting it under the one prefix whose whole
  boundary is "everything here needs a bearer token" would put an
  unauthenticated route inside it. The path was also not free: it is the one
  `effective_callback_url` has been deriving since Step 3.
- **`pull_forward_in_tx` is a new method, not `enqueue_in_tx` growing a
  `DO UPDATE`.** The argument against the upsert
  (`docs/reference/vpay-db.md`) is unchanged — the backstop scan re-enqueues
  every live charge's key every ten minutes, and an upserting enqueue would
  drag a job scheduled a quarter of an hour out back to now on every pass.
  One caller asking for the pull-forward is a callback; every caller getting
  it is a hot loop.
- **It refuses a leased job, an already-due job and a parked job.** The last
  is the sharp one: `docs/reference/vpay-db.md` §"Why a dead letter is parked"
  already said the occupied `dedupe_key` is what keeps a scan *or a callback*
  from re-creating the work, so the `run_at < 'infinity'` guard is that
  sentence being made true.
- **`CallbackRef::ref_extra` is discarded.** Orange's `parse_callback` carries
  a `notif_token` (and sometimes a `pay_token`) out of the notification, and
  repairing a charge's lost `ref_extra` from a callback needs the *stored*
  `notif_token` compared against the received one first. That comparison is
  not built. Merging unverified rail key material onto the row would corrupt
  the token the next status query is addressed by, so the honest state is
  "not built" — §4 has the status row.
- **A new migration was added** rather than leaving the unauthenticated
  lookup on a sequential scan over `charges`. Blast radius: it moved
  `postgres_smoke.rs`'s migration-count assertion from 26 to 27
  (`backends/tests/integration/tests/postgres_smoke.rs:151`), which is the
  only file outside this lane's own scope that had to change.

## 3. **Left to the maintainer, deliberately not decided here**

`charges.provider_reference_id` has **no unique constraint**, and `0027`'s
index is deliberately not `UNIQUE`. Every insert path mints the reference with
`Uuid::new_v4()` before committing, so "one charge per rail reference" appears
to be true by construction — but it is a schema-level invariant this
repository has never claimed, and taking it as a side effect of adding a route
would be deciding something reserved for whoever owns the schema. The lookup
is written to be total without it (`ORDER BY created_at DESC, id DESC
LIMIT 1`, with the reasoning on the trait method), and the blast radius of an
ambiguous match is one extra *authenticated* status query. **Recommendation:
make it `UNIQUE (provider_code, provider_reference_id)` in a commit of its
own, with a test that the constraint fires.**

## 4. `docs/status.md` — rows for lane E

### 4a. New row (Payments / API surface table, beside the confirm row)

| Rail callbacks — `POST /provider/{code}/callback` (`vpay_api::provider_callback`) | 🟡 | **New 2026-09-04 (Step 8 lane C); was ⛔ "no callback route exists".** The route is mounted, unauthenticated by necessity (neither rail signs a callback or sends a shared secret), and it **never writes charge or intent state**: it resolves the adapter by `code`, `parse_callback`s the body into identifiers, finds the charge with `Charges::get_by_provider_reference` (scoped by rail, served by migration `0027`'s index), and performs exactly one write — `TxRepositories::pull_forward_in_tx`, an `UPDATE jobs SET run_at = now()` on that charge's existing `poll:<charge id>` job, refusing a leased, already-due or dead-lettered one. Four answers: unknown rail code → `404` byte-identical to the router's fallback; unparseable body → `400` + a `warn`; unknown reference → `202` (a rail must not be told to retry forever, and a different answer would be an oracle); accepted → `202`. Body bounded at 16 KiB by `RequestBodyLimitLayer`, `track_http_metrics` on the nest, its own `.fallback`, no CORS, `request-id` on the way out like every other route. Proven by `backends/tests/integration/tests/provider_callback.rs` — 9 container-backed cases over both rails, including the headline one: the worker's first poll parks the job ten seconds out, `run_once` then finds **nothing runnable**, the rail's documented body is POSTed to the URL read back off the rail's own WireMock journal, and the next `run_once` settles the charge, whole round trip under 8 s against a `poll_delay(0)` of 10 s. Still 🟡 and not ✅ for two reasons: **no real rail has ever called it** (the bodies are transcribed from `docs/flows/adapter-*.md`, so a document that is wrong about the rail would pass), and Orange's `notif_token` is **not** compared against the stored one — see the row below. |

### 4b. New row (or an amendment to the Orange adapter row)

| Orange `notif_token` verification, and `ref_extra` repair from a callback | ⛔ | **Still not built, and now explicitly so.** `vpay_adapter_orange_money::Adapter::parse_callback` returns the received `notif_token` in `CallbackRef::ref_extra` and fails closed when there is none, and `docs/flows/adapter-orange-money.md` names comparing it against the stored one as "the callback route's job". The route (new 2026-09-04) **discards `ref_extra` entirely** rather than doing half of it: merging rail key material taken from an unauthenticated request onto the charge would corrupt the `pay_token` the next status query is addressed by, and the comparison that would make it safe needs a stored-token read nothing implements. `a_callback_writes_no_charge_or_intent_state` in `backends/tests/integration/tests/provider_callback.rs` holds it there, on both rails, with a body carrying tokens no rail ever issued. Repairing a lost `ref_extra` write from a callback therefore remains unavailable; the crash-safety answer for that case is unchanged (`ProviderError::Config`, a human). |

### 4c. Amendments to existing rows

- **`docs/status.md:1645-1646`** — "and the callback path, because no callback
  route exists" is now false. Replace with: *"and the callback path end to
  end: the route exists and is proven against WireMock
  (`provider_callback.rs`), but no real rail has ever called it, and Orange's
  `notif_token` is still not compared against the stored one."*
- **Migration count.** `backends/migrations` now holds **27** files;
  `postgres_smoke.rs`'s assertion was moved with it in the same commit.
- **Test counts.** `cargo nextest list --workspace`: **1016 total, 40 test
  binaries, 0 ignored** (was 999 / 39 / 0). `justfile`'s `expected_suites`
  → `40` and `min_tests` → `980`, both bumped in this lane's commits with the
  reasoning in the recipe's own comment.
- **Conformance count.** `cargo nextest run -p vpay-tests-conformance`:
  **28 tests, 28 passed, 0 skipped** (was 26) — 12 cases over both rails plus
  the same 4 that are not rail-specific.

## 5. Flow-doc lines to retire (lane E)

Each is quoted verbatim as it stands today, with the replacement.

### `docs/flows/adapter-mtn-momo.md`, "Not proven" list

**Retire:**
> * **No callback route exists.** `parse_callback` is implemented and tested,
>   and nothing in a running vpay calls it — and when something does, MTN
>   signs nothing, so it will still be a hint.

**Replace with:**
> * **The callback route exists (Step 8), and nothing has ever called it but
>   this repository's own tests.** `POST /provider/mtn_momo/callback`
>   (`vpay_api::provider_callback`) parses this document's notification body
>   into identifiers and pulls the charge's poll job forward; MTN signs
>   nothing, so it is still a hint and the route writes no charge state.
>   `backends/tests/integration/tests/provider_callback.rs` POSTs the body
>   transcribed above to the URL MTN was handed on the submit, so a body
>   faithful to this document but not to MTN would pass.

### `docs/flows/adapter-orange-money.md`, "Not proven" list

**Retire:**
> - `notif_token` equality is **not** performed by the adapter — it holds no
>   state. `parse_callback` returns the received `notif_token` in `ref_extra` and
>   fails closed when there is none; comparing it with the stored one is the
>   callback route's job, and that route is not built yet.

**Replace with:**
> - `notif_token` equality is **not** performed by the adapter — it holds no
>   state — and, since Step 8, **not by the callback route either**.
>   `parse_callback` returns the received `notif_token` in `ref_extra` and
>   fails closed when there is none; `vpay_api::provider_callback` discards
>   that `ref_extra` rather than merging unverified rail material onto the
>   charge, so the comparison is still unbuilt and `ref_extra` repair from a
>   callback is still unavailable. The adapter's fail-closed check is now
>   load-bearing in production, not only in tests: it is the only thing
>   between an unauthenticated POST and a queued poll
>   (`docs/reference/rails.md`).

### `docs/flows/reconciler.md`, Status intro (line 63)

**Retire:** `unbuilt is named at the end, and the callback endpoint is still one of them.`

**Replace with:** `unbuilt is named at the end. The callback endpoint left that list on 2026-09-04 (Step 8).`

### `docs/flows/reconciler.md`, "What is not built" (lines 146-151)

**Retire the whole bullet:**
> - **No callback endpoint.** `POST /provider/{code}/callback` does not exist, so
>   nothing enqueues a poll from a callback and nothing compares Orange's
>   `notif_token` against the stored one. `parse_callback` is implemented on both
>   rails and is exercised by tests and by nothing else. The section above
>   describes a design, not a route.

**Move a reduced form into "What is built":**
> - **The callback endpoint exists.** `POST /provider/{code}/callback`
>   (`vpay_api::provider_callback`) is the route the section above describes,
>   built 2026-09-04. It never changes state: it enqueues the charge's
>   `poll:<charge id>` job if it is missing and brings it forward to `now()`
>   if it is parked at a rung, refusing a leased or dead-lettered one. The
>   `dedupe_key` really is what stops duplicate callbacks becoming a job
>   storm, and it is now that on a live path rather than in a design.

**And leave one honest gap behind, in "What is not built":**
> - **Nothing compares Orange's `notif_token` against the stored one.** The
>   route discards `CallbackRef::ref_extra` rather than trusting it, so a
>   callback still cannot repair a charge whose key material was lost. See
>   [adapter-orange-money.md](../../flows/adapter-orange-money.md).

### `docs/flows/crash-safety.md`, "What is still not built" (line 215)

**Retire:**
> - **No callback route**, so a rail that tries to tell us about a charge is
>   ignored and only the ladder finds out — see [reconciler.md](../../flows/reconciler.md).

**Replace with:**
> - **A rail that tells us about a charge is now heard** (Step 8): the
>   callback route pulls that charge's poll forward instead of leaving it to
>   the ladder's next rung. It changes nothing about recovery — the
>   authenticated status query is still the only thing that settles anything,
>   and every kill point above resolves identically whether a callback arrives
>   or not. See [reconciler.md](../../flows/reconciler.md).

### `docs/flows/provider-port.md`, "What the suite does not prove" (line 127)

**Retire:** `No callback route exists, so`
`parse_callback`'s output is verified by tests and by nothing in production.`

**Replace with:**
> Since Step 8 `parse_callback`'s output *is* consumed in production, by
> `vpay_api::provider_callback` — but only to name a charge and pull its poll
> job forward, and only from a body no rail has ever actually sent to this
> deployment.

### `docs/flows/browser-checkout.md` (lines 188-196) — **a claim that is now factually wrong**

The paragraph says Orange redirects the payer to
`{public_base_url}/provider/orange_money/callback`, "a path the router does
not mount". The router mounts it now, for `POST` only, so a payer's `GET`
gets **axum's bare `405`** instead of the 404 envelope. That is measured, not
assumed (`a_get_on_the_callback_path_is_a_405_and_not_the_404_envelope` in
`vpay-api`).

**The redirect gap itself is unchanged and still out of scope** (the Step 8
plan names it explicitly). Only the failure's *name* changed. Suggested
amendment to that paragraph:

> …so Orange redirects the payer to
> `{public_base_url}/provider/orange_money/callback` — which since Step 8 *is*
> a mounted path, but a `POST`-only one for the rail's own backend, so a
> payer's browser arriving there gets an empty `405`. That is not a return
> trip either. **Do not ship a redirect-rail (Orange) checkout on
> `@vpay/stripe-js` until a real return endpoint exists**; the callback route
> did not close this gap and was not meant to.

### `docs/roadmap.md` (lines 744-745 and 839-840)

- Line 744: `- No callback route exists: nothing verifies Orange's` `notif_token`, and
  `MTN's callbacks are unsigned.` → *"The callback route exists (Step 8) but
  nothing verifies Orange's `notif_token`, and MTN's callbacks are unsigned —
  so a callback is a hint on both rails and always will be."*
- Line 839: drop `the callback route (POST /provider/{code}/callback)` from
  the "not in this phase's original scope and still unbuilt" list, keeping
  `prompt_ttl_seconds` / `prompt_expired_at` / `payment_intent.processing`.

## 6. Verification — commands, counts and revert proofs

Run in `/home/selast/dev/vpay/.claude/worktrees/step8-lane-c-callback` with
`CARGO_TARGET_DIR` pointed at that worktree's own `target`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `CARGO_BUILD_JOBS=6`.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy -p vpay-api -p vpay-db -p vpay-worker -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-api -p vpay-db -p vpay-tests-conformance` | see §6a |
| `cargo nextest run -p vpay-tests-integration -E 'binary(confirm_rails) \| binary(worker_e2e) \| binary(provider_callback)' --no-fail-fast --retries 2` | see §6a |
| `just verify` | see §6a |
| `just verify-ignored` | `0 ignored (expected 0), 40 test binaries (expected 40), 1016 total (minimum 980)` |
| `just test-doc` | see §6a |

### 6a. Counts, from the final run on `4326809`

- `cargo fmt --all --check` — clean.
- `cargo clippy -p vpay-api -p vpay-db -p vpay-worker -p vpay-adapter-mtn-momo
  -p vpay-adapter-orange-money --all-targets -- -D warnings` — clean.
- `cargo nextest run -p vpay-api -p vpay-db -p vpay-tests-conformance
  --no-fail-fast --retries 2` — **323 tests run: 323 passed, 0 skipped, 0
  ignored** (`vpay-api` 214, `vpay-db` 81, `vpay-tests-conformance` **28**).
  Two **flaky, and neither is this lane's**: `vpay-db::postgres`'s
  `an_abandoned_transaction_survives_a_rollback_it_cannot_send` (passed on try
  3) and `run_migrations_applies_cleanly_and_is_idempotent` (try 2), both
  failing with testcontainers' `failed to create a container: Timeout error`.
  Cause: another Step 8 lane was running its own `cargo nextest` process
  against the same rootless Docker daemon at the same time, and
  `.config/nextest.toml`'s `postgres-containers` group (`max-threads = 1`)
  serialises container starts **within one nextest invocation only**, not
  across concurrent ones. Nothing about it is specific to this branch.
- `cargo nextest run -p vpay-tests-integration
  -E 'binary(confirm_rails) | binary(worker_e2e) | binary(provider_callback)'
  --no-fail-fast --retries 2` — **19 tests run: 19 passed, 0 skipped, 0
  ignored** (`confirm_rails` 7, `provider_callback` **9**, `worker_e2e` 3).
- `just verify` — `verify-no-mocks` ok; `verify-status` ok (1 unimplemented
  item, `mtn_momo::refund`, unchanged); `verify-errors` ok (14 error types,
  all classified — this lane added none). `verify-docs` is advisory and
  `provider_callback::callback` is **not** on its long-function list:
  the four decisions that were inline comments were moved into the function's
  own doc comment and into `docs/reference/vpay-api.md`, which is what this
  repository asks for.
- `just verify-ignored` — `0 ignored (expected 0), 40 test binaries (expected
  40), 1016 total (minimum 980)`.
- `just test-doc` — **77 doctests passed, 0 failed, 1 ignored** (the ignored
  one is `vpay_sdk`'s, pre-existing). This lane added none; every new item's
  reasoning is prose, and the two new `vpay-db` methods have no example
  because neither can be demonstrated without a live transaction.
- `just verify-sdk-parity` is not on this branch, as the brief said it would
  not be.

### 6b. Revert proofs (each mutation applied, measured, and restored)

1. **The pull-forward is what makes a callback worth anything.**
   Replaced `tx.pull_forward_in_tx(dedupe_key).await?` in
   `provider_callback::callback` with `false`.
   `a_callback_settles_the_charge_before_the_ladders_next_rung_would_have_fired::case_2_orange_money`
   fails: *"the callback must make the poll claimable now; run_at is still
   2026-09-04 0:21:33"*. Restored.

2. **An unparseable body cannot enqueue anything** (the plan's named guard
   proof). Made Orange's `notif_token` requirement optional in
   `vpay_adapter_orange_money::Adapter::parse_callback` — so the body
   `{"order_id":"…0ce0","status":"SUCCESS"}` becomes parseable.
   `an_unparseable_callback_body_is_refused_and_moves_no_job::case_2_orange_money`
   fails on the status: *"left: 202, right: 400"*. With that assertion
   temporarily removed as well, it fails on the *queue* — the assertion the
   guard is really about: *"a refused body must not enqueue a job and must not
   move one; left: [("poll:ch_…", 0:15:31)] right: [("poll:ch_…", 0:15:41)]"*,
   i.e. the poll had been dragged back from the +10 s rung to now. Both
   mutations restored.

3. **The mapping contracts are decisive on both rails.**
   - Removed `.header(CALLBACK_URL_HEADER, callback.clone())` from
     `vpay_adapter_mtn_momo::Adapter::submit`:
     `submit_returns_a_reference_and_a_flow_shaped_result::case_1_mtn_momo`
     **and** `the_submit_tells_the_rail_where_to_call_back::case_1_mtn_momo`
     both fail with `Config("mtn_momo: requesttopay answered HTTP 404 Not
     Found; check base_url")` — WireMock matched no mapping.
   - Pointed Orange's `notif_url` at `config.base_url` instead of
     `config.callback_url`: the same two Orange cases fail with
     `Config("orange_money: no webpayment endpoint under the configured
     base_url (HTTP 404): Request was not matched …")`.
   - Both restored; `git status` clean afterwards, verified.

## 7. What this lane did **not** do

- **No real rail was called.** The bodies POSTed are transcribed from the flow
  docs; a document that is wrong about MTN or Orange would pass every case
  here. The "do not deploy" banner is untouched.
- **Orange's `notif_token` is not verified** and `ref_extra` repair from a
  callback is not built — §4b is the status row for it.
- **The Orange redirect return trip is not built.** Out of scope by the Step 8
  plan. The only change is that the payer's `GET` now lands on an empty `405`
  rather than a 404 envelope; §5 has the doc amendment.
- **`GET` on the callback path is axum's bare `405`, not the Stripe
  envelope.** Left as-is, following the precedent
  `docs/reference/vpay-api.md` already records for `GET /v1/oauth/token`: the
  right fix is a `method_not_allowed` renderer for the whole router, not one
  route at a time.
- **No rate limiting, and no signature verification.** Neither rail offers
  anything to verify. What bounds the route is the `dedupe_key`'s unique
  index (one job per charge, forever), the three guards on the pull-forward,
  and the 16 KiB body limit — not a counter. Same reasoning, and the same
  honest limitation, as `vpay_api::browser`'s D5.
- **`docs/status.md`, `docs/roadmap.md` and `docs/flows/*.md` are untouched**,
  per the Step 8 plan's lane split. §4 and §5 are lane E's input.
