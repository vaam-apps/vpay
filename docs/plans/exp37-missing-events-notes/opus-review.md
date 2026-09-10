# exp37 — the sabotage review of "the transitions that emitted no event" (#57, #66)

Reviewer's transcript for `claude/exp37-missing-events`, rebased onto
`origin/master` at `ff1f507` (which carries #94, the dashboard port variable,
and #95, the invoice SDK surface). Everything below is a measurement taken on
this branch on 2026-09-10 under the pinned 1.98.0 toolchain, the `.nvmrc` Node
(22.23.2), `postgres:16-alpine` and `wiremock/wiremock:3.9.2`, or a citation
into this tree.

The brief for this review is the one the implementer worked from plus the
attack list in the review brief: the money invariants, the races, the deletion
of the pooled twins, migration `0039` against rows, the SDK unions after the
rebase, and the documents.

---

## Verdict

**Not safe as delivered — but the unsafety is in what the branch _said_, not
in what it _wrote_.** Every money-path claim the implementer made was true and
every mutation they recorded reproduces. Four things were wrong or missing:

1. the demo's own user-facing copy still told a buyer that vpay emits no
   `payment_intent.canceled` (**misleading-claim**, and the one a person
   actually reads);
2. `just test-e2e` had never been run and **no browser case covered either new
   outcome**, while `examples/shop/README.md` had just been rewritten to claim
   both (**gate-hole**);
3. a cancel racing a settlement was asserted **nowhere**, and the guard that
   decides it is not the one the branch documented (**correctness**, latent);
4. migration `0039` had never met a row — every suite here migrates an empty
   database, so `ADD CONSTRAINT`'s validation scan was untested
   (**gate-hole**).

Plus two smaller ones: the implementer's own notes named the wrong function in
the paragraph explaining why the customer update cannot use a timestamp
equality, and `docs/status.md`'s migration count was left stale by a
deliberate decision the review reverses.

**And one of this review's own tests was vacuous in one direction when first
written**, caught by its own mutation (R3) and rewritten. That is recorded
here rather than quietly fixed, because it is the same class of defect this
review is here to find.

All of it is fixed on `6bed3d0`, where `just ci` is nine recipes at exit 0
(1636 Rust tests, 0 skipped; 109 doctests; 1262 web) and `just test-e2e` is
exit 0 with 18 Cypress tests.

All six are fixed on this head. Nothing was weakened; the two assertions this
review touched were both tightened.

---

## The rebase

`git rebase origin/master` produced two conflicts, both content and both
resolved by keeping **both** sides:

- `docs/status.md` — the Customers row (this branch's `customer.*` paragraph)
  and the Invoices rows (master's exp33 text) are adjacent lines in the same
  hunk. Branch's customers cell + master's invoices cell.
- `sdks/rust/tests/resources.rs` — this branch's
  `the_customer_created_and_updated_event_types_are_known_and_their_payloads_decode`
  and master's `// ---- invoices` section were appended at the same point.
  Both kept, in that order.

After the rebase `verify-sdk-parity` reports **450 proving tests, 33 dated
gaps, 32 SDK methods across 35 rows** — the exp33 numbers plus this branch's
closed row — and both SDK unions carry `customer.created`, `customer.updated`
and `payment_intent.canceled` with a decoding test each.

`check-schema` runs against the **pinned** cratestack 0.12.0 here (a private
`--root`, never the shared bin), so the `WARNING — cratestack 0.11.1 on PATH`
the implementer recorded is not present in this review's runs. Their green was
the weaker statement they said it was; this one is not.

---

## Findings

### F1 — the shop still told a buyer the cancel event does not exist (misleading-claim)

`examples/shop/README.md` and `src/lib/test-numbers.ts` were corrected when
the events landed. Three places were not, and they are the three a person
meets:

- `src/components/order-actions.tsx` rendered, **on screen**, "This order will
  nevertheless stay unpaid: vpay does not yet emit a
  `payment_intent.canceled` event";
- `src/server/orders.ts`'s `cancelOrder` doc said the `cancelled` status "is,
  in practice, unreachable";
- `src/app/orders/[id]/cancelled/page.tsx` told the payer, in bold, "It will
  not: vpay emits no event for that transition."

This is CLAUDE.md's failure mode inverted — the demo understating the stack it
exists to demonstrate — and it is the same class of defect, a claim the repo
makes about itself that is not true. Fixed as dated corrections, because each
measurement (the demo stack, 2026-09-06) was right when it was made.

**No gate covers this prose**, and that is stated rather than papered over. Its
subject is now covered by a browser case (F2).

### F2 — no browser case covered either new outcome (gate-hole)

`just test-e2e` was not run by the implementer, for a reason that no longer
holds: `compose.demo.yml`'s dashboard port became `demo_dashboard_port` on
master (#94), so a second stack comes up beside the user's. The four Cypress
specs, 11 cases, covered neither new transition — the shop's README had been
rewritten to claim `237600000400` now reaches `failed` and `cancelled` is
reachable, and the only evidence for either was a Rust suite asserting the
`events` row and a vitest cross-check of the README against a TypeScript
constant.

Two cases added to `shop-embedded.cy.ts` (the framed run, which is where the
order id is known before the payment):

- **the decline** — buy, choose MTN, type `237600000400`, watch vpay's page
  refuse it on the field, then watch the shop's own `orders.get` reach
  `failed`;
- **the cancel** — buy, go to the order page, press "Cancel this payment",
  watch `orders.get` reach `cancelled`.

Neither touches a line of shop code, which is the point: the README's claim is
that nothing in the shop had to change.

### F3 — a cancel racing a settlement was asserted nowhere (correctness, latent)

The branch documents the cancel's `NOT EXISTS` live-charge guard as what makes
a cancel safe. It is not sufficient, and nothing said so. A confirm may commit
its charge **while** a cancel's transaction is open — a legal interleaving of
two requests, and one the `NOT EXISTS` cannot see, because its statement
already took its snapshot. What actually stops a payment settling onto a
withdrawn intent is the _settlement's_ guard,
`succeed_after_submission`'s `WHERE … status IN (SETTLEABLE_STATUSES)`.

`a_cancel_racing_a_settlement_leaves_one_terminal_state_and_one_event` forces
that interleaving with a barrier and pins four things: one status, one event,
the settlement's `WriteMatchedNoRow`, and the charge left untouched by the
rolled-back settlement. The mirror direction — a `succeeded` intent refusing
the cancel and writing no event — is asserted too.

**The residual risk is stated rather than closed**, in
`docs/flows/payment-lifecycle.md`: that outcome is _loud_, not _safe_. The rail
accepted a payment for an intent that was withdrawn, and vpay has no repair
path for it. This review did not invent one; that is a maintainer's call.

### F4 — migration 0039 had never met a row (gate-hole)

Every suite in this repository migrates an **empty** database, so
`ADD CONSTRAINT … CHECK`'s row-validation scan — the half that decides whether
a deployment boots — was exercised by nothing.
`migration_0039_validates_a_populated_events_table_in_both_directions`
re-applies the file's own text (`include_str!`) over one row of each of the
fifteen types, and then over a row it does not name, requiring the second to
fail naming `type_is_a_documented_event`.

### F5 — the notes named the wrong function (misleading-claim)

`docs/plans/exp37-missing-events-notes/opus.md` said `record_payment_error`
binds `updated_at` from the calling process's clock. It binds `now()`, which
is why the decline's assertion is an equality and why the file's own M4b could
fire it — and the same file's "What was NOT done" section says so two screens
later. Corrected in place.

### F6 — `docs/status.md`'s migration count (misleading-claim)

The row said "thirty-one migrations in total (`0001`–`0031`)" and had been
wrong since `0032`. The implementer surfaced it and declined to fix it, on the
ground that rewriting the count without rewriting the two-thousand-word chain
around it would make the cell disagree with itself. The count is what a reader
takes from a status page, so it is corrected — with the chain explicitly left
at `0031` and _said_ to be, and with both machine-checked authorities named so
the next reader consults a gate.

### Two assertions that stopped one step short (nit, both tightened)

- The concurrent-metadata case pinned only the **last** event's body. The
  first update's event is exactly what an ordering assertion lets through: it
  must carry one key, not the merge the second transaction went on to make.
- Migration `0039`'s header says `touch_last_used` emits nothing. Nothing
  asserted it. That matters now that `customer.updated` exists and the stamp
  is a write to the same table on the confirm path — a later author reaching
  for "every customer write emits" would turn one webhook per payment into
  the merchant's problem.

---

## What the review attacked and found sound

| Attack                                                                                                        | Result                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Money invariants of a submit decline: status, `last_payment_error`, one charge, one event, no `client_secret` | Sound. Now also pinned end to end, including the delivery                                                                                                                                                                                                                                                                               |
| A cancel refused by the status guard, and by the live-charge guard                                            | Sound, both already asserted, both emit nothing                                                                                                                                                                                                                                                                                         |
| A cancel of a `processing` intent with a live charge                                                          | Refused `409`, no event — covered                                                                                                                                                                                                                                                                                                       |
| `customer.updated` under two concurrent PATCHes                                                               | Sound. Two events, in commit order, each carrying the state it committed. They do **not** coalesce, and that is now pinned at both ends                                                                                                                                                                                                 |
| `touch_last_used`                                                                                             | Emits nothing. Was prose; now a test                                                                                                                                                                                                                                                                                                    |
| The pooled twins are gone                                                                                     | Confirmed by grep across `backends`, `.xtask`, `sdks`: `PaymentIntents::cancel`, `Customers::create`, `Customers::update` have **no** callers anywhere, including the sweep, the worker, the invoices `pay` path and the tests. The only pooled `customers` writes left are `delete` and `touch_last_used`. `verify-repositories` green |
| Drift constants after `0039`                                                                                  | Unmoved, as predicted: `0039` adds no table and no column. `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` is an exact `assert_eq!` and it passes                                                                                                                                                                   |
| Both SDK unions after the rebase over #95                                                                     | Both carry all three types; a decoding test per SDK; the parity row is ✅/✅ and names them; `verify-sdk-parity` green in both directions                                                                                                                                                                                               |
| `docs/flows/webhooks.md`'s per-type writer table                                                              | Complete — fifteen rows for fifteen types, each naming its writer or "nothing"                                                                                                                                                                                                                                                          |

---

## Mutations

Every one applied to this tree, measured, reverted. Rows R1–R4 are this
review's; the implementer's M1–M10 are in `opus.md` and were not re-run except
where noted.

| #   | Mutation                                                                                            | Expected to fail     | Measured                                                                                                                                                                                                                                                                                                                                                                         |
| --- | --------------------------------------------------------------------------------------------------- | -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| R1  | delete the `insert_in_tx` from `persist_decline`                                                    | the decline cases    | **FAIL** — `a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` with `left: []`, `right: ["payment_intent.payment_failed"]`, **and** the new delivery case with `a decline at submit must emit exactly one event, got []`. Run twice: once against both, once scoped to the new one alone, because nextest's fail-fast stopped the first run before it reached it |
| R2  | drop `SETTLEABLE_STATUSES` from `succeed_after_submission`'s `WHERE`, replacing it with a tautology | the race case        | **FAIL, but for the wrong reason.** The first form spliced the constant into a comparison and produced _malformed SQL_, so `apply_succeeded` errored instantly and the barrier assertion fired instead of the outcome assertion. Recorded rather than dropped                                                                                                                    |
| R2b | the same guard removed, leaving `WHERE id = $1` — valid SQL                                         | the race case        | **FAIL** — `a settlement onto a canceled intent must not succeed quietly`, and the dump shows exactly the defect: `charges.state: "succeeded"` and `payment_intents.status: "succeeded"` on the intent the cancel had already withdrawn. Two terminal states, two terminal events                                                                                                |
| R3  | `ADD CONSTRAINT … CHECK (…) NOT VALID` in migration `0039` — the row scan skipped                   | the 0039 case        | **PASS — not caught.** The finding is in the test, not in the migration: see below                                                                                                                                                                                                                                                                                               |
| R3b | the same mutation, against the rewritten case                                                       | the 0039 case        | **FAIL** — `0039 must refuse to apply over a stored row its list does not name … : PgQueryResult { rows_affected: 0 }`                                                                                                                                                                                                                                                           |
| R4  | remove `FOR UPDATE` from `customers::lock_for_update`                                               | the concurrency case | **FAIL** — `round 0: the \`left\` key was clobbered by a merge computed over a stale read: {"right":"0"}`. Reproduces the implementer's M5 on the rebased tree                                                                                                                                                                                                                   |

### R3 is the finding worth keeping, and it is about this review's own test

The first version of `migration_0039_validates_a_populated_events_table_in_both_directions`
planted its offending row by **dropping** `type_is_a_documented_event` and
leaving it dropped. Re-applying the migration then failed on the migration's
own _first_ statement — `ALTER TABLE events DROP CONSTRAINT
type_is_a_documented_event`, "constraint does not exist" — an error whose
message happens to name the constraint, which is what the assertion checked.
The second direction proved nothing, and `NOT VALID` (the exact defect the
case exists to catch) left it green.

Rewritten: a permissive `CHECK (true)` of the same name stands in while the
row is planted, so the migration has something to drop and the only thing left
that can fail is the scan; and a second assertion requires the message to say
"is violated by some row", which is the distinction the first version could
not make. Both directions were re-run unmutated (**PASS**, 48 s) before R3b.

### Two assertions this review added with no independent mutation

Stated rather than left implicit. The **first-update event** assertion in the
concurrency case and the **`touch_last_used` emits nothing** assertion are
tripwires: each fails against a future implementation that renders an event
body outside its transaction, or that emits for the retention stamp, and
neither has a one-line mutation on this tree that produces exactly that
implementation without rewriting the handler. They are belt-and-braces beside
R4 and the `now()`/ordering assertions, not evidence in their own right.

---

## Gates

Recipe by recipe, each run separately, exit codes read from
`exp37-review-gate.tsv` rather than from a banner, on the final head `6bed3d0` (echoed to
`exp37-review-ci-head.txt` before the run started).

| Recipe           | Exit | Wall       |
| ---------------- | ---- | ---------- |
| `fmt-check`      | 0    | 0 s        |
| `clippy`         | 0    | 2 s (warm) |
| `verify`         | 0    | 7 s        |
| `test-rust`      | 0    | 1154 s     |
| `test-doc`       | 0    | 6 s        |
| `verify-ignored` | 0    | 1 s        |
| `lint-web`       | 0    | 22 s       |
| `test-web`       | 0    | 13 s       |
| `deny`           | 0    | 0 s        |

- `Summary [1140.389s] 1636 tests run: 1636 passed, 0 skipped` — **1633 → 1636**,
  the three cases this review added. A pre-change baseline was run first on the
  rebased head: `1633 tests run: 1633 passed, 0 skipped`.
- `verify-ignored: 0 ignored (expected 0), 45 test binaries (expected 45),
1636 total (minimum 1080)` — no binary added or dropped.
- `test-doc`: **109 passed, 1 ignored**, a separate runner and a separate
  count.
- `test-web`: **1262** vitest cases across nine packages, 0 skipped
  (`frontends/apps/checkout` 507, `sdks/nodejs` 208, `frontends/apps/dashboard`
  150, `sdks/stripe-js` 146, `examples/shop` 102, `@vpay/ui` 74,
  `@vpay/config` 63, `@vpay/tokens` 8, `@vpay/api-client` 4).
- `verify`'s twelve gates, each with its own number: `verify-status` 1
  unimplemented item; `verify-errors` 18 error types, 16 `#[from]` variants;
  `verify-sdk-parity` **450 proving tests, 33 dated gaps, 32 SDK methods across
  35 rows**; `verify-links` 1030 links in 193 files; `verify-serde` 83 types,
  16 exempted; `verify-repositories` 4 concrete implementations named by none
  of 82 files outside `vpay-db`; `verify-migrations` **38 migration files**,
  all matching the manifest; `check-schema` **ok under cratestack 0.12.0**, the
  version this repository pins — the implementer's run had 0.11.1 on PATH and
  said so.

`just test-e2e` — **exit 0** on `ff7a8e8`, read from
`exp37-review-e2e-exit.txt`. **18 Cypress tests across four specs, 18 passing,
0 failing, 0 skipped**: `checkout.cy.ts` 1, `dashboard.cy.ts` 8,
`shop-hosted.cy.ts` 3 in pass 1; `shop-embedded.cy.ts` **6** (was 4) in pass 2.
Run in its own compose project (`demo_project=exp37-review`) entirely on
non-default ports — dashboard 13300, shop 13301, checkout 13380, api 18080,
orange 18082, receiver 18083 — beside the author's own `just demo` stack,
which held 3000/3001/3080/8080/8082/8083 throughout and was untouched. The
stack was torn down by the recipe's own `down -v` and no `exp37-review-*`
container or volume survives. **This is the first `test-e2e` run since #94 made
the dashboard port a variable**, which is exactly the blocker the exp37
implementer surfaced and declined to work around.

---

## Maintainer decisions, surfaced and not taken

- **A rail-accepted payment on a cancelled intent has no repair path.** F3's
  race ends in `DbError::WriteMatchedNoRow`, which pages. Whether vpay should
  attempt an automatic refund, park the charge, or keep paging is a money
  decision and is left open.
- **`DELETE /v1/customers/{id}` still emits nothing**; only the retention
  sweep writes `customer.deleted`. The implementer surfaced this and it is
  re-surfaced unchanged: widening later is cheaper than narrowing, and
  `POST /v1/checkout/sessions/{id}/expire` has the same shape and the same
  open question.
- **A bodiless `POST /v1/customers/{id}` emits nothing.** Stripe agrees.
  Unchanged.
- **`invoice.marked_uncollectible` and `invoice.payment_failed`** remain real
  Stripe types with no writer, recorded in `docs/flows/webhooks.md` and out of
  scope here.

---

## What this review did NOT do

- **`customer.created` and `customer.updated` have still not been fanned out
  to a receiver in any suite.** The review drove the third of the three new
  types to the receiver and stopped there: the two customer types are asserted
  at the `events` row, and `docs/flows/webhooks.md` says so in those words. The
  fan-out has now been observed carrying four types and branches on none, but
  that is still an argument.
- **No merchant endpoint outside this repository has received any of them**,
  which is the standing limit on every event type here.
- **The rail-accepted-payment-on-a-cancelled-intent case has no repair path**,
  and this review did not build one. It measured that the failure is loud and
  rolled back, and wrote down what an operator is left holding.
- **The `customer.updated` "one transaction" claim still rests on an ordering,
  not an equality**, for the reason the implementer's notes give (corrected:
  `customers::update_in_tx` binds the caller's clock; `record_payment_error`
  does not).
- **The prose corrected in F1 is covered by no gate.** Its subject is covered
  by two Cypress cases; the sentences themselves are not.
- **`just helm-check` and `cargo xtask verify-citations` were not run.**
  Neither is part of `just ci`, both need the network, and nothing here
  touches the chart. The issue and PR numbers cited in this branch's documents
  are therefore unverified by a gate on this machine.
- **Nothing was done about the remaining unwritten types** —
  `payment_intent.created`, `payment_intent.processing`, both `charge.refund.*`,
  `invoice.marked_uncollectible`, `invoice.payment_failed`. Out of scope, and
  still recorded in `docs/flows/webhooks.md`.
- **No flake measurement.** Every suite above was run once green on the final
  head (`test-rust` twice, counting the pre-change baseline). The two new
  barrier-based cases use a 750 ms wait in the passing direction, which is
  deterministic by construction — the transaction they wait on stays open — but
  neither has been run ten times.
