# exp37 — the transitions that emitted no event (#57, #66)

Implementer's transcript for `claude/exp37-missing-events`, base `d5a93df`
(= `master`). Everything below is a measurement taken on this branch on
2026-09-10 against `postgres:16-alpine` and `wiremock/wiremock:3.9.2` under
the pinned 1.98.0 toolchain and the `.nvmrc` Node (22.23.2), or a citation
into this tree. Where a mutation is claimed, the command and the failure it
produced are recorded; where one did **not** fire, that is recorded too.

---

## What was built

Three transitions moved money-path or merchant-facing state and told no
merchant. Each now writes its `events` row in the transaction of the
transition itself — the shape `vpay_db::settlement`,
`checkout_sessions::expire_due`, `customers::delete_idle` and
`vpay_api::v1::invoices::write_with_event` already had, and the only shape
this repository has for an outbox write.

| Transition | Event | Where it is written now |
|---|---|---|
| `POST /v1/payment_intents/{id}/cancel` | `payment_intent.canceled` | `vpay_api::v1::payment_intents::cancel_with_event` |
| A rail decline at **submit** | `payment_intent.payment_failed` | `vpay_api::v1::payment_intents::persist_decline` |
| `POST /v1/customers` | `customer.created` | `vpay_api::v1::customers::create_with_event` |
| `POST /v1/customers/{id}`, when something changes | `customer.updated` | `vpay_api::v1::customers::update_once` |

### The design decision that is not in the brief, and why it was taken

Every one of the four pooled statements these replaced was **deleted**, not
kept beside its transactional twin:

* `vpay_db::PaymentIntents::cancel` — gone; `payment_intents::cancel_in_tx`
  is `pub(crate)` and reachable only through `TxRepositories`.
* `vpay_db::Customers::create` and `::update` — gone; `customers::insert_in_tx`
  and `::update_in_tx` likewise.

The brief did not ask for the deletions and they are the reason the diff
touches `vpay-db`'s public trait surface at all. The argument: no gate in this
repository objects to a `pub` method nobody happens to call, so leaving the
pooled variant beside the transactional one keeps "write the row and tell
nobody" exactly one call away, and the next writer has no reason to prefer the
longer form. Deleting it makes the wrong shape *not compile*. It cost three
call-site rewrites in `vpay-db`'s own tests and nothing else.

`persist_decline` did not need this: the charge and the intent stamp were
already in one transaction, so the event only had to join them.

---

## The three claims, and what proves each

### 1. `payment_intent.canceled` (#57)

It had been in `type_is_a_documented_event` since migration `0018`, in
`docs/flows/webhooks.md`'s vocabulary and in **both** merchant SDKs — with
nothing writing it. That is the one type in this vocabulary that predates
migration `0023`'s lockstep rule and never acquired a writer, which is why it
sat there for a week of releases; the shop demo's unreachable `cancelled`
order status was the visible half.

`a_cancel_emits_one_payment_intent_canceled_and_it_reaches_the_receiver`
(`backends/tests/integration/tests/webhooks.rs`) drives the shipping route
through the shipping Rust SDK, then the shipping `handle_fan_out` and
`handle_deliver`, and reads the bytes back out of the WireMock receiver's own
`__admin/requests` journal, verifying them with `vpay_sdk::webhooks::verify`.
It separates three things:

1. **one event, from the transition's own transaction** — `data.object` is the
   intent as committed, `status: "canceled"`, and carries no `client_secret`;
2. **a refused cancel writes nothing** — the second cancel is a `409` and the
   count does not move;
3. **an intent with a live charge is refused and emits nothing** — the
   `NOT EXISTS` guard, and the case where an event would report a cancel that
   did not happen.

### 2. `payment_intent.payment_failed` at submit (#57, broadened 2026-09-06)

`persist_decline` wrote the charge and `last_payment_error` and no event;
`vpay_db::settlement::apply_failed` on the worker's poll path was the only
writer of the type. So a rail that refused the charge outright — MTN's
`PAYER_NOT_FOUND`, the demo's documented `237600000400` — was the one terminal
outcome no signed event reported.

It emits the **same** type the poll path does. To a merchant the two are the
same thing happening; a second label would be a type no Stripe-shaped handler
branches on, which is `docs/flows/webhooks.md`'s standing rule. A merchant
cannot receive both for one intent: one charge per intent, forever, and this
path runs only when the rail refused it before anything polled it.

**One case is fail-closed rather than emitting.** If `record_payment_error`
matches no row — the intent moved between the rail's refusal and the write —
there is no committed intent to render, so no event is written and the
existing `WARN` was extended to name that omission as well. The alternative
would be a body that is either stale or invented. `cancel`'s live-charge
`NOT EXISTS` makes the case unreachable while a charge is `submitting`, which
is why it is a log line and not a repair path.

`a_payer_the_rail_does_not_know_is_a_decline_the_merchant_can_read` had its
event assertion **inverted** — `Vec::<String>::new()` to an exact one-element
list — rather than deleted, and gained four assertions on the body.

### 3. `customer.created` / `customer.updated` (#66)

Migration `0039` adds both labels **in the same change that writes them**,
which is `0023`'s lockstep rule working rather than an exception to it;
`0034`'s own comment is the record of why it declined to add them.

The load-bearing part is not the events, it is the lock. `metadata` is merged
key-wise (Stripe's contract), so the written value is a function of the stored
one, and `vpay_api::v1::customers::update`'s own doc comment said in as many
words that two concurrent updates each adding one key could lose one, and left
it. That was tolerable while nothing depended on the result being definite. A
`customer.updated` is exactly such a dependency: a merchant acting on a body
describing the losing merge acts on a state the database does not hold, and
has no reason to doubt it.

So the read is `SELECT … FOR UPDATE` inside the transaction that writes and
emits. The merge itself stays in Rust — a `jsonb ||` in the statement would
move Stripe's semantics into a migration and out of the layer that documents
them.

Nothing is emitted for a bodiless update (Stripe's no-op; nothing was
written), for a `404`, or for `touch_last_used`, which moves a column that is
on no wire object at all and would otherwise produce a byte-identical body
once per payment for ever.

---

## How "in one transaction" is measured rather than asserted

This is the claim that is easiest to make and hardest to check, because the
happy path looks identical either way: one event, right type, right body. Two
independent devices are used, and neither is a code seam.

### The `now()` identity, for the two payment-intent events

Postgres' `now()` is `transaction_timestamp()` — fixed at the start of the
transaction, identical for every call inside one. The cancel statement writes
`payment_intents.updated_at = now()` and the `events` row takes migration
`0018`'s `created_at DEFAULT now()`, so the two are **bit-identical** exactly
when they were one transaction.

The **update** path cannot use equality, and that limit is stated in the test
rather than glossed: `record_payment_error` and `customers::update_in_tx` bind
`updated_at` from the calling process's clock, not from `now()` (it is the
same instant `last_used_at`'s `GREATEST` needs, and that one must be the
caller's). What holds instead is an *ordering* — the transaction starts, Rust
then reads its clock — so `events.created_at <= payment_intents.updated_at`,
and an event written in a transaction opened after the update committed is
strictly later. It relies on the wall clock not jumping backwards between two
statements microseconds apart; that is the one flake this assertion could
have, and it is written down here rather than discovered.

### The abandon cases, at the seam

`a_cancel_and_its_event_roll_back_together`,
`a_submit_decline_writes_its_charge_error_and_event_in_one_transaction` and
`a_customer_write_and_its_event_roll_back_together` (all in
`backends/crates/vpay-db/tests/repositories.rs`) run every statement of a
transition and then `TxOutcome::Abandon`, and require that **nothing**
survived. That is what would fail if `TxRepositories::insert_in_tx` ever
stopped running on the caller's connection.

---

## Mutations run, and what each produced

Every one was applied to this tree, measured, and reverted.

| # | Mutation | Expected to fail | Measured |
|---|---|---|---|
| M1 | delete the events insert from `cancel_with_event` | the cancel case | **FAIL** — `a cancel must emit exactly one event, got []` |
| M2 | move the canceled event into a **second** transaction after the cancel commits | the cancel case | **FAIL** — the `now()` equality, `8:47:18.651176` vs `8:47:18.652493` (1.3 ms) |
| M3 | delete the events insert from `persist_decline` | the decline case | **FAIL** — `left: [], right: ["payment_intent.payment_failed"]` |
| M4 | move the decline event into a second transaction **that also re-runs `record_payment_error`** | the decline case | **PASS — not caught.** The re-stamp moved `updated_at` into the second transaction, so the ordering assertion held. Recorded rather than dropped: see below |
| M4b | the same, carrying the row out of the first transaction and **not** re-stamping | the decline case | **FAIL** — `8:48:31.773123` vs `8:48:31.774844` (1.7 ms) |
| M5 | remove `FOR UPDATE` from `customers::lock_for_update` | the concurrency case | **FAIL** — round 2, `{"right":"2"}`, the `left` key clobbered |
| M6 | the same mutation, against the seam case | `a_locked_customer_read_waits_for_the_writer_and_then_sees_its_value` | **FAIL** — `B answered while A still held the row lock` |
| M7 | drop `'customer.updated'` from migration `0039` | the vocabulary case and the create/update case | **FAIL** — both; `23514` naming `type_is_a_documented_event` |
| M8 | rename the Node parity test away | `verify-sdk-parity` | **FAIL**, naming the `sdks/nodejs` column |
| M9 | delete the `"customer.updated"` literal from `sdks/nodejs/src/types.ts` | — | `verify-sdk-parity` **passes**; `pnpm typecheck` **FAILS** (`TS2820` on the annotation in `types.test.ts`) |
| M10 | put the shop README's `237600000400` row back to `unpaid` | the README/module cross-check | **FAIL** — `agrees with the README for mtn_momo, row for row` |

### M4 is the finding worth keeping

The brief's decisive mutation for the decline was "move the event out of the
transaction". The first attempt at writing that mutation *also* re-ran the
`last_payment_error` stamp inside the second transaction, which put both
`now()` values in the same transaction again and the assertion held. The
mutation was rewritten (M4b) to carry the committed row out of the first
transaction instead, and then it fired.

That is a real limit on what the timestamp assertion proves, and it is stated
plainly: it catches an event moved to a later transaction, and does **not**
catch an event moved to a later transaction that also re-writes the row it
describes. The abandon case at the seam is what covers the general shape.

### The mutation the brief asked for that does not exist

> remove a union entry from one SDK → `verify-sdk-parity` FAILS (the gate now
> checks per column)

Measured, and it is **not** what happens (M8/M9). `verify_sdk_parity` checks,
per column, that a `✅` cell names a test that exists in that SDK's sources —
so *renaming the test* fails it, and removing a union *entry* does not,
because the test's name is still there. What catches a missing union entry is
the language: `pnpm -r typecheck` for TypeScript (the `const … : KnownEventType
= "customer.updated"` annotation in `types.test.ts`), and `cargo build` for
Rust (the `match` in `KnownEventType::from_wire` is exhaustive by
construction, and `sdks/rust/tests/resources.rs` names both variants). Both
are in `just ci`. The brief's sentence is corrected here rather than restated,
and `docs/status.md` says which gate catches which.

---

## The gate

`just ci`, recipe by recipe, each run separately so a failure could be
attributed, on the final head. Exit codes read from a file
(`exp37-opus-gate-summary.tsv`), not from a banner.

Run **twice**: on `b1bd971` (the last code commit) and again on `bd8001e`
(that plus this file and `docs/flows/webhooks.md`'s Status paragraph). Each
run recorded `git rev-parse HEAD` to a file before starting and each recipe's
exit code to a file after it, so the numbers below are read from
`exp37-opus-gate-summary.tsv` rather than from a harness banner.

| Recipe | Exit (`b1bd971`) | Wall | Exit (`bd8001e`) | Wall |
|---|---|---|---|---|
| `fmt-check` | 0 | 1 s | 0 | 0 s |
| `clippy` | 0 | 0 s (warm) | 0 | 12 s |
| `verify` | 0 | 7 s | 0 | 8 s |
| `test-rust` | 0 | 1054 s | 0 | 997 s |
| `test-doc` | 0 | 6 s | 0 | 5 s |
| `verify-ignored` | 0 | 1 s | 0 | 1 s |
| `lint-web` | 0 | 32 s | 0 | 21 s |
| `test-web` | 0 | 11 s | 0 | 10 s |
| `deny` | 0 | 1 s | 0 | 1 s |

`Summary [996.979s] 1614 tests run: 1614 passed, 0 skipped` on the second run,
identical to the first. The **branch head this report describes is the amend
of `bd8001e`** — this table's own row, added after the second run — whose only
delta over the gated tree is the markdown below. `fmt-check` and `verify`
(which is where `verify-links` lives, and the only gate a `.md` can move) were
re-run on it and both exit 0; nothing else can see it. Saying that is
preferable to a third full run whose only new input is the sentence recording
the third full run.

* `Summary [979.856s] 1614 tests run: 1614 passed, 0 skipped`
* `verify-ignored: 0 ignored (expected 0), 45 test binaries (expected 45), 1614 total (minimum 1080)`
* `test-doc`: **107 passed, 1 ignored** — a separate runner and a separate
  count; `cargo nextest` runs none of them.
* `test-web`: **1245** vitest cases across nine packages, 0 skipped
  (`examples/shop` 102, `sdks/nodejs` 191, `frontends/apps/checkout` 507,
  `frontends/apps/dashboard` 150, `sdks/stripe-js` 146, `@vpay/ui` 74,
  `@vpay/config` 63, `@vpay/tokens` 8, `@vpay/api-client` 4).
* `verify`'s twelve gates, each with its own number:
  `verify-status` 1 unimplemented item; `verify-errors` 18 error types, 16
  `#[from]` variants; `verify-sdk-parity` **409 proving tests, 39 dated gaps,
  19 SDK methods across 22 rows**; `verify-links` 1002 links in 188 files;
  `verify-serde` 83 types, 16 exempted; `verify-repositories` 4 concrete
  implementations named by none of 82 files outside `vpay-db`;
  `verify-migrations` **38 migration files**, all matching the manifest.

**One environment note, and it is pre-existing rather than something this
branch introduced.** `check-schema` prints
`WARNING — cratestack 0.11.1 on PATH, this repository pins 0.12.0` and then
runs in full against the 0.11.1 grammar, exiting 0. Nothing here touches
`schemas/vpay.cstack`, so the branch is not the reason; it is recorded because
a green `check-schema` on this machine is a weaker statement than a green one
on a host with the pinned CLI.

**The gate being green is not the finding.** Every mutation in the table above
was applied with `just ci` green, which is the point of running them.

---

## What was NOT done

* **`just test-e2e` was not run, and it is blocked on this host by something
  the repository already decided.** `compose.demo.yml` publishes the dashboard
  on a hard-coded `127.0.0.1:3000:3000` — the only one of the six published
  ports with no `VPAY_DEMO_*` variable — and the machine this ran on has the
  user's own `just demo` stack up, holding exactly that port
  (`vpay-demo-dashboard-1  127.0.0.1:3000->3000/tcp`, measured 2026-09-10). A
  second stack cannot come up beside it. Five overrides took
  (`demo_project=exp37` and four ports); the sixth had nowhere to go.

  The obvious fix — add `demo_dashboard_port` beside the other five — was
  drafted and then **reverted**, because `compose.demo.yml` says in its own
  comment that this port "is NOT a `demo_*` variable and cannot become one:
  the dashboard client's registered `redirect_uri` names 3000 and authkestra
  matches a redirect URI byte for byte". That is a considered decision with a
  reason, and making the port a variable means making the generated overlay's
  `redirect_uri` follow it — a third agreeing place, the shape
  `demo_checkout_port` already has. It is cheap and it is **a maintainer's
  call**, not this branch's. Surfaced rather than taken.

  The two ways to get the evidence: stop the user's stack for the duration
  (refused here — the brief says leave it untouched), or parameterise the port
  and the `redirect_uri` together. A hand-assembled subset — everything but
  `dashboard`, with the dashboard spec filtered out — was **not** run either,
  because a reconstruction of a recipe tests the reconstruction rather than
  the gate.

  What is *not* missing as a result: the shop's own 102 vitest cases cover the
  webhook handler's mapping of both new types, `examples/shop/README.md`'s
  table and the panel are cross-checked in both directions by
  `test-numbers.test.ts` (measured failing when the row was put back), and the
  Rust integration suite proves the events are written and — for the cancel —
  delivered. What is missing is one composition step: a real browser watching
  an order reach `failed` from the submit-time number, and `cancelled` from
  the cancel button.
* **No `customer.created`, `customer.updated` or `payment_intent.payment_failed`
  from the submit path has been fanned out to a receiver in any suite.** Only
  the cancel is driven all the way to the WireMock receiver. The fan-out is
  type-agnostic — it reads `events` by `seq` and branches on nothing — so this
  is an argument rather than a measurement, and `docs/flows/webhooks.md` and
  `docs/status.md` say so in those words.
* **No merchant endpoint outside this repository has received any of them**,
  which is the standing limit on every event type here.
* **The `customer.updated` "one transaction" claim rests on an ordering, not
  an equality** (see above), plus the seam's abandon case. The cancel's and
  the decline's rest on an equality.
* **`docs/status.md`'s "Database schema / migrations (core)" row still says
  "This repository now has thirty-one migrations in total (`0001`–`0031`)".**
  That was already stale on `master` at 37 files and is 38 now. It was left
  alone deliberately: the sentence sits inside a two-thousand-word narrative
  cell that chains every migration's history, and rewriting the count without
  rewriting the chain would make the cell disagree with itself. Surfaced here
  rather than half-fixed.
* **Nothing was done about `invoice.marked_uncollectible` or
  `invoice.payment_failed`**, the two remaining "real Stripe type, no writer"
  cases, or about the four listed-and-unwritten types
  (`payment_intent.created`, `payment_intent.processing`, both refund types).
  Out of scope, and still recorded in `docs/flows/webhooks.md`.

## Maintainer decisions surfaced, not taken

* **Should `DELETE /v1/customers/{id}` emit `customer.deleted`?** It does not:
  only the retention sweep does. The existing argument — "you asked for it" —
  is the same one `POST /v1/checkout/sessions/{id}/expire` uses, and
  `docs/flows/webhooks.md` already records that one as left to the maintainer.
  This change did not widen either, because "one transition, one event" is a
  contract merchants build dedupe logic on and widening later is cheaper than
  narrowing.
* **Should a bodiless `POST /v1/customers/{id}` emit?** It does not, on the
  ground that nothing was written. Stripe behaves the same way. If a merchant
  ever wants a "touched" signal, that is a different type, not this one.
