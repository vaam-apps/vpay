# Roadmap — Phase 3: The payment API (`/v1`)

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

## Phase 3 — The payment API (`/v1`)

**Goal.** Merchants can create and confirm `PaymentIntent`s through `/v1`,
authenticated, idempotent.

**Status.** In progress — see the 2026-09-03 addendum at the end of this
phase. _This line said "Not started" until then._ The object model and state
machine (`vpay-core::state`) are implemented and tested, and four
`/v1/payment_intents` paths now route HTTP requests through them; `confirm`
reaches the rail adapter and stops at its `NotImplemented`.

_Addendum, 2026-09-03 (evening): two later deliverables sit **on top of** this
surface without changing its contract — the official Stripe SDK path (Step 5b,
PR #20, [flows/stripe-sdk-compat.md](../flows/stripe-sdk-compat.md)) and the
unauthenticated browser surface a payer's own page calls (Step 5c, PR #22,
`/v1/browser`, [flows/browser-checkout.md](../flows/browser-checkout.md)).
Migrations `0025` and `0026` came with them; the repository total is now 26,
not the 20 the Step 3 addendum below records._

**Scope.**

- `POST /v1/payment_intents` (create) — writes a row via the existing
  `vpay-core` types. **Has no rail dependency**; it can be built and tested
  before Phase 4 lands.
- Idempotency-key handling on create.
- ~~Request-auth middleware consuming Phase 2's merchant token validation.~~
  **Done ahead of this phase, 2026-09-02**: `AuthenticatedMerchant` is
  mounted in front of the whole `/v1` nest, so a route added here is
  authenticated by construction rather than by remembering to add a layer.
  What that nest currently holds is one honest 404.
- `POST /v1/payment_intents/{id}/confirm` — submits to the adapter. **This
  one genuinely depends on Phase 4**: [`docs/flows/payment-lifecycle.md`](../flows/payment-lifecycle.md)
  and [`docs/flows/crash-safety.md`](../flows/crash-safety.md) both describe
  `confirm` as calling the adapter's `submit()`; until that call is real
  (Phase 4), `confirm` can only reach the `NotImplemented` stub.

**Note on ordering.** Phases 3 and 4 interleave rather than strictly
sequence — `create` doesn't need a rail at all, `confirm` needs one to do
anything beyond call a stub. `docs/status.md`'s MVP checklist lists adapters
(its item 2) ahead of `/v1` (its item 4); read as a strict build order that
disagrees with placing Payment API before The rails here. Read as an
unordered checklist of exit criteria it doesn't — both lists agree on what
must be true, just not on a claimed sequence. This roadmap places `create`
first because it is buildable now, and treats `confirm`'s completion as
gated on Phase 4 regardless of which phase number it sits under.

**Definition of done.**

- An integration test drives create → confirm → a terminal state over real
  HTTP and asserts the object shape at each step.
- `one_charge_per_intent` is proven at the API level (a second confirm
  attempt does not produce a second charge), not just as a bare DB
  constraint test.
- A replayed idempotency key returns the same object without a second row.

**Unblocks.** Webhooks (Phase 6, needs a state change to notify about);
worker (Phase 5, needs charges to poll).

**Decisions this phase rests on.** [ADR-0002](../adr/0002-provider-port.md)
(core branches on capability values, never a provider code) governs how
`confirm` picks push vs. redirect handling.

**Risks carried by this phase.**

- `docs/flows/crash-safety.md`'s write-first-network-second discipline
  ("generate the reference, persist it, only then call the rail") is
  documented but unimplemented — this phase is where it has to land, and
  getting the ordering wrong is the exact failure mode the doc exists to
  prevent. _Landed for `confirm` on 2026-09-03; the recovery half did not —
  see the addendum._

### Status addendum — 2026-09-03 (Step 2, branch `claude/step2-payment-intents`)

**Done, with the test that would fail if it broke.** Everything below ran
against a real `postgres:16-alpine` on the authoring machine on 2026-09-03
(74 container-backed tests, 0 failures); ~~it has not run in CI.~~
**Retired 2026-09-05: it runs in CI.** CI run `33929374663` (2026-09-04,
`master`, head `33d6c25`) ran `cargo nextest run --workspace` on
`ubuntu-latest` to 1159 tests run, 1159 passed, **0 skipped** — 163 of them
`vpay-tests-integration` and 86 `vpay-db`, the container-backed crates.

- **`POST /v1/payment_intents`** — form-encoded, validated, merchant-scoped,
  idempotent, writing a real row
  (`create_then_retrieve_round_trips_through_the_sdk`).
- **`GET /v1/payment_intents/{id}`**, with another merchant's id answering a
  byte-identical 404 (`merchant_b_cannot_read_merchant_as_intent`).
- **`GET /v1/payment_intents`** — keyset pagination over a new `seq` column
  (`list_pages_forward_and_backward_with_cursors`,
  `a_list_refuses_two_cursors_and_a_malformed_one`).
- **`POST …/cancel`** — a compare-and-swap that also refuses while a charge
  is live (`cancel_is_legal_only_from_requires_payment_method`,
  `a_confirmed_intent_cannot_be_canceled`).
- **Idempotency on every `POST`**, required rather than optional: replay,
  mismatch, in-flight, release-on-`5xx`, reclaim-expired and sweep, each with
  a named test (see `docs/status.md`'s Idempotency row).
- **Request-auth middleware (D3)** validating once, resolving the tenant, and
  checking `payments:write` / `payments:read`
  (`a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not`).
- **Boot step 4** — `vpay_db::ConfigReconcile::reconcile`, one
  advisory-locked transaction in both binaries, with a YAML rail that has no
  linked adapter exiting `78`
  (`a_provider_code_with_no_linked_adapter_is_exit_78`).
- **Five migrations**, `0014`–`0018`. _This document's Phase 1 scope line
  says "All 12 Postgres migrations (`0001`–`0012`)"; that remains an accurate
  description of **Phase 1's** scope, and is not the repository total. The
  repository now has **18** (`0001`–`0018`): `0013` landed with the
  authkestra upgrade, `0014`–`0016` are Step 2's working schema, and `0017`
  (`refunds`) and `0018` (`events`) are **schema only — no code reads or
  writes either table**._

**The three "definition of done" items above are all still unmet, and none of
them can be met before Step 3 / Phase 4:**

- _"create → confirm → a terminal state over real HTTP"_ — **unmet.**
  `confirm` reaches `adapter.submit(..)` and receives
  `ProviderError::NotImplemented`, which is a real `501`. No intent has ever
  reached `processing`, `requires_action` or `succeeded`. The terminal state
  in this criterion requires a rail.
- _"`one_charge_per_intent` proven at the API level"_ — **met in the half
  that does not need a rail**, and stated exactly: a second confirm produces
  no second charge (`a_second_confirm_cannot_produce_a_second_charge`, with
  `a_second_charge_for_one_intent_is_refused_as_a_named_unique_violation`
  under it). What is not proven is the same property across a _successful_
  submission, because there has never been one.
- _"a replayed idempotency key returns the same object without a second
  row"_ — **met**
  (`a_replayed_idempotency_key_returns_the_same_object_and_no_second_row`).

**Also not done in this phase, and not hidden by the above:** `next_action`
is never populated and a redirect `return_url` is validated and then dropped
(no column); there is no recovery pass reading the `submitting` charges and
status-less `provider_requests` rows that `confirm` deliberately leaves
behind; `/v1/refunds`, `/v1/events` and `/v1/balance` are unrouted; the
worker sweeps nothing; and the Node SDK has still never spoken to a running
vpay.

### Addendum to the addendum — 2026-09-03 (Step 3, branch `claude/step3-rails`)

**Three of the items just above are closed by Phase 4a, and one criterion
moves from "unmet" to "half-met".**

- **`confirm` now moves the intent.** `processing` on a push rail,
  `requires_action` with a `next_action.redirect_to_url` on a redirect rail,
  `409 charge_declined` on a decline, `502` when the rail is unreachable —
  seven integration tests in `backends/tests/integration/tests/confirm_rails.rs`
  against real Postgres **and** WireMock containers.
- **`next_action` is populated**, and only ever from the committed charge
  row (`redirect_confirm_commits_the_rails_material_before_it_answers`).
- **`return_url` has a column** (`charges.return_url`, migration `0019`, with
  length and scheme CHECKs) and is committed before the rail is called.
- _"create → confirm → a terminal state over real HTTP"_ — **still unmet, and
  the reason changed.** The HTTP is real but the rail is a stub, and no
  terminal state is reached by anything: `succeeded` requires a poll, and
  nothing polls. That is Phase 4b/Phase 5.
- _"`one_charge_per_intent` proven at the API level"_ — the half that needed
  a successful submission is now proven too: a confirm that succeeds still
  cannot produce a second charge, and a retry after a _lost_ submit is
  refused with "poll, do not create a new PaymentIntent".

**Migration count, corrected:** the repository now has **20**
(`0001`–`0020`). `0019` adds `charges.return_url`; `0020` adds only a column
comment documenting the `provider_requests.status_code = 0` sentinel
(_answered, but the port carries no HTTP status_), changing no data and no
constraint.

---
