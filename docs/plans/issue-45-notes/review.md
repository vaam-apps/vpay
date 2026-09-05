# Issue #45 — sabotage review of `claude/issue-45-refund-retrieve`

Reviewer pass over `git diff 65a5952..14f66d7`, the implementer's own account
being [impl.md](impl.md). Written 2026-09-05. Every number below was measured
on this machine, not read off the implementer's report.

## Verdict

**Safe as delivered: yes, for what it does.** The route is merchant-scoped in
SQL, the tenancy refusal is byte-identical to a missing id, the renderer is
shared with the event payload, no creation and no events were added, and the
refund object's field list is untouched. Seven findings, one of them a real
hole in the proof and six of them documents the change falsified and left
standing. All seven are fixed on this branch; one carries a decision reserved
for the maintainer.

**The PR can honestly say `Closes #45`** — see the criterion map.

## Findings

| # | Severity | Finding | Fixed by |
|---|---|---|---|
| F1 | rule-break | The `re_` short-circuit — an explicit brief item — was proven by **no test**. Deleting it left all four integration cases and all seven `vpay-api` refund unit tests green. | `test(api): pin that the re_ short-circuit is reached, not decoration` |
| F2 | misleading-claim | `vpay-api`'s crate header still said `/v1/refunds` "routed nowhere". Issue #45 quotes that exact sentence as its evidence the route did not exist. | `docs(api): the crate header stopped being true…` |
| F3 | misleading-claim | `docs/sdks/parity.md`'s "what this matrix does not claim" said `/v1/refunds` "is not mounted at all", ~150 lines below the `refunds.retrieve` row the same diff added. | `docs(sdks): parity.md contradicted its own new row` |
| F4 | misleading-claim | `docs/flows/merchant-auth.md`'s live "Not done" list still read "**No `/v1/refunds`, `/v1/events` or `/v1/balance`** … nothing reads or writes either table". The diff corrected the *other* stale sentence in the same section. | `docs(flows): merchant-auth's "Not done" list…` |
| F5 | misleading-claim | `README.md` — the front page — said all three of `/v1/refunds`, `/v1/events`, `/v1/balance` "still answer the honest 404". | `docs(readme): the front page still said…` |
| F6 | misleading-claim | `sdks/nodejs/README.md` said the route 404s **and** that "this package has no client method for any of that". The diff adds `refunds.retrieve` to that package. | `docs(sdk-node): the package README denied the method…` |
| F7 | misleading-claim | Migration `0017`'s header **and its live `COMMENT ON TABLE refunds`** say the table is read by no code. Not editable (sqlx checksums the file); correction recorded in `docs/status.md`, per the `0006`/`0013` precedent. | `docs(status): record that migration 0017's comment is now false` |

F1 is the one that matters. By this repository's own test — "would a test fail
if it broke? If no, it is not done" (`AGENTS.md` rule 2) — the prefix check was
not done. `the_prefix_short_circuit_uses_the_minters_own_vocabulary` asserts
only that `REFUND_PREFIX == "re_"`; nothing asserted that `lookup` consults it,
and nothing had a row behind a mis-prefixed id, so the branch the check skips
returned `None` anyway and the two paths were indistinguishable.

## Mutation table

Each applied, measured, reverted. The first five re-run the implementer's own;
the rest are this review's.

| # | Mutation | Expected | Measured |
|---|---|---|---|
| MU1 | Drop `p.merchant_id = $1` from the join | tenancy case fails | **FAIL** `merchant_b_cannot_read_merchant_as_refund`, `left: 200, right: 404` — as reported |
| MU2 | Delete `/refunds/{id}` from `V1_ROUTES` | 3 of 4 integration + 1 unit fail, SDK green | **FAIL** exactly those; `the_refund_resource_is_mounted_for_a_read_and_for_nothing_else` fails; **136/136 `vpay-sdk` tests still pass** — as reported |
| MU3 | Render the response through a second hand-built map (`created` in ms) | byte-identity fails | **FAIL** on the bytes, `1788644015000` vs `1788644015` — as reported |
| MU4 | **Delete the `refunds.retrieve` parity row** | brief predicted FAIL | **PASSES** (346 → 342 proving tests, exit 0). The gate is one-directional. The implementer reported this honestly and the brief was wrong; see Reserved |
| MU5 | Rename a test the parity row names | `verify-sdk-parity` fails | **FAIL**, naming `docs/sdks/parity.md:106` and the cell — as reported |
| MU6 | **Delete the `re_` short-circuit** | brief item; something should fail | **NOTHING FAILED** — 7/7 `vpay-api` refund units and 4/4 integration cases green. This is F1 |
| MU6′ | Same deletion, against the fix | the new case fails | **FAIL** `a_refund_id_without_the_re_prefix_is_never_looked_up`, `left: 200, right: 404`, and it is the **only** case that fails |
| MU7 | Drop `.to_lowercase()` on `currency` | rendering + wire cases fail | **FAIL** `a_refund_renders_the_nine_documented_keys_and_no_others`, `the_merchant_sdk_deserialises_the_refund_this_renders`, `a_stored_refund_reads_back_through_the_sdk` |
| MU8 | `RESOURCE` `"refund"` → `"payment_intent"` | the envelope's wording is pinned | **FAIL** `the_resource_name_is_the_one_the_documented_envelope_uses`, `merchant_b_cannot_read_merchant_as_refund` |

Tenancy mutations that did **not** need a fix: a refund whose owning intent
belongs to another merchant is exactly what MU1 and
`merchant_b_cannot_read_merchant_as_refund` cover — the join is the only path
from a refund to a tenant, `refunds` carries no `merchant_id`, and there is no
unscoped variant of the read to reach by accident. The handler logs nothing, so
no id or metadata reaches a log line. Both SDK `retrieve` methods are
structurally incapable of sending an `Idempotency-Key`: the Rust one goes
through `get(…)`, which takes no `RequestOptions` at all, and both send an
empty body — asserted in `retrieve_refund_is_a_get_with_no_body_and_decodes_the_object`
and `refunds.retrieve: exact GET path, no body, no Idempotency-Key`.

## Documentation claims the diff makes, checked against the tree

| Claim | Verdict |
|---|---|
| "Twelve methods across ten paths" (`docs/api/README.md`) | **TRUE** — counted in `V1_ROUTES`: 2+1+1+1+1+1+2+1+1+1 |
| "eleven across nine until 2026-09-05" | **TRUE** — the same count minus this route |
| "`vpay_sdk` exposes **thirteen** resource methods, not eight" | **TRUE** — 5 payment_intents + 4 checkout.sessions + 2 refunds + `events.list` + `balance.retrieve` |
| "Two of the thirteen have no route" (`refunds().create()`, `balance().retrieve()`) | **TRUE** |
| "`GET /v1/events` had been `⛔ 404` in the Resources table and has been served since Step 5" | **TRUE** — `/events` and `/events/{id}` are both in `V1_ROUTES` |
| "the Checkout Session routes are not in that table at all" | **TRUE** |
| "`confirm` answers `501`" was stale since Step 3 | **TRUE** |
| "136 tests, 0 ignored" (`cargo nextest run -p vpay-sdk`) | **TRUE** — measured 136/136 |
| "174 tests" (Node SDK) | **TRUE** — `sdks/nodejs test: Tests 174 passed (174)` |
| "1293 total, 42 test binaries, 0 ignored"; "1279 plus those 4, plus 2, plus 5, plus 2 and 1" | **TRUE, and the arithmetic checks**: 4+2+5+2+1 = 14. Now 1294 with F1's case |
| "37 `AssertSqlSafe` sites"; "eleven `const … : &str`" | **TRUE** — the gate pins the first; the eleven are enumerable in the crate |
| "the nine keys `merchant-auth.md` documents" | **TRUE** — and the object's field list is untouched, as issue #46 requires |

Nothing the diff asserts turned out to be false. The failures were all
omissions — documents elsewhere that the change made untrue.

## Criterion map

The maintainer took the decision the issue asked for and took the **stronger**
of the two shapes the issue offered (serve it, rather than declare it `⛔ 404`),
so the criteria below are the brief's, which supersede the issue's proposal (1).

| Criterion | Verdict |
|---|---|
| `GET /v1/refunds/{id}` in `V1_ROUTES`, merchant-scoped | **delivered** |
| Foreign merchant and missing id → byte-identical `resource_missing` 404 | **delivered**, and it is the `resource_missing` envelope rather than `unknown_route` |
| `re_` prefix validation | **delivered but unproven as merged**; proven now (F1). Answering the same `404` rather than a `400` (impl D4) is a **justified deviation** — `v1::events::retrieve`'s own doc makes that argument in this repository's words, and a distinguishable answer would tell a caller one thing more than `/v1/events/{id}` does |
| One renderer, so API response and event payload cannot disagree | **delivered** — `RefundObject`, pinned byte-for-byte |
| Test rows seeded through the repository | **deviated (D2), and the deviation is right.** A `create` no shipping code calls is the "claiming a feature" failure `AGENTS.md` rule 2 names; raw SQL against the real schema has precedent in `support::age_the_crash`. Recorded, not reversed |
| Docs: Resources row ✅, "Not served" keeps `POST`, measured count, `vpay-api.md`, `status.md` | **delivered**, plus five stale claims corrected outside scope, all seven checked true — and six more that were left (F2–F7) |
| Both SDKs, byte-identical in shape to the payment-intent read | **delivered** — `RefundsResource::retrieve(&self, id: &str)` and `refunds.retrieve(id)`, both matching their `payment_intents` sibling exactly |
| Parity row naming both tests | **delivered** — four test names across the two columns, all existing |
| `stripe-compat` convention | **delivered** — no route table and no refunds case there, so `flows/stripe-sdk-compat.md` records the untested gap in the same words it uses for `stripe.events.list()` |
| No creation, no events, no field-list change | **delivered** — `creating_a_refund_is_still_the_honest_404` also asserts `count(*) FROM refunds = 0` |

**`Closes #45` is honest.** Issue proposal item 3 asked for "two of eight → three
of nine"; the implementer measured instead of incrementing and wrote "two of
thirteen", which is the same criterion answered better.

## Reserved for the maintainer

1. **Making `verify-sdk-parity` two-directional.** Confirmed one-directional
   (MU4): it checks that every test a ✅ cell *names* exists, and cannot know
   that a capability the SDKs *have* is unrecorded. Not cheap to reverse, and
   the expensive part is a decision, not code: the matrix's rows are prose
   capabilities ("RS256 assertion with the six claims the OP verifier reads"),
   not method names, so a code→doc direction needs a convention for how a row
   names an SDK method before anything can enforce it. That convention is a
   maintainer's call and this review did not take it.
2. **Whether to spend a migration number on `COMMENT ON TABLE refunds`.** F7.
   The comment is live in every deployed database and now false; the only way
   to correct it is a new migration, which this change deliberately avoided.

## Pre-existing staleness found and deliberately **not** fixed

Named so they are visible, not silently swept in — none is this branch's:

- `backends/crates/vpay-api/src/lib.rs`'s STATUS paragraph has never mentioned
  `/v1/checkout/sessions`, served since Step 9.
- `README.md`, one sentence after F5's: `vpay-worker-bin`'s "job loop is not
  implemented … a startup banner and a repeating heartbeat log line" — untrue
  since Step 4, per `docs/status.md`'s own Process-lifecycle row.
- `backends/tests/integration/tests/payment_intents.rs:1081` calls `/v1/events`
  "an unrouted `/v1` path" in a comment and "not implemented" in an assertion
  message. The assertion (401 for an anonymous caller) still holds for all
  three paths; only the wording is stale.
- `docs/roadmap.md`'s two mentions are inside dated per-phase addenda, which
  are historical records rather than live claims, and were left as such.

## The gate, recipe by recipe

`just ci` on the delivered tree (`14f66d7`) and again on the final tree. Both
green end to end; the second run's numbers are in `impl.md`'s successor section
below and in the reviewer's report.

## Flake, observed

`sdks/rust/tests/token_exchange.rs`'s
`a_second_concurrent_401_does_not_discard_the_token_the_first_one_just_fetched`
is recorded in `status.md` as flaky under load. It did **not** fail in either
of this review's two full `just ci` runs or in the two standalone
`cargo nextest run -p vpay-sdk` runs (136/136 each time). Neither confirmed nor
refuted here.
