# exp2 — a confirm on an intent whose Checkout Session is no longer open

Branch `claude/exp2-expired-confirm-opus`, base `3b251f6` (content-identical
to `origin/master`'s `c531407`, which is the merge commit of the same tree).

## The defect, restated from the evidence

`docs/flows/hosted-checkout.md` ("The lifecycle is minimal and vpay's own",
D10) says four things move a session and only four, and that a payer's browser
is not one of them. Two of the four **tell the merchant the checkout is
over**: the hourly sweep flips `open` -> `expired` and, since 2026-09-04,
emits `checkout.session.expired` in the same transaction; and
`POST /v1/checkout/sessions/{id}/expire` is the merchant's own abandon.

Neither retracts the payer's credential. `docs/flows/browser-checkout.md`:
the intent's `client_secret` is "minted once at `create` ... This *is* the
credential — it authorises exactly one payment intent, **for its whole life**,
and there is no rotation endpoint". So a payer whose page loaded before the
checkout ended could still call
`POST /v1/browser/payment_intents/{id}/confirm`, and the confirm did not
consult the session at all — `v1/return_trip.rs` asked
`find_open_by_intent` only to decide a return URL, and answered `Ok(None)` for
an expired session exactly as it does for an intent that never had one.

The charge would then succeed, and `vpay_db`'s
`settle_for_intent` — guarded `WHERE payment_intent_id = $1 AND status =
'open'` — would correctly match nothing. The end state is a `succeeded`
intent over an `expired`/`unpaid` session, with a
`checkout.session.expired` webhook already delivered saying the opposite.

## What landed

| # | What | Where |
|---|---|---|
| 1 | `CheckoutSessions::find_latest_by_intent` — the newest session on an intent, **no `status` filter** | `backends/crates/vpay-db/src/checkout_sessions.rs:567` (trait) / `:844` (impl) |
| 2 | `ClosedSession` (`Expired`/`Complete`) and `ApiError::CheckoutSessionNotOpen { session_id, state }`, classified | `backends/crates/vpay-api/src/error.rs:257` / `:611` |
| 3 | `Classify` arms — `Category::Conflict`, the two codes, the two sentences | `backends/crates/vpay-api/src/error.rs:858` (`category`), `:914` (`code`), `:308` (`ClosedSession::message`) |
| 4 | `return_trip`: `CheckoutSessionGate` / `SessionGate` / `verdict` / `admit_confirm` — one read, two answers | `backends/crates/vpay-api/src/v1/return_trip.rs:55`, `:90`, `:203`, `:263` |
| 5 | Step 1b of the confirm, after `load_confirmable_intent` and before `open_attempt` | `backends/crates/vpay-api/src/v1/payment_intents.rs:786-805` |
| 6 | Seven integration cases | `backends/tests/integration/tests/checkout_sessions.rs:3130` onward |
| 7 | Four unit cases (three on `verdict`, one on the classification) | `backends/crates/vpay-api/src/v1/return_trip.rs:360`, `:405`, `:426`; `backends/crates/vpay-api/src/error.rs:1446` |
| 8 | Reference pages | `docs/reference/vpay-api.md` § the confirm path; `docs/reference/vpay-db.md` § `find_latest_by_intent` |

### The decisions, and the arguments

**Two codes, not one code with a `param`.** `checkout_session_expired` and
`checkout_session_complete`. `param` on this API names a *request parameter* —
`ApiError::param()` renders it only for `InvalidParam`, whose own doc says it
"must be a field name and never a value" — and the request that trips this
refusal carries no reference to a session at all, so a `param` would point a
merchant's form at a field they never sent. The distinction is carried by
`ClosedSession`, a two-variant enum rather than the row's `status` string, so
`Classify::code`'s match stays total and `open` is a state the error cannot be
constructed in.

**Not the category default `invalid_state`.** Same argument
`idempotency_key_in_flight` makes against `idempotency_key_in_use`: a merchant
must be able to tell "your payer walked away and we told you an hour ago" from
"this intent is already processing". Category, status (409), `type`
(`invalid_request_error`), retry (`Never`) and severity (`Info`) are all
derived from `Category::Conflict` — only the `code` and the sentence are the
variant's own, which is ADR-0011's shape.

**Both surfaces, because it lives in `confirm_once`.** That is what the two
share (`docs/reference/vpay-api.md` § the confirm path), so this is one rule
rather than two. It is worth having on `/v1` deliberately and not by accident:
that surface is not authenticated by the payer's `client_secret` at all, so a
merchant server that kept confirming after its own systems recorded the
checkout as abandoned would produce exactly the contradiction the browser
refusal prevents.

**After `load_confirmable_intent`, never before.** This answer is not the
uniform 404, so asking it first would let a caller learn that some other
tenant's intent has a checkout session on it. Before `open_attempt`, so the
refusal costs no charge row, no `provider_requests` row and no job.

**One read, not two.** The refusal and the return URL are answered from the
same row. Two reads would race the hourly sweep: a gate that read `open`,
followed by a return-URL lookup running a millisecond after `expire_due`
committed, would admit the confirm and then submit it with no return URL.

**The newest session decides.** An intent can carry several sessions over its
life (`create` refuses only an intent that already has an *open* one), so "any
session on this intent is not open" would refuse the ordinary "expire the
abandoned checkout, offer a fresh link" flow. `find_latest_by_intent` orders
by `seq DESC LIMIT 1`, and `checkout_sessions_one_open_per_intent` is what
makes "an open session is always the newest" a property of the schema.

**An `open` session past `expires_at` is expired, and the read writes
nothing.** Same rule and same reasoning as
`browser::checkout_sessions::authenticate`'s sixth refusal: a worker that is
down must not decide whether a payer can pay, and a confirm is the wrong place
to repair a row — flipping it there would emit no `checkout.session.expired`
(the sweep's transaction is what does that) and would skip the `NOT EXISTS`
live-charge guard that transaction carries.

**`Complete` is defence in depth and is unreachable today — stated plainly.**
`complete` is written by exactly one thing, `vpay_db`'s settlement
transaction, in the same commit as the intent reaching `succeeded` — and
`load_confirmable_intent` refuses a `succeeded` intent one step earlier with
`invalid_state`. So no sequence of shipping operations reaches the
`checkout_session_complete` code. The integration case for it **stages the row
with an `UPDATE`** and says so in its own doc comment. The arm is kept because
falling through to "expired" would tell a merchant their payer abandoned a
checkout that was in fact paid.

## Proof

`backends/tests/integration/tests/checkout_sessions.rs` — real Postgres, the
real WireMock MTN rail, the shipping router and the shipping
`vpay_worker::run_once`:

| Case | Function |
|---|---|
| 1 — sweep-expired -> `409 checkout_session_expired`, no charge/`provider_requests`/job, intent still `requires_payment_method` | `a_confirm_on_a_swept_session_is_refused_before_any_charge` |
| 2 — merchant `/expire` -> the same | `a_confirm_on_a_session_the_merchant_expired_is_refused` |
| 3 — `open` past `expires_at`, unswept -> the same, and `(status, payment_status, updated_at)` byte-equal before and after | `a_confirm_past_the_horizon_is_refused_by_the_read_and_writes_nothing` |
| 4 — `complete` -> `409 checkout_session_complete` (staged row; see above) | `a_confirm_on_a_complete_session_is_a_different_code` |
| 5 — no session -> `200`, and 1 charge + 1 `provider_requests` + 1 poll job | `an_intent_with_no_session_confirms_exactly_as_before` |
| 6 — merchant `/v1` confirm -> the same `409`, and the `Idempotency-Key` replay is the **stored** body, asserted byte-equal | `the_merchant_confirm_is_refused_too_and_the_replay_is_the_stored_409` |
| 7 — expire, create a second session, confirm -> `200` (the newest-row rule) | `a_second_session_after_an_expiry_makes_the_intent_payable_again` |

Unit (ADR-0011 classification and the verdict):
`error::tests::a_closed_checkout_session_is_its_own_409_and_not_a_lifecycle_conflict`
(category, 409, `invalid_request_error`, `Retry::Never`, both codes distinct
and distinct from `invalid_state`, no `param`, both sentences pinned
verbatim), plus the two new rows in `error::tests::cases()` (which
`every_variant_answers_with_the_classification_its_leaf_chose` and
`every_variant_renders_that_classification_over_a_real_router` walk) and one
new row in `the_retry_advisory_follows_the_classification_not_the_status`;
`v1::return_trip::tests::{only_an_open_session_inside_its_horizon_admits_a_confirm,
the_horizon_is_closed_at_the_instant_it_names, an_unknown_status_is_ours_and_pages}`.

### Guard-failure proofs (measured 2026-09-05)

| # | Sabotage | Result |
|---|---|---|
| 1 | Delete `verdict(&session, OffsetDateTime::now_utc())?;` from `SessionGate::admit_confirm` | `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions)' --retries 0 -j 1 --no-fail-fast` -> **26 passed, 5 failed**. Cases **1** and **6** fail as the brief asks (`a_confirm_on_a_swept_session_is_refused_before_any_charge`, `the_merchant_confirm_is_refused_too_and_the_replay_is_the_stored_409`), and so do 2, 3 and 4. Cases **5 and 7 still pass**, which is what makes the sabotage precise rather than a blanket break |
| 2 | Make `verdict` consult `status` alone — replace the `OPEN if now < session.expires_at` arm with a bare `OPEN => Ok(())` | Exactly **case 3** fails, 30/31 pass (`a_confirm_past_the_horizon_is_refused_by_the_read_and_writes_nothing`), and both `v1::return_trip` horizon units fail |
| 3 | Restore | `sha256sum` of `return_trip.rs` back to `72914bae…`; `git status` shows only the intended files |

### One existing case changed, and why

`a_payer_confirming_between_the_read_and_the_write_keeps_the_session`
(Claim 14g) **failed with `409` where it expected `200`** the first time this
change was run. It opened its window by rewriting the session's stored
`expires_at` into the past and *then* confirming — which the new rule refuses,
correctly. The window is now opened the other way round, with the instant the
sweep carries (`due_for_expiry(now + 25h)`, which `expire_due`'s own doc
offers: "a test can sweep a future instant instead of rewriting a stored
horizon"), so the payer confirms inside the real horizon and the sweep still
sees the row as due. Nothing is staged that a deployment does not do — it is
the ordinary shape of the race, a payer who confirmed shortly before the
horizon with a rail still holding the payment. The case still fails if
`expire_due`'s `NOT EXISTS` is deleted, which is what it is for.

## The status row

`docs/status.md` was **not edited** (the brief forbids it). Line 1379 today,
verbatim:

> | Checkout Sessions — `/v1/checkout/sessions` and `/v1/browser/checkout/*` (`vpay_api::v1::checkout_sessions`, `vpay_api::browser::checkout_sessions`) | 🟡 | **New 2026-09-04 (Step 9 lane 1).** A `checkout.session` (`cs_…`, migration `0028`) a merchant creates from its server against an existing `pi_…` — `create`/`retrieve`/`list`/`expire` on `/v1` (token-authenticated, `Idempotency-Key`, tenant-scoped), and three `GET`s on `/v1/browser/checkout` for the page. **Two payer credentials, not one** (D6): `client_secret` rides in the hosted `url`'s *fragment* and buys the intent's own `client_secret`; `return_token` rides in the return page's *query string* — it must, a fragment does not survive a rail's redirect — and buys the session and its intent **without** that credential. Every failure on both browser reads is the byte-identical uniform 404, including the tenancy case, and neither read renders the `url` (it carries the stronger credential in its fragment). `create` refuses an intent that is not `requires_payment_method`, one that already has a charge, and one that already has an open session — the last enforced by a **partial unique index**, not only by the pre-check. `expire` is a compare-and-swap with a `NOT EXISTS` live-charge guard in the same statement, so a session cannot be marked abandoned while a rail may still take the payment. The settlement transaction (`vpay_db::settlement`) flips `payment_status`/`status` in the **same commit** as the intent — `paid`/`complete` on success, `failed`/`expired` on a terminal decline. Proven by `backends/tests/integration/tests/checkout_sessions.rs` — **17 container-backed cases** against real Postgres, the real WireMock MTN rail and the shipping worker loop, with twelve recorded guard-failure proofs across lanes 1 and 1b (`docs/plans/step9-notes/lane-1.md` §5, `lane-1b.md` §5). Since **2026-09-04 (lane 1b)** the return trip is wired, sessions expire on their own, and both browser reads carry `merchant: { name }` — the member vpay's own checkout page reads when it is there. Two rules that were missing from those reads are now on the **read** itself and not on any sweep: past `expires_at` both answer the uniform 404 whatever the `status` (the `return_token` travels in a query string and therefore lands in a rail's logs, so the 24-hour horizon has to bound it), and the intent's `client_secret` is rendered only while `status = 'open'` (after settlement there is nothing left to confirm). A redirect confirm on an intent an open session drives no longer requires a `return_url` and ignores one that is sent — the page has none to send, and until this it answered `400`, so the hosted Orange flow could not complete at all. Still 🟡 and not ✅: **no real rail has taken a payment through a session** — every session ever driven end to end (lane 6's Cypress specs) settled against a WireMock host |

What is now true of it and is not said there: a confirm on an intent whose
Checkout Session is not `open` is refused with a `409`
(`checkout_session_expired` / `checkout_session_complete`) on **both** the
`/v1` and `/v1/browser` confirm, before any charge is opened, writing nothing;
an `open` session past `expires_at` that no sweep has reached is treated as
expired **by the read**, which does not write; an intent with no session, and
an intent whose expired session was replaced by a new open one, are
unaffected. `backends/tests/integration/tests/checkout_sessions.rs` is now
**31 container-backed cases**, not 17. `docs/flows/hosted-checkout.md`'s "Four
things move a session, and only four" is still true — this change moves no
session; it reads one. But its list of what the lifecycle is *for* now has a
fifth consequence worth a sentence there: a session that is not `open` refuses
the confirm, which is what makes `checkout.session.expired` a promise rather
than a notification. `docs/flows/browser-checkout.md`'s "the credential ...
authorises exactly one payment intent, for its whole life" now has an
exception, and `docs/flows/errors.md`'s policy table has two codes it does not
list. **None of those three files was edited**, per the brief.

## Measured (2026-09-05, this worktree, `DOCKER_HOST=unix:///run/user/1000/docker.sock`)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo nextest run -p vpay-db -p vpay-api -p vpay-worker -p vpay-core` | **465 passed, 0 skipped** |
| `cargo nextest run -p vpay-tests-integration -E 'binary(checkout_sessions) \| binary(browser_checkout) \| binary(confirm_rails) \| binary(payment_intents) \| binary(postgres_smoke)' --retries 2 -j 1` | **92 passed, 0 skipped**, no retries consumed |
| `just verify` | ok — `verify-errors: 15 error type(s), all classified`; `verify-status`, `verify-no-mocks`, `verify-sdk-parity` ok |
| `just verify-ignored` | **1158 total, 42 test binaries, 0 ignored** (was 1147/42/0) |
| `just test-doc` | **86 passed, 1 ignored** (unchanged; the ignored one is `sdks/rust`'s README block) |
| `cargo xtask verify-docs` | production functions >= 80 lines back to **9**, the base's number — `error::public_message` briefly reached 87 and the sentence was moved onto `ClosedSession::message` |

## Not done

- **Neither merchant SDK was touched, and neither needed to be.** Checked
  rather than assumed: `sdks/nodejs/src/errors.ts` declares
  `readonly code: string | undefined` and `sdks/stripe-js/src/errors.ts`
  declares `code?: string | undefined` — neither enumerates *server* codes.
  `CLIENT_ERROR_CODES` in `@vpay/stripe-js` is a closed set of codes the
  **client** originates, which this is not. So there is no closed union to
  extend and no `docs/sdks/parity.md` row to add, and ADR-0015's parity rule
  is not engaged. `just verify-sdk-parity` passes unchanged.
- **`docs/status.md`, `docs/flows/hosted-checkout.md`,
  `docs/flows/browser-checkout.md` and `docs/flows/errors.md` were unedited**
  when this note was written, per the brief. That was a real gap in the
  repository's own terms — AGENTS.md says a behaviour change updates
  `docs/status.md` and the flow doc in the same PR — and the sentences each
  needed are written above. **Closed 2026-09-05 by the landing pass**, which
  applied all four in the commit that lands this work; see the "Landed" note
  at the end of this file.
- **No migration, no schema change.** `postgres_smoke` was run anyway (its
  migration-count assertion is the gate master last failed on) and passes at
  29.
- **No real rail.** Every assertion here settled against a
  `wiremock/wiremock` container, like everything else in this repository.
- **Nothing proves this in a browser.** `frontends/apps/checkout` already
  paints an expired screen from the session read's 404, so the refusal is
  defence in depth behind a page that should never reach it; no Cypress spec
  drives a confirm on a dead session, and none was added.
- **The `checkout_session_complete` code is unreachable through the shipping
  API today** (see above). Its integration case stages the row.
- **No frontend, TypeScript, Helm or conformance suite was run** — nothing in
  this change touches them. `just ci` as a whole was therefore not run; the
  Rust half of it was, command by command, in the table above.

## Landed (2026-09-05)

Rebased onto `origin/master` at `c531407` (the merge of `3b251f6`, so the tree
this was written against is unchanged) and squashed into one commit. Three
things the landing pass changed rather than carried over:

1. **The four documents this note said were unedited are edited** —
   `docs/status.md` (the confirm row's third refusal, the Checkout Sessions
   row's rule and index and 17 → 31, the migration count 29 → 30, and the
   expiry-sweep row's "Not fixed here"), `docs/flows/hosted-checkout.md`,
   `docs/flows/browser-checkout.md` and `docs/flows/errors.md`.
2. **The reserved decision is named, not absorbed.** `docs/status.md`'s
   expiry-sweep row recorded the choice between "refuse the confirm" and
   "widen the settlement's guard" as reserved for the maintainer. This branch
   implements the first on the integrator's instruction, and the row now says
   so — chosen by the integrator on 2026-09-05, stated in the pull request for
   the maintainer's veto — rather than reading as though the limitation simply
   went away.
3. **Credential-bearing response bodies are out of the new tests' assertion
   messages.** `error_code`'s `with_context` and thirteen `assert_eq!`
   messages interpolated a whole body; a browser confirm,
   `GET /v1/payment_intents/{id}` and every `/v1` checkout-session response
   render a live `client_secret`, so the one moment an assertion failed it
   would print a credential into CI's logs (CodeQL `rust/cleartext-logging`).
   Every assertion is unchanged; only the messages are.

The measured gate for the landing pass is in the pull request body.
