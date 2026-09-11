# `vpay-db` reference

Why the code in `backends/crates/vpay-db` looks the way it does. The crate's
own doc comments say _what_ each item is and link here; this page carries the
reasoning, the invariants and the history that a reader needs once — not on
every `cargo doc` build.

Tier: an [ADR](../adr/) records a decision, a [flow](../flows/) describes a
process, and a reference page like this one explains why a particular piece of
code is shaped the way it is. The migrations under `backends/migrations/` are
the schema's own record and are never edited (ADR-0003); this page is the
application's side of them.

- [The repository seam](#the-repository-seam)
  - [Why a trait object and not a generic parameter](#why-a-trait-object-and-not-a-generic-parameter)
  - [Why the transaction API is a closure](#why-the-transaction-api-is-a-closure)
  - [Why `PendingTransaction` owns its `sqlx` transaction](#why-pendingtransaction-owns-its-sqlx-transaction)
  - [What stays `pub`, and why](#what-stays-pub-and-why)
- [`payment_intents`](#payment_intents)
  - [`cancel` checks for a live charge inside the statement](#cancel-checks-for-a-live-charge-inside-the-statement)
  - [`set_payload` is a separate write from `reschedule`](#set_payload-is-a-separate-write-from-reschedule)
- [`checkout_sessions`](#checkout_sessions)
  - [The repository trait, for the lanes that call it](#the-repository-trait-for-the-lanes-that-call-it)
  - [Why the settlement flip is `pub(crate)` and not a trait method](#why-the-settlement-flip-is-pubcrate-and-not-a-trait-method)
  - [`expire` checks for a live charge inside the statement](#expire-checks-for-a-live-charge-inside-the-statement)
  - [`expire_due` is the same guard on a clock, and it emits the event](#expire_due-is-the-same-guard-on-a-clock-and-it-emits-the-event)
  - [`publishable_key` is a column, and `return_page_url` is a method](#publishable_key-is-a-column-and-return_page_url-is-a-method)
- [`charges`](#charges)
  - [One charge per intent is the index's job](#one-charge-per-intent-is-the-indexs-job)
  - [The charge read carries Postgres' clock](#the-charge-read-carries-postgres-clock)
  - [`mark_submitted` merges rather than assigns](#mark_submitted-merges-rather-than-assigns)
  - [The transition counter, and why its timing moved out](#the-transition-counter-and-why-its-timing-moved-out)
- [`settlement`](#settlement)
  - [Which intent statuses a settlement may land on](#which-intent-statuses-a-settlement-may-land-on)
  - [The `from` label degrades rather than failing a settlement](#the-from-label-degrades-rather-than-failing-a-settlement)
- [`events`](#events)
- [`refunds`](#refunds)
- [`webhook_deliveries`](#webhook_deliveries)
  - [`record_attempt`: what each column is allowed to say](#record_attempt-what-each-column-is-allowed-to-say)
  - [`pending_due` is a backstop, never a scheduler](#pending_due-is-a-backstop-never-a-scheduler)
- [`jobs`](#jobs)
  - [The lease is the whole design](#the-lease-is-the-whole-design)
  - [`enqueue_in_tx` exists only in the transactional form](#enqueue_in_tx-exists-only-in-the-transactional-form)
  - [`pull_forward_in_tx` is the exception, and it has to be asked for](#pull_forward_in_tx-is-the-exception-and-it-has-to-be-asked-for)
  - [Why claiming does not consider lease expiry](#why-claiming-does-not-consider-lease-expiry)
  - [Why a dead letter is parked and not deleted](#why-a-dead-letter-is-parked-and-not-deleted)
- [Dynamic SQL strings and sqlx 0.9](#dynamic-sql-strings-and-sqlx-09)
  - [The audit, done rather than asserted](#the-audit-done-rather-than-asserted)
  - [Why not `QueryBuilder`](#why-not-querybuilder)
  - [The two interpolations that are not constants](#the-two-interpolations-that-are-not-constants)
- [TLS: no `CryptoProvider` is installed here](#tls-no-cryptoprovider-is-installed-here)
- [CrateStack](#cratestack)
  - [What runs through it today](#what-runs-through-it-today)
  - [Migration 0032: what `currencies` and `providers` cost](#migration-0032-what-currencies-and-providers-cost)
  - [`reconcile`: read first, then write, and why that is the guard](#reconcile-read-first-then-write-and-why-that-is-the-guard)
  - [`providers` cannot be written through CrateStack, and that is a maintainer's decision](#providers-cannot-be-written-through-cratestack-and-that-is-a-maintainers-decision)
  - [Why the generated module is private, and what keeps it that way](#why-the-generated-module-is-private-and-what-keeps-it-that-way)
  - [What stays raw sqlx, and why](#what-stays-raw-sqlx-and-why)
  - [What compiling the schema does not prove](#what-compiling-the-schema-does-not-prove)
  - [Errors: why vpay classifies them itself](#errors-why-vpay-classifies-them-itself)
  - [The context, and the policy it satisfies](#the-context-and-the-policy-it-satisfies)
  - [What this added to the dependency graph](#what-this-added-to-the-dependency-graph)

---

## The repository seam

Every table family exposes one `#[async_trait]` trait (`Charges`, `Jobs`, …)
whose methods are the queries that family owns. `Repositories` is the umbrella
every consumer holds — `&dyn Repositories` in `vpay-api`'s router state and in
every `vpay-worker` handler — and `PgRepositories` is its only implementation,
built by `connect`. Nothing this crate exports takes a `PgPool`, and `PgPool`
is no longer re-exported at all.

ADR-0006 is untouched by any of it: `PgRepositories` is the sole
implementation, tests construct it against real Postgres, and no fake is
written.

### Why a trait object and not a generic parameter

`<R: Repositories>` would appear on every axum handler, on `AppState`, on
`RouterDeps` and on every job handler's signature. `dyn` costs one boxed future
per query on a path that is already awaiting Postgres — the same trade
`vpay_provider::ProviderAdapter` already documents and accepts (ADR-0002).

### Why the transaction API is a closure

`UnitOfWork::transaction` hands the closure a `&mut dyn TxRepositories` and
decides `COMMIT`/`ROLLBACK` from what the closure returns, so "forgot to
commit" is not expressible and no `sqlx` type leaves this crate.
`vpay-worker`'s `Cargo.toml` records that no-`sqlx` rule as deliberate; before
this seam existed it was a comment, and seven call sites spelled `pool.begin()`
anyway.

`TxOutcome` exists because a successful closure has two endings, not one:
`vpay_worker::webhooks`' fan-out loses a race to another drain and must roll
back _without_ that being an error, and the confirm path's duplicate-charge
recovery abandons its transaction and re-reads outside it. Encoding either as an
`Err` would push "not a failure" through the error channel, which is the shape
ADR-0011 exists to prevent.

`TxOutcome::Abandon` does not surface a rollback failure: it logs at `warn!` and
returns `Ok(Abandon)`. `ROLLBACK` is best-effort by construction, so a failure
changes nothing about the database and only about what the caller may report —
and both abandoning call sites have an answer that must survive (the confirm
path's `409`, and `persist_submitted`'s "a rail may hold a live payment"
alert). It is staged in `tests/postgres.rs` by terminating the backend that
holds the open transaction, with the commit path as the control.

The closure is generic over its error type rather than pinned to `DbError`.
Three call sites raise their _own_ layer's error from inside the unit of work —
the confirm path's "the rail accepted a charge whose intent moved" invariant
(`ApiError`) and two worker sites whose payload will not encode (`JobError`).
Pinning the closure to `DbError` would have forced each of them either to
smuggle the error out through the success channel or to relabel it as storage,
which is the exact shape ADR-0011 exists to stop. The signature is
`transaction<'a, T, E, F>(&self, f: F) -> Result<TxOutcome<T>, E>` with
`E: From<DbError> + Send`, so the common case is still spelled `E = DbError` and
each call site names its error type once.

### Why `PendingTransaction` owns its `sqlx` transaction

Not an aesthetic choice, and it looks like an easy simplification. The closure
signature `for<'t> FnOnce(&'t mut (dyn TxRepositories + 'a)) -> TxFuture<'t, _>`
is only usable because the `'a` on the trait object gives the implied bound
`'a: 't`, and that is what lets a closure borrow the caller's locals
(`&NewCharge`, a `&str` merchant id) across an `.await`. With a borrowing
`PgTransaction<'t>` the same signature forces every capture to be `'static`,
which no call site in this workspace can satisfy.

`PendingTransaction` carries no public method beyond the trait, so a caller
outside this crate can obtain one from `TransactionSource::begin_transaction`
and do nothing with it but hand it back — which is what makes `PgRepositories`
the only usable implementation of `TransactionSource`, and therefore of
`Repositories`, without a sealed-trait dance. Dropping it rolls back.

### What stays `pub`, and why

The table-family modules stay `pub` for the row and seed types, the two
`provider_requests` sentinels a test asserts on, and each family's own trait.
They hold no `pub fn` any more — a query is reached through `Repositories`,
never through a free function that would need a `PgPool` to call.

`lock_keys` is `pub` for one reason of its own: a test that wants to prove a
writer actually takes its lock has to be able to _hold_ that lock from outside
(`reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released`), and an
operator reading `pg_locks` needs the values to be findable from a crate doc
rather than by grepping for a hex literal.

`Migrations` is a trait on `Repositories` rather than a free function taking a
pool. `run_migrations` is not a table family and had no home once `PgPool`
stopped leaving the crate; making it a fourteenth trait is what let
`pub use sqlx::PgPool` go, where keeping one `pub fn` taking a pool would have
kept the whole re-export alive for one caller.

`connect_lazy` is `#[doc(hidden)]` and mechanically guarded. `connect` is
deliberately eager, which makes "a handle whose queries fail" unobtainable, and
`vpay-api`'s own unit tests need exactly that to prove an unreachable database
produces a refusal rather than an admission. It is **not** a test double — the
pool is the real `sqlx` one and every query really reaches Postgres — which is
why nothing in ADR-0006's dependency rules would object to a binary using it.
`cargo xtask verify-no-mocks` nonetheless fails the build if it appears in
non-test code anywhere under `backends/apps`.

`client_assertion_store` is the one place a _foreign_ trait
(`authkestra_op::client_assertion::ClientAssertionStore`) is implemented over
the database, and it was the precedent this seam followed.

It returns `impl ClientAssertionStore` rather than a named type, and that is
the seam again rather than a flourish: [ADR-0016](../adr/0016-engineering-standards.md)
standard 5 says an implementation is private to this crate and reached through
its trait, and until 2026-09-05 this store was the exception — it was `pub` and
`vpay-api` wrote `SqlClientAssertionStore::new(pool)` by name. `cargo xtask
verify-repositories` is what keeps it that way now. A free function rather than
a method on `Repositories`, because the trait it implements is authkestra's and
putting an authkestra type on vpay's umbrella trait would make the worker —
which mints no tokens — depend on the OP's vocabulary.

## `payment_intents`

Two rules the module exists to keep.

**Every query is merchant-scoped in SQL, not in Rust.** There is no
`get(id)`: the merchant is a parameter of the lookup itself, so a handler
cannot forget to filter and cannot leak another merchant's object by reading
first and comparing afterwards. A foreign id therefore comes back as `None` —
indistinguishable from a missing one, which is what
[merchant-auth.md](../flows/merchant-auth.md) requires, because an
authorisation failure that answers differently from a missing object is an
existence oracle.

**Status changes are compare-and-swap, never read-then-write.** `transition`
carries the expected status into the `UPDATE`'s own `WHERE`, so two concurrent
requests cannot both observe `requires_payment_method` and both act on it. A
validation function that is not part of the write statement enforces nothing
under concurrency; this one _is_ the write statement.

### `cancel` checks for a live charge inside the statement

`requires_payment_method` is not on its own enough to make a cancel safe. A
`confirm` commits its charge row — carrying the `provider_reference_id` it is
about to submit under — _before_ it calls the rail, and leaves the intent's
status alone until it knows what happened
([crash-safety.md](../flows/crash-safety.md)). So there is a real, reachable
window in which the status still says `requires_payment_method` while a live
charge exists.

Cancelling there would tell a merchant the payment was withdrawn while the rail
may hold it. A check in the _caller_ would not fix it: between reading "no
charge" and writing `canceled`, a concurrent confirm can commit one. Only the
write statement can decide this, which is why the `NOT EXISTS` is a predicate of
the `UPDATE` and not a preceding `SELECT`.

Charges in a terminal state do not block the cancel: nothing is in flight, and
"one charge per intent, forever" means the intent cannot get another. `Ok(None)`
therefore carries three meanings — no such intent for this merchant, an illegal
status, or a live charge — and the caller that needs to tell them apart re-reads
(`vpay_api::v1::payment_intents::cancel_once`, which turns them into a `404` and
two different `409`s). The status pair mirrors `vpay_core::state`'s
`Transition::Cancel`, and `cancel_is_legal_only_from_requires_payment_method`
plus `a_confirmed_intent_cannot_be_canceled` are what prove the two agree end to
end.

**There is no pooled `cancel` any more, and its absence is load-bearing
(2026-09-10, [issue #57](https://github.com/vaam-apps/vpay/issues/57)).** The
statement lives in `payment_intents::cancel_in_tx`, reachable only through
`TxRepositories`, because a cancel emits `payment_intent.canceled` and that
event has to commit with the status flip or not at all — the same rule
`settlement`, `checkout_sessions::expire_due` and `customers::erase_idle`
already apply. Leaving the pooled variant beside the transactional one would
have kept "cancel without an event" one call away; deleting it makes it not
expressible. `Ok(None)` is the answer that must write **no** event, and
`a_cancel_and_its_event_roll_back_together` is what proves the pair share one
unit of work — it abandons a transaction that ran both statements and requires
that neither survived.

### `set_payload` is a separate write from `reschedule`

The recovery table keeps per-job state in the payload — the `not_found_streak`
and `first_not_found_at` that decide when a charge the rail claims never to have
seen is resubmitted. That state has to survive the _current_ attempt even when
the job is not being rescheduled at all (it is being finished, or it is about to
fail), so it cannot ride along on the rescheduling statement.

The two writes are therefore not atomic with each other, deliberately: the worst
a crash between them can do is lose one increment of a counter whose only effect
is _when_ a resubmit happens. Making them one statement would mean either a
`reschedule` that silently rewrites a payload its caller did not mean to touch,
or a payload update that cannot happen without also moving the schedule. Neither
trade is worth the atomicity of a retry heuristic.

## `checkout_sessions`

Migration `0028`. One _checkout attempt_ driven through a page vpay serves —
`cs_…`, referencing an existing `pi_…`, carrying the merchant's forward URLs
and **two** payer credentials of its own. The three rules the module keeps are
`payment_intents`' two (merchant-scoped in SQL; compare-and-swap on status)
plus one: the single unscoped read is named `get_by_id_unscoped` so a `/v1`
handler that reaches for it has to type the word.

**Two credentials, not one, and that is the whole of D6.**
`client_secret_suffix` joins with the row's `id` into `cs_…_secret_…` and
rides in a URL _fragment_, which never leaves the browser; presenting it buys
the intent's own `client_secret`, and therefore the ability to confirm.
`return_token` rides in a _query string_ — it has to, because a fragment does
not survive a rail's redirect back to vpay — and buys strictly less: the
session and its intent without that credential. Both are 160 bits from the
same generator and both are redacted in `CheckoutSessionRow`'s hand-written
`Debug`; what differs is what they authorise, not how strong they are.

### The repository trait, for the lanes that call it

Written down here because Step 9's lanes build in parallel against it and a
signature is a contract before it is code.

```rust
#[async_trait]
pub trait CheckoutSessions: Send + Sync {
    async fn create(&self, new: &NewCheckoutSession)
        -> Result<CheckoutSessionRow, DbError>;

    async fn get_for_merchant(&self, merchant_id: &str, id: &str)
        -> Result<Option<CheckoutSessionRow>, DbError>;

    async fn get_by_id_unscoped(&self, id: &str)
        -> Result<Option<CheckoutSessionRow>, DbError>;

    async fn find_open_by_intent(&self, payment_intent_id: &str)
        -> Result<Option<CheckoutSessionRow>, DbError>;

    async fn find_latest_by_intent(&self, payment_intent_id: &str)
        -> Result<Option<CheckoutSessionRow>, DbError>;

    async fn list_page(&self, merchant_id: &str, page: &SessionListPage)
        -> Result<(Vec<CheckoutSessionRow>, bool), DbError>;

    async fn expire(&self, merchant_id: &str, id: &str)
        -> Result<Option<CheckoutSessionRow>, DbError>;
}
```

`find_open_by_intent` is the one lane 2 calls. Its contract, stated precisely
because a confirm depends on it:

- **Unscoped**, deliberately. The confirm path has already resolved and
  authorised the intent through a `MerchantScope`, so the id it passes is one
  the caller may act on; re-filtering by a tenant derived from that same
  intent would be an authorisation check against itself. That is
  `PaymentIntents::get_by_id`'s argument, unchanged.
- **`None` is the common answer and never an error.** Most intents are
  confirmed with no session in the picture at all, and the confirm path falls
  back to the merchant's stored `charges.return_url` for those.
- **At most one row, by construction.** The partial unique index
  `checkout_sessions_one_open_per_intent` is built over exactly this
  predicate, so "the open session" is a well-formed phrase rather than a
  `LIMIT 1` over an ambiguous set.
- **The whole row**, not the `return_token` alone: building the return URL
  needs the `id` too, and a two-value tuple is a shape that grows a third
  value the next time something is needed.

#### `find_latest_by_intent` — the same question with the `status` filter off

Added 2026-09-05, for the refusal `vpay_api::v1::return_trip` now makes: a
confirm on an intent whose checkout session is over is a `409`, and the
interesting case is exactly the one where **no** session is open. A lookup
whose `WHERE` says `status = 'open'` cannot tell "no session was ever created"
from "the session that was created is finished", and those two need opposite
answers.

`ORDER BY seq DESC LIMIT 1`, and one row is enough because an _open_ session
is always the newest one. That is a property of the schema and not a hope:
`checkout_sessions_one_open_per_intent` refuses a second insert while one is
open, so nothing can be newer than an open session. The direction that matters
in practice is the permissive one — an intent whose first session expired and
whose merchant then created a second, open one reads back the open one, so
"expire the abandoned checkout and offer a fresh link" is not refused.

`seq` and not `created_at`, because `created_at` is the caller's
(`NewCheckoutSession::created_at`) and two sessions could carry the same
instant; `seq` is the table's own insertion order, and a tie here would decide
whether a payer can pay.

Served by `checkout_sessions_intent_seq_idx` (migration `0030`), which had to
be added for it: 0028's only lookup by intent is _partial_
(`WHERE status = 'open'`), and this query cannot use it, because dropping that
predicate is the whole point. Without it the plan is a scan — of the table, or
of `checkout_sessions_seq_key` with `payment_intent_id` demoted to a filter —
on **every** confirm, including the majority that have no session at all,
which is the case with no matching row to stop early on. Measured at 200,000
sessions: 11.7 ms of parallel sequential scan against 0.047 ms through the
index. Pinned by
`postgres_smoke::the_confirm_paths_session_lookup_is_served_by_an_index`,
which asserts the plan and not only the index's existence.

`get_for_merchant` collides by name with `PaymentIntents::get_for_merchant`,
so every call site names its trait — `PaymentIntents::get_for_merchant(repos,
…)` — exactly as `list_page`'s callers already had to. That is more readable,
not less: the call now says which table it reads.

`SessionListPage` is a separate type from `ListPage` rather than that one with
a `payment_intent` field added, because `ListPage` is the payment-intent
list's contract and a filter on it would be a parameter that resource
silently ignores. The filter is applied _in the statement_, beside the tenant
filter: applied after `LIMIT` it would return short pages and a `has_more`
describing the wrong set.

### Why the settlement flip is `pub(crate)` and not a trait method

`checkout_sessions.payment_status` denormalises what the intent says, so a
payer's page can render an outcome from one read. That is only safe while the
two cannot disagree — and they cannot only if the session's write lands in the
_same transaction_ as the intent's.

So `checkout_sessions::settle_for_intent(tx, intent_id, paid)` takes a
`&mut PgConnection` and is `pub(crate)`, reachable from `settlement` and
nowhere else. The visibility is what enforces that rather than this paragraph:
a caller elsewhere — a handler, a repair script, a worker hook written after
the fact — would have to move a `pub` in the same diff, which is the moment
the argument has to be re-made. Same device, same reasoning, as
`payment_intents::succeed_after_submission`.

The Step 9 plan calls this "the worker hook" and locates it in
`vpay-worker/src/handlers.rs`. The _decision_ is indeed the worker's —
`settle_succeeded` or `settle_failed` — but the write is not: a second write
after the commit would leave a window in which the intent is `succeeded` and
the session still `open`/`unpaid`, and a crash in that window would make it
permanent, with no job that would ever notice. D10 adds none.

It is guarded on `status = 'open'`, so it is idempotent by compare-and-swap
like every other write in that transaction, and `Ok(0)` — no session, or one
already finished — is the normal answer rather than an error. `paid: true`
writes `paid`/`complete`; `paid: false` writes `failed`/`expired`, because
D10 has no `failed` session status: a session whose intent failed terminally
is reported as `expired` carrying `payment_status: failed`.

### `expire` checks for a live charge inside the statement

The same argument [`cancel`](#cancel-checks-for-a-live-charge-inside-the-statement)
makes, over the same `LIVE_CHARGE_STATES` constant, and worth repeating
because the consequence is different. `status = 'open'` is not on its own
enough to make an expiry safe: a payer's page may have confirmed seconds ago,
and a `confirm` commits its charge _before_ it calls the rail. Expiring there
would tell a merchant the checkout was abandoned while the rail may still take
the payment — and would then be contradicted by the settlement transaction
flipping the same row to `complete`/`paid`.

A check in the caller cannot close that window either, so the `NOT EXISTS` is
a predicate of the `UPDATE`. `Ok(None)` therefore carries three meanings — no
such session for this merchant, one that is no longer `open`, or a live charge
— and `vpay_api::v1::checkout_sessions::expire_once` re-reads to turn them
into a `404` and two different `409`s.

`payment_status` is deliberately untouched by an expiry. An expired session
that was already `paid` keeps saying so: the money is a fact about the intent,
and an expiry that rewrote it would be vpay telling a merchant a completed
payment had not happened.

### `expire_due` is the same guard on a clock, and it emits the event

`expire` is a merchant saying "I am done with this". `expire_due` is D10's 24
hours arriving, and it is what `vpay_worker::handlers::sweep_expired` calls on
its hourly pass. Until Step 9's lane 1b `expires_at` was written at create and
read by **nothing**: a session past its horizon reported `status: open` until
a merchant expired it by hand or the intent settled, so `status` could not
tell "still payable" from "abandoned yesterday".

It carries the identical `NOT EXISTS` live-charge predicate, and the reason
sharpens rather than weakens on a sweep: nobody is watching. A session whose
payer confirmed thirty seconds before the horizon has a rail holding a live
payment, and a background job that expired it would be contradicted by the
settlement transaction minutes later, with no request anywhere to correlate
the two. Such a session stays `open` until it settles — which is the honest
answer, because something is still driving it. Measured 2026-09-04: with the
clause deleted, `the_housekeeping_sweep_expires_a_stale_session_and_spares_a_paying_one`
(`backends/tests/integration/tests/checkout_sessions.rs`) fails with the
paying session `expired`.

**One statement became two functions on 2026-09-04**, when expiry started
emitting `checkout.session.expired` (migration `0029`). The read is
`due_for_expiry(now, limit)` and the write is
`expire_due(id, now, event_id, event_data)`, and the split is forced by what
an event _is_: `events.data` holds the **rendered wire object**, which only
`vpay-api` knows how to shape, so the row has to be read and rendered before
the write that describes it. That is the same order
`Settlement::apply_succeeded` and `vpay_worker::handlers::intent_snapshot`
have been in since Step 4, and it is why neither function can be the other's
`RETURNING`.

**The event and the flip are one transaction, and that is the whole point.**
`expire_due` opens its own transaction, runs the compare-and-swap, and — only
if it matched — appends the event before committing. The settlement's
argument, sharpened again: a session that says `expired` with no event is
**invisible**. There is no sweep over "expired sessions with no event", no
fan-out backlog row naming it, and D10 adds neither; the merchant simply never
hears, and their reconciliation sees an abandoned checkout they were not told
about. `a_failed_event_insert_leaves_the_session_open` proves the rollback
against a real CHECK violation, and the reverse — the flip committed before
the insert — was measured failing it on 2026-09-04.

The `type` is this module's own `EVENT_SESSION_EXPIRED` constant, not the
caller's, for the reason `settlement::EVENT_SUCCEEDED` is a constant: the type
is a property of _which transition this is_, and a caller free to choose it
could report an abandoned checkout as a settled payment.

**The guard is evaluated twice, on purpose.** `due_for_expiry` carries the
same `status`/horizon/`NOT EXISTS` predicate the write does. The write needs
its own copy because the read's answer is stale the moment it returns — a
payer can confirm in between, and `Ok(None)` on that path is the _normal_
answer rather than an error. The read needs one because rendering a session a
rail is still holding would mint an `evt_…` and build an object claiming the
checkout was abandoned, for a write that would then correctly refuse it: work
done for nothing, and one more place a future change could leak that object
out of.

Three things about the signatures. Neither is merchant-scoped, unlike every
other read here, because their caller acts for the deployment and not for a
tenant — the name says `due` rather than `all` for exactly that reason. Both
take `now` rather than comparing against Postgres's `now()` as the other two
sweeps do, because the horizon on the other side of that comparison was
computed in Rust at create (D10's constant belongs to the API, not to a
migration), and because it lets a test sweep a future instant instead of
rewriting a stored horizon; the sweep takes the instant **once** and passes it
to both, so a session that was due for the read cannot be undue for the write
a few milliseconds later. And `due_for_expiry` has a `limit` where the bulk
statement had none: an `UPDATE` that returned nothing cost one number however
many rows it touched, while this one materialises rows that each carry two
live payer credentials and each get their own transaction.
`vpay_worker::handlers::EXPIRY_PAGE` owns the value.

`payment_status` is untouched by either, exactly as in `expire`.

### `publishable_key` is a column, and `return_page_url` is a method

All three `/v1/browser/checkout` routes authenticate by publishable key plus a
session credential, so every URL vpay mints has to carry one as `?key=`: the
hosted page, the embedded iframe, and the return page.

**Why a column rather than a lookup at render time.** The return page is
reached from a URL the _rail_ holds — built once at submit, stored, and
replayed when the payer finishes. The documented key rotation is "add the new
one, deploy, remove the old", and a return URL derived from `merchant_id`
would stop resolving the moment the old key came out, stranding every payer
already sitting on a rail's page. Pinning the choice on the row makes the URL
stable for the session's life.

It is **not a secret** — it names a tenant and authorises nothing — so it is
printed in `CheckoutSessionRow`'s `Debug` while the two credentials beside it
are redacted. The column's CHECK is a shape backstop (`pk_` plus 1–124
characters) and deliberately looser than `vpay_config`'s
`pk_(test|live)_[A-Za-z0-9]{16,64}`: that rule includes a livemode agreement
this table cannot see, and a constraint restating two thirds of a rule is a
second copy that can drift. The real rule — _the key belongs to this session's
merchant_ — is the registration list, which no constraint can see either
(there is no merchants table; ADR-0003), so
`vpay_api::v1::checkout_sessions::chosen_publishable_key` is what enforces it.

**`CheckoutSessionRow::return_page_url(checkout_base)`** builds
`{base}/c/{id}/return?t={return_token}&key={publishable_key}`. It is a method
on the row rather than a `format!` in `vpay-api` because two callers construct
it — the confirm path, when a session drives the charge, and the return trip —
and every character has to be identical between them, since the _rail_ holds
the copy that matters. Both values are URL-safe by construction (`vpay_core`'s
base32 alphabet, and `pk_` plus `[A-Za-z0-9]`), so it is a `format!` and not
an escaping routine; a future alphabet that needed escaping would break
`vpay_core::ids`' own test first. A trailing slash on `checkout_base` is
absorbed, so `//c/…` — a protocol-relative URL naming a different host — is
not reachable through it.

## `customers`

The first vpay table **born** with a `schemas/vpay.cstack` model rather than
acquiring one afterwards, and the only one whose whole content is another
person's personal data. [`../flows/customers.md`](../flows/customers.md) is the
product document; this section is why the code is shaped the way it is.

### Two of eight methods go through CrateStack, and one column decides which

| Method                                                                                                                                                                                |                                                  |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `touch_last_used`                                                                                                                                                                     | **CrateStack** — `update_many(..).set(..)`       |
| the hard-delete branch of `erase_in_tx`                                                                                                                                               | **CrateStack** — `delete_many(..).run_in_tx(..)` |
| `insert_in_tx`, `lock_for_update`, `update_in_tx`, `get_for_merchant`, `list_page`, `idle_since`, `erase_idle`, and the anonymise branch and six trailing statements of `erase_in_tx` | hand-written `sqlx`                              |

_(The table read `create`, `update` and "two of seven" until 2026-09-10. The
two pooled writers are gone: `POST /v1/customers` and
`POST /v1/customers/{id}` each emit an event that must commit with the row, so
the statements moved behind `TxRepositories` and a third joined them —
`lock_for_update`, the `SELECT … FOR UPDATE` the update's `metadata` merge is
computed under. See "The customer writes are transactional" below.)_

The column is `metadata JSONB NOT NULL`, undeclared on `model Customer` for
the two costs that model's GAP note measures: `map_scalar` does not read
`jsonb` back, so declaring `metadata Json` would make the live column stay
invisible while the _declared_ one became a `[blocking]` drift line; and
`Value::from_plain_json` demotes any JSON number outside `i64` to `f64`,
silently, on a column that is merchant-authored and echoed back verbatim.

**That second cost is also what decided the shape of the GPS half of the
address** (2026-09-11, "address in our system means both formal as well as
GPS"). A coordinate had to be something this stack can carry exactly, and
`from_plain_json` routes every JSON number through `Number::as_i64()` — so a
decimal degree is a value CrateStack rounds. `address_latitude_microdeg` and
`address_longitude_microdeg` are therefore `BIGINT` counts of millionths of a
degree, declared on `model Customer` as `Int?` with a `@range` mirroring
`0041`'s CHECKs, and the unit is in the column name so no layer can read the
number as degrees. It is the money layer's rule — integer minor units with the
scale named — applied to the other quantity vpay stores at a fixed scale. The
[customers flow doc](../flows/customers.md) carries the product half.

What that model **cannot** say is the pair rule:
`address_coordinates_are_both_or_neither` is multi-column, this grammar has no
cross-field validator, and so the only places the rule exists are migration
`0041` and `vpay_api::v1::customers`' `400`. Two columns and three constraints
cost exactly **two** drift lines — the two single-column range CHECKs, which
are reported once each as `[safe] … is not declared` exactly as the six
`address_*_length` bounds are; the columns themselves and the multi-column pair
rule cost nothing. `EXPECTED_DRIFT_CHANGES` 177 → 179, measured.

The consequence is not obvious and is worth stating: because the model does
not declare the column, the generated model _struct_ has no field for it
either, so a CrateStack **read** could not render the wire object at all.
Every operation that touches `metadata` in either direction — create, update,
and every read — is therefore a hand-written statement, which is five of the
seven.

That is not a consolation prize. The two that did move are the two where being
wrong is irreversible: `touch_last_used` is the write the twelve-month
retention sweep reads, and `delete_many` is the hard delete of personal data.
It stayed on CrateStack through migration `0041` deliberately, even though its
caller became a `pub(crate)` function inside a larger transaction, because
`a_customer_delete_is_a_delete_and_not_a_soft_delete` pins the _rendered_
statement — and adding `@@soft_delete` to `model Customer` is a one-line
schema edit that compiles, passes `check-schema`, and turns every erasure into
a flag on a row that keeps the payer. Both
policy failures are **silent** — `update_many` and `delete_many` compile the
`@@allow` into the statement's own `WHERE`, so a deleted arm matches zero rows
and returns `Ok` — which is why `every_action_this_module_calls_has_an_allow_arm`
asserts the compiled descriptor with no container, and why it also asserts the
_absence_ of the three arms this model deliberately does not grant.

### `list_page` and the sweep stay raw SQL, and it is the query shape

`list_page`'s cursor is `seq < (SELECT seq FROM customers WHERE id = $2 AND
merchant_id = $1)` — a correlated subquery, which `cratestack::Filter` has no
constructor for. The two-statement alternative (resolve the cursor id to a
`seq`, then filter on the literal) is a _different_ query with a race in it:
between the two reads the cursor row can be deleted, and this table's rows
**are** deleted, by both `DELETE /v1/customers/{id}` and the retention sweep.
The one-statement form degrades to an empty page there.

`erase_in_tx` reads a `NOT EXISTS` triple over `payment_intents`,
`checkout_sessions` and `invoices` — correlated subqueries over _different_
tables, which `Filter` compares nothing of, and for which there is no
`@relation` to side-load because none of those tables is modelled.

It is one `const UNREFERENCED`, and it is a constant because `sql_audit` made
it one: it was first written as a function taking the outer query's alias, and
the gate failed on the computed `{guard}`. Both call sites spelled the table
`customers`, so there was no alias to parameterise.

**It stopped being a guard and became a branch on 2026-09-10** (migration
`0041`, issue #68). It used to be in `idle_since`'s `WHERE` as well, so a
customer any of the three tables referenced was skipped by the sweep and
refused by `DELETE /v1/customers/{id}` — which exempted from the twelve-month
retention promise exactly the payers vpay had taken money from. It now selects
between hard-deleting the row and anonymising it, and what a missing table
costs got worse: the branch would choose the hard delete, the `NO ACTION`
foreign key would raise `23503`, and the whole erasure — event and four
redaction statements included — would roll back, leaving the payer un-erased
with the merchant told nothing.
`the_sweep_guard_names_every_table_that_can_reference_a_customer` pins the
count at three, and
`an_invoiced_customer_is_anonymised_rather_than_deleted` is the behavioural
half.

### The erasure is seven statements in one transaction, and six of them are not about `customers`

`erase_in_tx` is the whole of issue #68, and the shape is what the issue got
wrong. The issue asks for identifiers to be redacted "on retained intents and
sessions"; an intent has never carried one. The copies that survived a
customer deletion were `events.data` — every `customer.*` body ever written,
in a table nothing prunes — `charges.payer_ref`/`payer_ref_masked`, reachable
from a customer only _through_ an intent, and
`idempotency_keys.response_body`, the exact JSON a `POST /v1/customers`
answered. The sabotage review of 2026-09-11 added two more found the same way:
`charges.failure_raw` and `refunds.failure_raw`, which are not identifier
columns and hold identifiers anyway — the rail's message kept verbatim, and a
rail refusing a collection names the subscriber. All five are rewritten in the
transaction that erases the row, because "vpay erased this payer" may not be
true of one table and false of four.

The seventh statement is not about a copy of the payer at all: rewriting
`events.data` changes the bytes a webhook delivery already mid-ladder would
re-render, and `webhook_deliveries.payload_sha256` is the digest the first
signed attempt recorded. So the deliveries of those events that are still
`pending` or `failed` have that column cleared in the same transaction, and
the next attempt signs and sends the redacted body. Without it the erasure
dead-letters exactly the delivery that announces it — measured, not argued,
in `an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering`.

The `events` and `idempotency_keys` statements share one `const
REDACT_CUSTOMER_KEY` — a `CASE` over `jsonb_each`'s key, naming the four
identifier keys and passing everything else through, so `metadata` (the
merchant's data) and `id` survive.

**The `address` arm replaces the whole key rather than walking into it**, and
since 2026-09-11 that is what makes the erasure reach a payer's GPS point
inside `data.object.address`. The replacement is `redacted_address_json()`,
which has to be key-for-key what `vpay_api::model::AddressObject` renders —
a stored body this rewrites is read back by a merchant on a replay or a
redelivery, and a key here the object does not have is a shape they meet
there and nowhere else. Its six formal components carry the marker and its
two coordinate keys carry JSON `null`, for the reason the column does: a
coordinate is an integer, the marker is not a value it can take, and a string
in a field both SDKs decode as an integer fails the decode rather than reading
as redacted. `the_redacted_address_body_is_the_shape_the_wire_renders` pins
the key set and the per-type value in milliseconds; the container-backed
scanner cannot tell an `address` reduced to `{}` from a redacted one, because
an absent key holds no identifier either. They cannot share the whole statement:
one reads `events.data` and the other `idempotency_keys.response_body`, and
`sql_audit` refuses a computed fragment, so the `FROM` differs and only the
rule is shared. That is the half that could drift, and the half a test can
read: `the_event_redaction_names_the_identifiers_and_spares_the_merchants_data`
asserts both directions of it in milliseconds.

`provider_requests` needs no statement, and that is a property of migration
`0016` rather than an omission: it stores a status code, an attempt number and
an operator-facing `error_kind` and no bodies at all. `webhook_deliveries`
holds no copy either — `0022` keeps a `payload_sha256` and not the payload —
and gets its statement for the opposite reason to a leak, above.

### The customer writes are transactional, and there is no pooled variant

`POST /v1/customers` emits `customer.created` and `POST /v1/customers/{id}`
emits `customer.updated` (2026-09-10, issue #66). Both events have to commit
with the row or not at all, so both statements live behind `TxRepositories`
and the pooled `Customers::create`/`Customers::update` were **deleted**. That
deletion is the load-bearing part: leaving them beside the transactional ones
would have kept "write the row and tell nobody" one call away, and no gate in
this repository objects to a method nobody happens to call.

`vpay-api` opens the transaction rather than `vpay-db`, for the reason
`insert_invoice_in_tx` gives: the event's `data` is the **wire object**, whose
shape this crate does not know, and it has to be rendered from the row the
statement returned — `seq` and both timestamps are the database's, so a
projection of the request would be a second implementation of the insert.

`lock_for_update` is the third method and the one worth reading the argument
for. `metadata` is merged key-wise in `vpay-api` (Stripe's contract), so the
written value is a function of the stored one and the update is a
read-modify-write. Reading on the pool left a window in which two concurrent
updates each adding one key lost one of them — documented, and tolerable while
nothing depended on the result being definite. An event is exactly such a
dependency: a merchant acting on a body describing the losing merge acts on a
state the database does not hold. So the read takes the row lock and the whole
sequence is one transaction. `a_locked_customer_read_waits_for_the_writer_and_then_sees_its_value`
forces the interleaving at this seam rather than racing for it, and
`a_customer_write_and_its_event_roll_back_together` is the abandon case.

The merge itself stays in Rust rather than becoming a `jsonb ||` in the
statement, deliberately: that would move Stripe's semantics into a migration
and out of the layer that documents them.

### `touch_last_used` is monotonic by _filter_, not by `GREATEST`

The method's contract is "a stamp never moves a customer's clock backwards",
and the obvious statement for that is `SET last_used_at = GREATEST(last_used_at,
$2)`. `UpdateCustomerInput` renders a plain assignment and cannot express it.

So the guard is the **predicate** instead: `where_(last_used_at.lt(now))`
means a stale stamp matches zero rows and is a no-op, which is the same
observable behaviour. `Ok(false)` therefore covers one more normal case —
"already stamped at or after this instant" — and the method's doc says so.

Why it matters at all: `now` is the _calling process's_ instant, two vpay
processes do not share a clock, and the horizon is twelve months. A rewind is
not a rounding error; it is the difference between a customer surviving a
sweep and not. It is also why migration `0034` carries **no**
`CHECK (last_used_at >= created_at)`: both instants come from process clocks,
so a server a second behind would turn a `POST /v1/payment_intents` into a
`500` for a reason no merchant could act on.

### The chrono/time boundary, crossed a second time

`to_chrono` is the second place this crate crosses it and the first in that
direction — `client_assertion::chrono_to_offset_date_time` converts the other
way for `authkestra-op`. CrateStack's generated inputs and filters take
`chrono::DateTime<Utc>`; every TIMESTAMPTZ this crate binds by hand is
`time::OffsetDateTime`.

The conversion has an `unwrap_or` that cannot be reached: chrono's range is
roughly ±262,000 years and `time::OffsetDateTime`'s (default features) is
years −9999..=9999, four orders of magnitude narrower.
`the_chrono_conversion_is_total_over_every_instant_time_can_hold` proves that
at both extremes rather than asserting it in prose, so a future `time` with
`large-dates` enabled turns the branch red instead of silently clamping.

ADR-0007 denies `expect`, so the unreachable branch still answers something,
and _which_ answer is not arbitrary: `MAX_UTC` means "freshly used" in both
slots this function feeds, so the branch fails in the direction that **keeps**
a merchant's personal-data record. `UNIX_EPOCH` would do the opposite — stamp
a live customer as maximally idle and hand it to the next sweep.

## `invoices`

The merchant's bill to a payer and the lines it is made of
(`backends/migrations/0036_create-invoices.sql`, S4b). Twelve repository
methods over two tables, plus three that are `TxRepositories` methods and one
that is `pub(crate)` to `settlement`.
[`../flows/invoices.md`](../flows/invoices.md) is the product document; this
section is the persistence argument.

### Two of twelve go through CrateStack, and three separate things decide it

`customers`' split had **one** cause (`metadata JSONB`). This family has
three, and each method's doc names the one that applies to it, because
collapsing them into "CrateStack cannot do it" would be false for two thirds
of the surface:

1. **`invoices.metadata` is `JSONB NOT NULL` and undeclared.** `model
Customer`'s two measured costs, unchanged — `map_scalar` does not read
   `jsonb` back, so declaring it would be a `[blocking]` drift line; and
   `Value::from_plain_json` demotes any JSON number outside `i64` to `f64`, on
   a column that is merchant-authored and echoed back inside every `invoice.*`
   webhook body. Every **read** of `invoices` has to carry that column, so
   every read stays raw.
2. **Three transitions write an `events` row in the same transaction**, and
   `Events`' insert is itself blocked on `events.data` (see "`events.data`:
   the one write that did not move").
3. **Every `invoice_items` write guards on a different table's column** —
   `EXISTS (SELECT 1 FROM invoices WHERE id = invoice_items.invoice_id AND
status = 'draft')`, which is what freezes an issued document.
   `cratestack::Filter` compares columns of the model's own table, and there
   is no relation that side-loads a parent's status into a child's `WHERE`.

What is left is exactly the two in the table above.
`mark_uncollectible` is the **only** transition with no event — Stripe's
`invoice.marked_uncollectible` is deliberately outside migration `0036`'s
vocabulary because nothing would write it — so it is the only one that is a
single statement, and a compare-and-swap `update_many` expresses the whole
operation. `items_for_invoice` goes through the generated layer because
`invoice_items` was **shaped** so it could: no `jsonb`, no `bytea`, no native
enum, no `int4`, which is migration `0035`'s discipline applied to a table that
also had to be readable through this layer.

### The transactions are opened by `vpay-api`, not here, and that is a departure

`settlement` and `Customers::erase_idle` both take a caller-rendered
`event_data` parameter and open their own transaction. The three invoice
transitions that emit an event do the opposite: they are
[`TxRepositories`] methods, so `vpay_api::v1::invoices` opens the transaction,
gets the written row back, renders the wire object **from that row**, and
appends the event with `insert_in_tx`.

The reason is specific rather than stylistic. Those two describe an object
whose post-write shape the caller can _project_ exactly — an intent that is
about to be `succeeded`, a customer that is about to be deleted. **A finalized
invoice cannot be projected**: its `number` comes out of a sequence the
statement itself advances and its `amount_due` is summed by the statement from
the lines, so a projection would be a second implementation of the assignment,
and the first thing it would get wrong is the number.

`invoice.paid` is the exception that proves it, and it is projected — see
below.

### The number sequence is a table row, and that is the whole design

`invoice_number_sequences` has no `.cstack` model, deliberately: its only write
is `next_number = invoice_number_sequences.next_number + 1`, a `SET` whose
right-hand side names the column being set, and `Update{Model}Input` carries
values rather than expressions. That is the same shape
`Customers::touch_last_used` wanted `GREATEST` for and could not have. A model
would declare a table nothing could write through, which this schema's header
says it does not do.

`INSERT … ON CONFLICT (merchant_id) DO UPDATE SET next_number = … + 1
RETURNING prefix, next_number - 1` does three things in one statement: it
creates the merchant's sequence on their first finalize (the row is born
pointing at 2 and that finalize takes 1), it advances it on every later one,
and it **takes the row lock that serialises two concurrent finalizes**. Under
`READ COMMITTED` the blocked transaction re-reads the committed value when the
lock is released, so two racing finalizes get two consecutive numbers.

It is not a Postgres `SEQUENCE`, and that is the point rather than an
oversight: `nextval` is non-transactional, so a rolled-back finalize would
burn a number. Migration `0036` argues why a hole matters more here than it
does for Stripe.

### `invoice.paid` in TX1, and why _this_ one is projected

`invoices::mark_paid_for_intent_in_tx` is `pub(crate)` and reached only from
`settlement::flip_invoice` — `checkout_sessions::settle_for_intent`'s
visibility argument verbatim: the point is that the write is not reachable
without the settlement it belongs to.

The worker projects the paid invoice before the transaction opens
(`vpay_worker::handlers::invoice_snapshot`), which is `intent_snapshot`'s
device. The projection is exact because an `open` invoice with a live intent
**cannot change**: `amount_due` was frozen at finalize, its lines are
immutable, and both `void` and `mark_uncollectible` refuse while the intent is
uncanceled (`NO_LIVE_INTENT`). If it changes anyway, the compare-and-swap
inside the transaction matches nothing and **no event is written at all** —
the fail-closed direction, pinned by
`a_settlement_whose_invoice_moved_emits_no_invoice_event`.

### The refund settlement, and why it is a second transaction rather than a hook

`Settlement::apply_refund_succeeded` (issue #91 D5, migration `0042`) opens one
transaction and runs two statements in it:

1. `refunds::settle_in_tx` — `UPDATE refunds SET status = 'succeeded' WHERE id
= $1 AND status = 'pending'`. `Ok(None)` is "already settled", the answer an
   at-least-once retry has to get.
2. `invoices::add_refund_for_intent_in_tx` — `UPDATE invoices SET
amount_refunded = amount_refunded + $2 WHERE payment_intent_id = $1 AND
status = 'paid'`. `Ok(None)` is "this intent pays no invoice", which is most
   of them.

Both are `pub(crate)`, so a consumer of this crate cannot settle a refund
without the document update in the same commit — `mark_paid_for_intent_in_tx`'s
visibility argument, applied to the other direction of the money.

**The increment is an expression, not a value the caller computed.** A
read-then-write would let two refunds settling concurrently both read `0` and
both write their own amount, losing one. The expression makes the second writer
block on the row lock MVCC already takes and re-evaluate against the first's
committed value — which is also what makes migration `0042`'s
`refunded_at_most_paid` a real ceiling rather than an advisory one.

**It fails closed, and the abandon test is the over-refund.** A refund past
`amount_paid` trips the CHECK on statement 2, after statement 1 has flipped the
refund inside the same transaction; everything rolls back and the refund is
still `pending`. Move the invoice write out — commit the flip, update on the
pool — and
`two_refunds_against_one_invoice_add_up_and_an_over_refund_is_refused` goes red
with the refund `succeeded` and the invoice unmoved, which is the permanent
inconsistency the transaction exists to prevent.

**No event.** `invoice.paid` is not re-emitted (the invoice did not
transition), and `charge.refunded` / `charge.refund.updated` stay types nothing
writes: emitting one needs the wire object `vpay-api` shapes, which is the
caller's to supply, and there is no caller.

**There is no caller at all, and that is the honest part.** No rail can refund,
`POST /v1/refunds` is unrouted and `Refunds` still exposes no `create`, so
nothing in `vpay-server` reaches this method and every deployment's
`invoices.amount_refunded` is `0`. It exists because D5 is a decision about
what the database does when a refund lands, and the alternative was to leave
that decision as a sentence in a document. `docs/status.md` carries the gap.

`payment_intents.amount_refunded` and `amount_refund_pending` (migration
`0003`) are deliberately **not** maintained here. Migration `0017`'s own GAP
note pairs them with the `INSERT` that creates a `refunds` row, and that insert
does not exist; incrementing one half of a paired total whose other half
nothing writes would leave `no_over_refund` counting money twice the day the
insert lands.

### `NO_LIVE_INTENT` is one rule with three call sites, and one of them cannot carry it

Voiding an invoice somebody is paying, writing it off, and minting a _second_
intent for it are three ways to end up with money moved against a document
that says nothing is owed. All three refuse on the same condition — the
attached intent is `canceled` — spelled once as a `const` so they cannot drift.

Two of them carry it in SQL. `mark_uncollectible` cannot: it goes through
CrateStack, and the condition is a correlated sub-select over
`payment_intents`. So `vpay-api` applies it from the row it has already read,
and the window that leaves is closed by **the settlement's own
compare-and-swap** rather than by the statement: a settlement that lands first
flips the invoice to `paid`, and `update_many`'s `status = 'open'` filter then
matches nothing. Neither ordering can produce an invoice that is both paid and
written off.

That is the one place in this module where a guard is not in the statement it
guards, and it is stated here for that reason.

### Migration 0042: what `amount_refunded` costs

**+1 change, 173 over 24, unmappable still 19.** Two of that migration's three
additions move nothing and the third is the whole of the +1:

- the **column** costs zero. `BIGINT`, no DEFAULT, declared in `model Invoice`
  as a plain `amount_refunded Int`, so both sides compare equal. The ADD's
  backfill DEFAULT is dropped on the very next line for exactly this reason —
  left in place it would have been a permanent
  `column amount_refunded default value differs` line, migration `0033`'s
  problem.
- `refunded_at_most_paid` costs zero because it is **multi-column**, and
  `migrate baseline` skips every multi-column CHECK in both directions. It is
  the load-bearing half of the migration and the report cannot see it at all,
  which is why it is in `postgres_smoke.rs`'s live-database CHECK inventory
  instead.
- `amount_refunded_non_negative` is the +1: single-column and hand-named, so
  one `[safe] … exists in the live database but is not declared in the schema`
  line, exactly like its six siblings on this table.

### Migration 0036: what the two tables cost

**156 changes over 23 relations, unmappable 19** (from 130 / 20 / 18). The +26
splits sixteen / nine / one across `invoices`, `invoice_items` and
`invoice_number_sequences`, and every line is a hand-named CHECK, an
undeclared index, a `seq` identity default, or the one permanent `status` type
line.

**`invoices_status_enum_check` costs nothing, and it is the first enum column
in this repository that does.** Migration 0032 had to _rename_
`providers.flow`'s hand-named CHECK after the fact, because `diff/checks.rs`
matches by name first; 0036 creates the constraint under
`naming.rs::check_name(table, column, "enum")`'s own spelling from the start,
so the declared constraint and the live one are one object and neither side
reports the other missing. The `[lossy] column status type differs (live:
Scalar("String"), schema: Enum("InvoiceStatus"))` line is still permanent, for
`introspect/postgres/enums.rs`' documented lossiness — a TEXT column has no
catalog representation that recovers an enum's name.

The five multi-column CHECKs contribute **nothing in either direction**, like
the fifteen before them, which is why
`the_invoice_invariants_are_enforced_by_the_database_itself` writes the row
each one refuses against a real Postgres.

## `charges`

Three writes, and only one of them is unguarded. `insert_for_intent` opens the
charge before the rail is called; `mark_submitted` and `mark_failed` record what
the rail answered, and both are compare-and-swaps out of `submitting` rather
than blind updates, so a recovery pass and a live confirm cannot overwrite each
other's answer.

The writes that take a charge to a _terminal_ state from anywhere in the live
set — what the worker's poll ladder decides — are not here. They move the
charge, the intent and an `events` row together and therefore belong to the one
transaction that does all three (see [`settlement`](#settlement)); splitting
them across this module would have made it possible to call one without the
others, which is the specific thing that transaction exists to prevent.

`insert_for_intent` takes a connection and not a pool because
[crash-safety.md](../flows/crash-safety.md) requires the charge row — carrying
the `provider_reference_id` the rail will be given — to be committed _before_
any network call. The confirm path therefore owns a transaction, and the insert
has to run inside it rather than on a second connection from the pool that would
commit independently.

### One charge per intent is the index's job

`insert_for_intent` does **not** check whether a charge already exists before
inserting one. The unique index `one_charge_per_intent` does that, and it is the
only thing that can: a `SELECT` followed by an `INSERT` leaves a window in which
two concurrent confirmations both see nothing and both write, which is precisely
the double-charge this rule exists to prevent. The `INSERT` is the check.

What this module adds is that the resulting `23505` arrives as
`DbError::UniqueViolation` naming `one_charge_per_intent`, so a handler can
answer `409` instead of the `503`-with-retry-advice an unclassified storage
error would produce. A handler may still read first (`get_for_intent`) to answer
a _friendly_ `409` without attempting the write — but that read is an
optimisation, never the guard.

### The charge read carries Postgres' clock

`Charges::get_by_id_as_of` answers a `ChargeAsOf` — the row, plus the `now()`
the same `SELECT` evaluated — and `get_by_id` is that read with the clock
dropped, so there is one statement and not two spellings of it.

The extra column exists because everything the worker decides about a
`submitting` charge is a **duration**: whether the state is evidence of a crash
or of a confirm still inside its rail call (sixty seconds,
[vpay-worker.md](vpay-worker.md#nothing-younger-than-the-window-is-recovered)),
and whether the charge is past the 24-hour escalation horizon. The subtrahend of
both is `charges.created_at`, which Postgres wrote. Until Step 8's review the
minuend was `OffsetDateTime::now_utc()` on the worker host, so the subtraction
spanned two machines' clocks: a worker sixty seconds ahead of the database
measured every charge as a minute older than it was, which made the recovery
window pass for every live confirm — the guard became a silent no-op on exactly
the deployment whose fleet clocks had drifted, and nothing in the data looked
wrong. The horizon leaned the milder way, escalating charges to `unresolved`
early.

Two statements (`SELECT now()` beside the row read) would not have fixed it
either: the gap between them is a scheduling delay, and a scheduling delay is
the quantity being measured. One statement, one transaction timestamp, and the
worker subtracts two values that came out of the same one — see
`vpay_worker::handlers`' `charge_age`, whose only job is that subtraction, and
`the_charge_read_carries_the_databases_own_clock_beside_the_row` in
`backends/crates/vpay-db/tests/repositories.rs`, which asserts the age moves
with `created_at` across the sixty-second boundary the worker compares against.

### `mark_submitted` merges rather than assigns

Every field the rail answered with moves in **one** statement, guarded on
`state = 'submitting'`. [crash-safety.md](../flows/crash-safety.md)'s
redirect-rail rule — "the commit is the gate on the redirect" — is a statement
about this write: the rail's `pay_token` (`ref_extra`) and the URL the payer is
sent to must become durable together, before anyone is handed the URL.
Splitting them across two statements creates a window in which a crash leaves a
payer stranded on the rail's page against a charge vpay cannot query. The state
guard is what makes it a state machine rather than a hope — a concurrent
recovery pass may have already advanced the same charge, and a blind
`UPDATE … WHERE id = $1` would drag it back to `submitted` and re-open a charge
the rail has already settled.

`provider_ref_extra` is **merged** (`||`, right-hand wins per key) and a `NULL`
argument leaves the column alone. The column is rail key material, and on a
redirect rail the `pay_token` in it is the only thing that can ever query the
charge again. `vpay_worker`'s `resubmit_charge` calls this with whatever the
rail answered the _second_ submit with, and a push rail answers with an empty
map; a plain assignment would overwrite key material with `{}` and leave a
charge nobody can ask about. Merging cannot lose a key; assigning can, and the
loss is silent and permanent.

`redirect_url` follows the same rule (`COALESCE($4, redirect_url)`): a `NULL`
argument means "this answer carried no URL", never "there is no URL". A plain
assignment would let a resubmit whose answer had no URL blank the only address
the payer can pay at while leaving the charge live — an intent in
`requires_action` with nothing to act on, which the API answers `500` for by
design.

Both merges are unreachable on today's paths: only a `submitting` charge
matches, nothing writes key material before the first answer, and a redirect
charge still in `submitting` is failed rather than resubmitted
(`RecoveryAction::FailDeadOrder`), so the only caller that could pass a second
answer never runs on the rail that has URLs. They are written as merges anyway
because "unreachable" is a property of today's callers and this is a column a
payer is standing on.

`return_url` is deliberately absent from the statement: it is the merchant's,
written at insert, and a rail's answer has no business overwriting it.

`mark_failed` is a separate function rather than `mark_submitted` with an
`Option<FailureCode>`, because the two writes are not variants of one decision:
a decline moves a charge to a **terminal** state and records the taxonomy
([failures.md](../flows/failures.md)), while a submit moves it to a live one and
records the rail's key material. One function would take five arguments of which
three are always `None`, and the call site would stop saying which happened.

### The transition counter, and why its timing moved out

`record_transition` — private, because the only correct callers are the six
statements' own modules — backs the three writes above and the three in
`settlement`, and nothing else. It lives in the database layer rather than in
the caller because _every_ transition passes through those six statements and
only some of them pass through the worker: a confirm opens and submits a charge
inside `vpay-api`, so a counter mounted on the worker's settlement points would
be silently blind to the busiest half of the state machine.

Two rules make the count mean what it says.

**Every label is read back off the returned row**, never off the caller's
argument, and the recording happens only after the statement returned a row — a
compare-and-swap that matched nothing is a transition that did not happen.

**A transition is counted after it is committed, never before.** The three
writes in `settlement` own their own transaction, so they record after their own
`COMMIT`. The three in `charges` run inside a _caller's_ transaction — that is
the whole point of taking a connection — so they cannot record at all: a
`ROLLBACK` after the insert, from a later statement in the same transaction
failing, would leave a counter claiming a charge that does not exist. Instead
each returns its row and the caller calls `record_opened` or
`record_left_submitting` **after** the commit. The seam is still this module —
the label vocabulary and the metric name are here and the callers pass no
strings — but the _timing_ has to belong to whoever owns the commit, because
nothing inside a transaction can know whether it will be committed.

Until 2026-09-03 all three recorded inline, and the module claimed the metric
"cannot claim a transition the database refused" while a rolled-back insert was
counted. What that timing costs now: a caller can _forget_ to record, which an
inline call could not, and a process that dies between the commit and the
recorder loses that transition for good — so the counter is at-most-once against
`charges`, never exactly-once, and drift after a crash is expected. Both
directions are pinned by tests rather than by review:
`a_rolled_back_charge_insert_counts_nothing_and_a_committed_one_counts_once`
fails if the recording moves back inside the statement, and
`a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`
(`worker_e2e.rs`) scrapes the running server and fails if any of the four edges
of one charge's walk goes uncounted, which is what happens when a caller drops
its call.

## `settlement`

**One transaction, three rows, no half-settled state.** A rail answering
`SUCCESSFUL` moves the charge to `succeeded`, the intent to `succeeded` with
`amount_received` filled in, and writes an `events` row a merchant will be told
about. `apply_succeeded` and `apply_failed` each write all three inside one
transaction, because every way of splitting them is a lie a merchant can
observe: a charge without its intent says the payment is still processing while
the money has moved; an intent without its event means the merchant's webhook
never fires and nothing retries it, because nothing knows it was missed; an
event without the rows is a webhook for a payment that did not settle.

**Idempotent by compare-and-swap, not by a flag.** Both guard the charge
`UPDATE` on the charge still being in a _live_ state. A re-run after a commit —
the poll job was rescheduled because the worker died between committing and
deleting the job, which is a normal outcome and not an error — matches zero rows
and returns `Ok(None)`; the caller finishes the job. Nothing is written twice,
and in particular no second `events` row, so at-least-once job execution does
not become at-least-twice webhook delivery for distinct event ids. That guard
has to be in the statement: a `SELECT` that checked the state first would leave a
window in which two workers — one holding a stale lease, one that just claimed
the reaped job — both see a live charge and both settle it.

**The charge is the record of a confirm; the intent may lag it.** A confirm
commits the charge (and its poll job) in one transaction _before_ calling the
rail, and moves the intent only afterwards, in a second transaction, once the
rail has answered. All three of [crash-safety.md](../flows/crash-safety.md)'s
kill points therefore leave a live charge against an intent still reading
`requires_payment_method` — not a corrupt database, but the ordinary state a
crashed confirm leaves and the one the recovery pass exists to resolve. So the
question these functions answer is never "does the intent's status agree that a
confirm happened": the charge answers that, because the compare-and-swap has
already matched a row in the live set and only a confirm writes one. The intent
write follows over a _wider_ set — the two confirmed statuses **and**
`requires_payment_method` — so a settlement lands whether or not the confirm
survived long enough to move the intent.

**Where the settlement's `from` label comes from.** The two settlement
statements need a `from` label their `WHERE` clause cannot supply, since it
matches a _set_ of live states rather than one, so each `RETURNING` carries an
extra `(SELECT prev.state FROM charges prev WHERE prev.id = charges.id)`. That
sub-select reads the statement's own snapshot — an `UPDATE` never sees its own
writes — so it yields the state the charge was in _before_ this statement. It
changes nothing about the compare-and-swap: the `WHERE` clause is unchanged, the
row lock is unchanged, and a statement that matches no row still returns no row.
The one honest caveat is that the snapshot is taken at statement start while the
guard is re-evaluated against the newest committed row version (Postgres'
read-committed recheck), so a charge another worker moved between the two —
`submitted` → `pending`, say — can be labelled with the earlier rung. `to` and
`provider` are exact either way, and they are what the alerting rules select on.

**What `None` does not mean.** It never means "the intent guard refused". After
the widening above, the only statuses left outside it are `succeeded` and
`canceled`, and neither can coexist with a live charge (`cancel` refuses to run
while one exists, and "one charge per intent, forever" means a settled intent
cannot acquire another). Either of them appearing is a broken invariant, and it
is reported as `DbError::WriteMatchedNoRow` — `Category::Internal`, which pages —
rather than being folded into the idempotent `None` a caller treats as "already
done". Committing the charge half and reporting success would leave the
merchant's intent permanently out of step with the money.

### Which intent statuses a settlement may land on

`SETTLEABLE_STATUSES` is `processing`, `requires_action` **and**
`requires_payment_method`. The first two are the confirmed statuses — a push
rail leaves the intent `processing`, a redirect rail leaves it
`requires_action` until the payer comes back. Both settlement writers guard on
the _set_ rather than on a single expected status supplied by the caller,
because the worker settling a charge does not know, and must not have to know,
which rail's flow put the intent where it is: branching on that in the caller
would be exactly the rail-shaped branch ADR-0002 forbids, while naming the legal
_values_ is not.

`requires_payment_method` is in the set because a crash puts it there.
Excluding it made the settlement of a crashed confirm unreachable: the charge
compare-and-swap would fire, the intent guard would match nothing, and the whole
transaction became `DbError::WriteMatchedNoRow` → `Category::Internal` →
`Retry::Never` → a dead-lettered poll job, with the charge left live and nothing
ever driving it again. A charge the rail may have collected is exactly what must
not be parked. It is safe because the settlement writers are never called on
their own — they run inside `settlement`'s transaction, _after_ a charge
compare-and-swap over the live states has already matched a row, and a live
charge is proof a confirm happened whatever the intent's status says. That is
also why they are `pub(crate)`.

`fail_after_submission` therefore performs a real
`requires_payment_method` → `requires_payment_method` write: the status does not
move and the write is the error pair alone, and counting that as _applied_ is
the point. It sits next to `record_payment_error` because the two are different
moments — that one is for a rail that declined at submit, where the intent never
left `requires_payment_method`; this one is for a decline the _poll_ discovered
after the intent had already moved, and
[payment-lifecycle.md](../flows/payment-lifecycle.md) is explicit that such a
failure returns the intent to `requires_payment_method` with
`last_payment_error` populated. The status change and the error pair must happen
in the same statement: an intent back at `requires_payment_method` carrying no
error reads to a merchant as one that was never attempted.

A merchant polling `GET` then sees a resolved intent that _looks_ confirmable
again. It is not — "one charge per intent, forever" means the failed charge
still blocks a second `confirm`, which answers `already_charged` and tells the
merchant a retry is a new intent. That guard is what makes this transition safe.

`succeed_after_submission` sets `amount_received = amount` rather than taking a
parameter. Neither rail vpay speaks to can settle _part_ of a submitted amount —
`ChargeStatus::Succeeded` carries a transaction identifier and no amount at all
— and taking one here would invite a caller to derive it from the charge, which
is already required to equal the intent's amount. When a rail that can partially
collect arrives, this becomes a parameter _and_ `succeeded` stops being the
right status; that is a change to the state machine, not a missing argument
today.

Neither writer is merchant-scoped, unlike every other query in
`payment_intents`. The caller is the worker settling a charge, not a merchant
addressing their own object, and there is no request whose authorisation could
be checked. Taking a `merchant_id` the worker would have to look up from the
intent it is already holding would _look_ like an authorisation check while
checking that the intent belongs to itself. The `id` comes from
`charges.payment_intent_id`, which is a foreign key.

The failure message is truncated to the column's 512 characters here rather than
left to the `lpe_message_length` CHECK: this write is the last statement of a
settlement transaction, and a rail whose text runs long would otherwise abort
the whole settlement — leaving the charge live and the job retrying forever
against a message that will be just as long next time.

### The `from` label degrades rather than failing a settlement

`PREVIOUS_STATE` is a correlated sub-select rather than an
`UPDATE … FROM charges prev` join: the join form changes how the statement is
planned and re-checked under a concurrent update, and this is the one statement
in the workspace that must not change shape for a metric label. It is aliased
away from `state` because `charges::COLUMNS` already returns a column of that
name, and two `state` columns in one row would make `ChargeRow`'s decode depend
on which one sqlx found first.

`decode_settled` reads it as `Option<String>` and `unwrap_or_default()`s, even
though it is `NOT NULL` in practice. The alternative is what makes that worth
writing: decoding straight into `String` would make a `NULL` — from a future
rewrite of the sub-select, or a schema change — a `DbError::Query` returned from
`apply_succeeded`, **a settlement that fails because a metric label could not be
decoded**. The charge is already `succeeded` and committed at that point, so the
caller would see a storage error and retry a settlement that has happened. A
`from` label reading `unknown` on a dashboard is a strictly smaller problem, and
it is visible: `a_settlement_counts_the_transition_it_actually_made` asserts the
real rung and fails in CI. The charge itself still fails to decode loudly — that
is the settled row, not a label.

## `events`

**The row is written in the same transaction as the state change** — not
afterwards, and not by a trigger. An event committed separately from the
transition it describes is either a webhook for something that did not happen
(the transition rolled back) or a transition no merchant is ever told about (the
event write failed), and the second is the one that actually happens, because it
is the failure nothing retries. So `insert_in_tx` takes a connection, never a
pool, and there is deliberately no pooled variant.

Events are written for **terminal transitions only** —
`payment_intent.succeeded` and `payment_intent.payment_failed`, both from
`settlement`'s single transaction. The milestone types
[webhooks.md](../flows/webhooks.md) also lists are not emitted by anything yet;
[../status.md](../status.md) is the record of which types are live.

`pending_page` is the backlog query the drain runs; `list_page` and `get_by_id`
are `GET /v1/events` and `GET /v1/events/{id}`, the documented fallback for a
webhook a merchant missed. Those two are merchant-scoped in SQL and page exactly
as `payment_intents::list_page` does; the handlers and the `EventObject`
renderer they and the deliverer must share live in `vpay-api`.

## `refunds`

**One read, no write, and the write's absence is the point.** `GET
/v1/refunds/{id}` was made part of the `/v1` contract on 2026-09-05 (issue
#45) because a refund is the one money movement on this surface with no
authoritative read: it is asynchronous and non-terminal (`pending`), the two
documented refund event types are emitted by nothing, and webhook delivery is
at-least-once and unordered. **Creating** one is a different question and is
still unanswered — `ProviderAdapter::refund` is `NotImplemented` on MTN
(refunds are the Disbursements product) and `Unsupported` on Orange — so
`Refunds` exposes `get_for_merchant` and nothing else. A `create` here would
be a write path no shipping code calls, which is a feature this repository
would be claiming it has.

**The tenant is reached by a join, and migration `0017` was deliberately not
altered.** `refunds` has no `merchant_id`; it has a `NOT NULL` foreign key
onto `payment_intents (id)`, and the intent is where the tenant lives. So the
one statement is

```sql
SELECT … FROM refunds r
  JOIN payment_intents p ON p.id = r.payment_intent_id
 WHERE p.merchant_id = $1 AND r.id = $2
```

A denormalised `merchant_id` column was the alternative and was rejected: it
would be a _second_ answer to "whose refund is this?", and two answers to a
tenancy question is how one of them ends up stale — for the cost of one
primary-key lookup per read. It would also have collided with the migration
numbering of two other branches in flight the same day, which is a reason to
notice the choice rather than a reason to make it.

`RefundRow` is a **projection**, not the whole table: `charge_id`,
`failure_code`, `failure_raw`, `provider_reference_id` and `updated_at` are on
the row in Postgres and on no wire object, and the writer that would fill them
does not exist. `fee` (migration `0031`, issue #46, 2026-09-06) **is** in the
projection, for the mirror-image reason: it is on the wire object as the tenth
key, so leaving it out would make the renderer invent a value. It is
`Option<i64>` all the way through — the column has no `DEFAULT`, `NULL` means
"the rail reported no fee" and `0` means "the movement was free" — and, since
nothing writes a `refunds` row at all, every value this repository can read
today is `NULL`. That is `events::EventRow`'s rule for `fanout_attempts`, not
`checkout_sessions::CheckoutSessionRow`'s one-to-one rule, and it is the right
one here precisely because guessing at the shape of code nobody has written is
what this repository calls claiming a feature.

## `webhook_deliveries`

**One row per (event, endpoint), created by the fan-out transaction.** The drain
reads the backlog and, per event, opens one transaction that creates a delivery
row per configured endpoint, enqueues a `deliver_webhook` job per created row,
and marks the event fanned out. All of it commits together, which is the only
arrangement in which a crash is harmless: an interrupted pass leaves the event
`pending` and the next pass redoes the whole of it, absorbed by
`webhook_deliveries_event_endpoint` and `jobs_dedupe_key`. Splitting the flip
from the inserts gives the two failures that matter — an event marked delivered
that has no delivery rows (a webhook nobody will ever send), or a second set of
rows for an event already fanned out (every webhook sent twice).

That is why `create_in_tx` and `mark_fanned_out_in_tx` take a connection and
there is deliberately no pooled variant of either.

**Why the `events` write lives in this module.** `mark_fanned_out_in_tx`
updates `events`, not `webhook_deliveries`. It is here rather than in `events`
because it is the _fan-out's_ closing write and is meaningless without the
inserts it commits beside: a caller that could reach it from the events module
could mark a backlog fanned out without creating a single delivery, which is
precisely the failure the shared transaction exists to make unreachable.

**Every column but `created_at` describes the most recent attempt.** `attempt`,
`state`, `status_code`, `response_excerpt`, `sent_at`, `responded_at` and
`next_attempt_at` are all rewritten by `record_attempt` and `record_success`.
This is a _state_ row with the latest attempt's outcome on it, not an
append-only attempt log — the per-attempt forensic trail is the worker's
structured log — and `payload_sha256` is the one column that deliberately does
not move.

The excerpt is truncated to migration `0022`'s `excerpt_length` ceiling here
rather than trusted from the caller, so an over-long excerpt cannot arise. The
worker cuts a receiver's body far shorter than that before it ever arrives; the
bound in this crate is the backstop, and it is what keeps the transport-failure
excerpt (which carries a whole `source()` chain) inside the CHECK.

### `record_attempt`: what each column is allowed to say

`status` is `None` for a transport failure and `responded_at` is cleared to
match. `status_code IS NULL AND responded_at IS NULL` with a `sent_at` set is
the encoding for "the request went out and nothing came back", which is why
migration `0022` deliberately carries no CHECK pairing those three columns.
Recording a heard refusal as an unheard one, or the reverse, is the one thing
this row must not do — the same argument `ProviderRequests::record_response`
makes for a rail.

`exhausted` is the caller's decision, not this layer's: the retry ladder lives
in `vpay_worker::delivery_delay` and `next_attempt_at` is the instant it
produced. The write is guarded on `state = 'pending'`, so a second call after
exhaustion changes nothing and a replayed job cannot walk `attempt` past the end
of the ladder.

`sha` is an `Option` because not every failed attempt rendered and signed
anything, and `None` leaves `payload_sha256` exactly as it was — including
`NULL`. The column records the digest of the bytes that were **rendered and
signed**, so an attempt abandoned before rendering must not stamp a digest for a
body that was never produced; the next attempt's mismatch check would then be
comparing against a body that never existed. _Rendered and signed_, not
_received_: a transport failure passes `Some`, because the signature was
computed over those exact bytes before the socket was ever opened.

When `sha` is `Some` it is `COALESCE`d rather than assigned, so the digest of
the _first_ attempt that rendered and signed a body survives — which is not
necessarily attempt 1. The handler compares its freshly rendered body against
the stored digest before sending and treats a mismatch as poisoned, so in every
non-buggy path the two are equal; keeping the earlier value means that if that
check is ever missed, the row still says what was originally signed instead of
quietly agreeing with whatever was sent last.

### `pending_due` is a backstop, never a scheduler

Delivery is driven by the `jobs` queue: the fan-out enqueues a job in the same
transaction that creates the row, and each failed attempt reschedules that job.
In a healthy deployment this query returns nothing. It is also the query an
operator runs to answer "what is outstanding right now?".

Two shapes qualify, and the second is why it takes a `lease`:
`next_attempt_at <= now()`, and
`next_attempt_at IS NULL AND created_at < now() - lease` — a delivery that has
**never** been attempted and whose job is not simply young. That second clause
was deliberately absent before migration `0023`, on the argument that a
never-attempted row's job was written in its own transaction so the two cannot
disagree. They can: the transaction makes the job _exist_, and nothing makes it
survive an operator's `DELETE` or a `jobs` truncation. Such a row was
unrecoverable, and the merchant is never told.

The `lease` is what keeps the scan from racing the queue rather than backing it
up: a delivery created moments ago has a job that has simply not been claimed
yet, and `RecoveryPolicy::lease` is the longest a claim may legitimately be
outstanding.

A returned row whose job was **dead-lettered** is a different case and is not
recovered by re-enqueuing — see
[vpay-worker.md](vpay-worker.md#the-outbox-drain) and
[webhooks.md](../flows/webhooks.md).

## `jobs`

### The lease is the whole design

A job is _claimed_ by an `UPDATE` that stamps `locked_at`/`locked_by` on exactly
one runnable row, and it is only ever finished or rescheduled by a statement
that also names the same `locked_by`. That guard is not decoration: without it, a
worker whose lease was reaped mid-run (it hung, the reaper freed the row,
another worker picked it up) would `DELETE` a job the second worker is in the
middle of executing, or reschedule it out from under them. This is ABA, and
`idempotency::claim` closes the same hole the same way with its `claim_id`.

### `enqueue_in_tx` exists only in the transactional form

The queue's one hard requirement is that the job and the write that creates the
work commit together. `confirm` opens its charge row before calling the rail
([crash-safety.md](../flows/crash-safety.md)); enqueueing the poll in that same
transaction is what makes _all three_ of that document's kill points leave a job
behind. A pooled `enqueue(pool, …)` would let a caller write the job on a second
connection that commits independently, which reintroduces both halves of the
failure it exists to prevent — a job for a charge that rolled back, and a
committed charge with nothing to drive it. So there is no such function.

It is deliberately not an upsert either. `DO UPDATE SET run_at = …` would let a
backstop scan drag a job already scheduled for an hour's time back to now, which
is how a poll ladder silently becomes a hot loop. `Ok(false)` — the `dedupe_key`
was already queued — is the normal answer for the backstop scan and for a
re-enqueue after a crash, not an error.

### `pull_forward_in_tx` is the exception, and it has to be asked for

Step 8 lane C added one write that _does_ move a scheduled job back to now:
`UPDATE jobs SET run_at = now() WHERE dedupe_key = $1 AND locked_at IS NULL
AND run_at > now() + $2 AND run_at < 'infinity'`. Its only caller is
`vpay_api::provider_callback` — a rail said something happened, and the point
of a callback is to ask the rail _now_ instead of at the ladder's next rung,
which is ten seconds away at best and fifteen minutes away after half an hour.

It is a separate method rather than the `DO UPDATE` the section above rules
out, and that is the whole distinction: an upserting `enqueue_in_tx` would
apply the pull-forward to every caller, including the backstop scan that
re-enqueues every live charge's key every ten minutes. One caller asking for
it is a callback; every caller getting it is a hot loop.

The three guards are each refusing a different thing:

- `locked_at IS NULL` — a leased job is being polled right now, and that poll
  will see the rail's answer. It is also the only way this write stays out of
  the `locked_by` discipline the lease section describes: it never touches a
  row someone holds.
- `run_at > now() + $2` — a job whose time has come needs nothing, and
  skipping the write is what makes a burst of duplicate callbacks (which both
  rails send) free rather than a queue of writers contending for one row lock.
  `$2` is the **floor**, added by Step 8's review: a job due within it is
  about to run, so moving it buys the rail nothing and costs an
  unauthenticated caller one rail request. The value is the poll ladder's
  fastest rung, and it is a _parameter_ because the ladder is
  `vpay_worker::poll_delay` — a policy about how often a rail is asked
  anything, which this crate must not hold (ADR-0002). The caller passes
  `vpay_api::provider_callback::PULL_FORWARD_FLOOR`, and
  [vpay-api.md](vpay-api.md#what-an-anonymous-caller-can-and-cannot-get-out-of-it)
  states what it does and does not bound.
- `run_at < 'infinity'` — a dead letter stays parked. The section below states
  that the occupied `dedupe_key` is what keeps a scan _or a callback_ from
  re-creating work a human has to look at first; this is the clause that makes
  the "or a callback" half true.

`Ok(false)` is therefore "nothing to do" in all of these cases and never a
failure, which matters because the caller always calls `enqueue_in_tx` first:
a job it just inserted at `now()` is the ordinary `false`.

### Why claiming does not consider lease expiry

`claim`'s predicate is `locked_at IS NULL`, full stop, so it matches
`jobs_claimable_idx` exactly. "Unlocked _or_ the lease has expired" depends on
`now()` and cannot be an index predicate, so it would turn every claim into a
scan over every leased row. Expiry is therefore a separate, periodic pass —
`reap_expired_leases` — which frees a stale lease _once_ and lets the ordinary
claim path pick the row up on its next turn. Its callers are described in
[vpay-worker.md](vpay-worker.md#two-lease-reapers-on-purpose).

### Why a dead letter is parked and not deleted

A job that is done is deleted (`finish`); a job that is not done is rescheduled
with its error recorded (`reschedule`). A job that _cannot_ be done —
`JobError::Poisoned`, or anything else `Classify::retry` answers `Retry::Never`
for — is neither, and `dead_letter` is the third write.

It exists because deleting one is not safe for a _payment_ queue. `poll_charge`
is the only thing driving a live charge to a terminal state; delete its row and
the charge is unattended, with nothing in the database saying why. The backstop
scan would then re-enqueue the same `dedupe_key` at its next pass and the same
failure would repeat every ten minutes, forever, with a fresh `attempts = 1`
each time — a hot loop that reads as a flapping rail rather than as a
permanently broken row.

Parking is `run_at = 'infinity'` (a real `timestamptz` value, not a sentinel
year) with the lease cleared. That single write is all four properties at once:
`claim`'s `run_at <= now()` can never match it, `reap_expired_leases`'
`locked_at` predicate can never resurrect it, the `dedupe_key` stays occupied so
no scan or callback re-creates the work, and `last_error` keeps the reason where
the operator handling the page is already looking. A `dead_lettered_at` column
would carry no fact these do not, and every reader of the table would have to
learn to exclude it.

The cost, stated plainly: a parked job is invisible to `oldest_runnable_run_at`
and to every other `run_at`-ordered query, so the _only_ way an operator learns
one exists is the alert the loop raises when it parks it, and
`SELECT * FROM jobs WHERE run_at = 'infinity'`. Requeuing one is an
`UPDATE jobs SET run_at = now()` by hand, which is deliberate: it should follow
a human deciding the underlying data is fixed.

`last_error` carries `vpay_core::error::source_chain` and not `Display` alone
(ADR-0011's amendment) — `ProviderError::Transport` keeps the `reqwest` error as
a `#[source]`, so the column would otherwise say "the request to the rail
failed" and never "operation timed out".

## Dynamic SQL strings and sqlx 0.9

Every statement in this crate is built with `format!` and then wrapped in
`sqlx::AssertSqlSafe`. Both halves need explaining, because the wrapper's name
is a promise and a promise nobody re-reads is worth nothing.

sqlx 0.9 (sqlx#3723) changed `query`, `query_as` and `query_scalar` to take
`impl SqlSafeStr`, which is implemented for `&'static str` and for the explicit
`AssertSqlSafe` wrapper — and for nothing else. A `String` built by `format!`
therefore no longer compiles as a statement. Under 0.8 this crate passed
`&sql` at 36 call sites; under 0.9 it passes `AssertSqlSafe(sql)` at the same
36 — **37 since 2026-09-05**, when `refunds::get_for_merchant` landed with
issue #45, **39 since 2026-09-06**, when `refunds::list_for_intent` and
`events::list_for_objects` landed with the `/dash/v1` payment detail (exp23),
and **45 since the same day**, when `customers` landed with S4a
(`insert_in_tx`, `get_for_merchant`, `update_in_tx`, `list_page`,
`idle_since`, `erase_idle` and, since migration `0041`, `erase_in_tx`'s
branch query, its anonymising `UPDATE` and two of its five redaction
statements — the other three are plain `&'static str` and need no wrapper;
`touch_last_used` and the hard-delete branch go through CrateStack and build
no string at all).
`EXPECTED_ASSERT_SITES` went **56 -> 60** in that change, a net +4 over five
additions and one removal, and its own doc comment enumerates them. Taking the
`String` **by value** rather than `AssertSqlSafe(&sql)` is
deliberate: the borrowed form goes through `AssertSqlSafe<&str>`, which sqlx's
own docs describe as copying the string.

The `format!` predates 0.9 and is not what that change is about. Each of these
statements ends in a column list — `RETURNING {COLUMNS}`, `SELECT {COLUMNS}` —
and that list is a `const … : &str` declared once per module so that a column
added to a `SELECT` cannot drift from the column read out of the `PgRow`. That
is the entire reason a statement here is not a literal.

### The audit, done rather than asserted

`AssertSqlSafe`'s contract is that the caller audited the string. Here is the
audit, re-done on 2026-09-05 from the source rather than inherited:

All 45 statements interpolate exactly two kinds of value. Re-done on
2026-09-06 for the six `customers` statements S4a added.

- **A `const … : &str` declared in this crate.** Thirteen of them:
  `charges::COLUMNS`, `checkout_sessions::COLUMNS`, `customers::COLUMNS`,
  `events::COLUMNS`,
  `payment_intents::COLUMNS`, `refunds::COLUMNS`,
  `webhook_deliveries::COLUMNS`,
  `checkout_sessions::OPEN`, `customers::UNREFERENCED`,
  `payment_intents::LIVE_CHARGE_STATES`,
  `payment_intents::SETTLEABLE_STATUSES`, `jobs::CLAIM_RETURNING` and
  `settlement::PREVIOUS_STATE`. A `const` cannot carry a caller's value.

  `customers::UNREFERENCED` is the one that had to _become_ a constant: it is
  the `NOT EXISTS` pair the retention sweep's read and its write must both
  carry, and it was first written as `fn unreferenced(alias: &str) -> String`
  so the two call sites could name the table differently. They do not — both
  spell it `customers` — and the audit is what said so, by failing on
  `{guard}`. A computed fragment fails this gate by construction however fixed
  its inputs are, which is the rule working rather than an inconvenience.

- **`direction`**, which is
  `let direction = if backwards { "ASC" } else { "DESC" };` — a `bool`
  choosing between two literals written in the same function
  (`events.rs`, `payment_intents.rs`, `checkout_sessions.rs`, in each case
  inside `list_page`). Postgres has no bind parameter for a sort direction,
  which is why it is interpolated at all.

**Re-checked 2026-09-06 for the two sites exp23 added, and for the one it
did not.** `refunds::list_for_intent` and `events::list_for_objects` are each
`SELECT {COLUMNS} … WHERE …` with `COLUMNS` the module's own `const` and every
caller value bound — `merchant_id`, `payment_intent_id`, and, in the events
read, a whole `&[String]` of object ids passed to `= ANY($2)` as one bound
array rather than a generated `IN (…)` list, precisely so the statement's
_text_ does not depend on the number of arguments. `payment_intents::list_page_filtered`
adds three predicates and **no** site: it is the existing `list_page`
statement with `($5::TEXT IS NULL OR status::TEXT = $5)` and two timestamp
bounds written the same way, so an absent filter sends the byte-identical
statement `/v1` sends and a filter value can never reach the SQL text.

**Re-done 2026-09-10 for issues #57 and #66, where the count moved 55 → 56 and
the net of +1 hides four additions and three removals.** Four terminal writes
left the pool for the caller's transaction, because each now commits with the
`events` row that reports it: `payment_intents::cancel` → `cancel_in_tx`
(interpolates `COLUMNS` and `LIVE_CHARGE_STATES`), `customers::create` →
`insert_in_tx` and `customers::update` → `update_in_tx` (both `COLUMNS`), and
the new `customers::lock_for_update` (`COLUMNS`), the `SELECT … FOR UPDATE`
that makes the update's `metadata` merge definite. The three pooled originals
were **deleted** rather than kept beside the transactional ones — which is
what makes "write the row and tell nobody" inexpressible — so the count moved
by one. Every one of the four interpolates crate constants and binds every
caller value, so the paragraph below is unchanged.

**Re-done 2026-09-10 for issue #91's D5, where the count moved 56 → 57 — one
addition, not a net.** `invoices::add_refund_for_intent_in_tx` is
`UPDATE invoices SET amount_refunded = amount_refunded + $2, updated_at = $3
WHERE payment_intent_id = $1 AND status = 'paid' RETURNING {COLUMNS}`. It
interpolates `invoices::COLUMNS` and nothing else; all three caller-supplied
values are bound, and the increment is an _expression over the row's own
column_ rather than a computed total, so there is not even an arithmetic
result to interpolate.

The other statement that landed with it, `refunds::settle_in_tx`, adds **no
site at all**, and that is worth a sentence rather than silence: it needs no
constant, so it is written as a plain `&'static str` and the compiler's own
check is never switched off for it. A statement that does not have to be a
`format!` should not be one — the count above is the budget, and this is what
spending nothing looks like.

**No caller-supplied value reaches a statement string anywhere in this crate.**
Every merchant id, intent id, cursor, limit, status, timestamp and payload is
already a bind parameter — the `.bind(..)` calls immediately below each
statement are the whole argument list.

That audit is a claim about a file that people edit, so it is also a test:
`vpay_db::sql_audit` (test-only, `backends/crates/vpay-db/src/sql_audit.rs`)
reads this crate's own sources and fails if a `format!` bound to `sql`
interpolates anything that is not one of the constants above or one of the two
named exceptions. It was proven to fire by three mutations on 2026-09-05, each
reverted — and it fired **unprompted** on 2026-09-06, twice, against S4a's
first draft of `customers.rs`: once correctly, on the computed `{guard}`
described above, and once as a false positive on a `format!("{column} = ")`
_inside a `#[cfg(test)]` assertion_, which the scanner read as a statement
because it looks for the word `sql` in the forty characters before a `format!`
and the assertion's own message printed `{sql}`. The test now builds that
needle with `concat` and says why. Worth recording rather than quietly working
around: the scanner is textual, so it will do this again, and the answer is to
avoid `format!` in a test that mentions `sql` — never to widen the
allowlist.

The three 2026-09-05 mutations:

- interpolating `{payment_intent_id}` into `charges::get_for_intent` →
  `every_interpolation_into_a_statement_is_a_crate_constant` fails, naming the
  file and the capture;
- redefining `direction` as anything other than the two-literal `if` →
  `the_audited_non_constants_are_still_what_the_audit_says_they_are` fails;
- wrapping a fresh `format!` in `AssertSqlSafe` instead of the audited `sql`
  variable → `every_assert_sql_safe_wraps_the_variable_the_audit_covers`
  fails.

The third is the one that matters most: without it the audit could be bypassed
by not using the variable the audit looks at.

**A fourth mutation, added by review on 2026-09-05, is why the list above was
not enough.** All three mutations above spell the interpolation by _name_.
Written positionally —

```rust
let sql = format!("SELECT {COLUMNS} FROM charges WHERE payment_intent_id = '{}'", payment_intent_id);
```

— the same injection passed all five tests, because the scanner discarded a
capture with no name. Since a positional capture's value comes from the
argument list, which this module does not resolve, it is now reported as a
violation on sight (`sql_audit::POSITIONAL_CAPTURE`), and
`a_positional_capture_is_reported_and_is_neither_a_constant_nor_allowed` pins
the scanner behaviour over synthetic text. **Every statement in this crate
captures its constants by name; `{}` is never the right spelling here**, which
is what makes a blanket refusal the correct rule rather than a heuristic.

### Why not `QueryBuilder`

sqlx's own suggested alternative. It was considered and rejected: it would
rewrite 45 working, reviewed statements to remove a risk the audit above shows
is not present, and it would replace SQL that reads as SQL with SQL assembled
by method calls — in a crate where the statement text _is_ the design
(`FOR UPDATE SKIP LOCKED`, `UPDATE … WHERE state = $2 RETURNING`, the
`NOT EXISTS` guards that make cancellation atomic). `QueryBuilder` earns its
place where the _shape_ of a statement varies with input. Nothing here has
that shape: the only variability is a sort direction and a fixed column list.

### The two interpolations that are not constants

`sql_audit`'s allowlist has exactly two entries and both are checked rather
than merely permitted:

- `direction` — the file must still contain the literal two-branch `if`.
- `columns` — `settlement.rs` writes `columns = crate::charges::COLUMNS` as a
  named argument, because it interpolates another module's constant and the
  implicit-capture form cannot name a path.

A third entry is a deliberate edit to that file, which is the review this
arrangement exists to force.

## TLS: no `CryptoProvider` is installed here

The root `Cargo.toml`'s comment on the `authkestra-*` dependencies documents a
real requirement: those crates build `reqwest` clients with `rustls-no-provider`,
so the _first_ one constructed panics unless a process-wide default
`CryptoProvider` was already installed. `sqlx` looks like the same hazard and is
not.

`sqlx` is configured with `tls-rustls-ring`, which vendors Mozilla's CA bundle
via `webpki-roots` (the runtime image is `FROM scratch` per ADR-0004, so there
is no OS trust store for the `-native-roots` alternative to read). Reading
`sqlx-core`'s own TLS setup (`src/net/tls/tls_rustls.rs`) shows it never
calls `rustls::crypto::CryptoProvider::get_default()` — the call that panics
without an installed default. It builds its own provider inline and passes it
explicitly:

**Re-read at 0.9.0 on 2026-09-05**, because a claim about a dependency's
internals is exactly what a major bump invalidates. Unchanged: `handshake`
still selects `rustls::crypto::ring::default_provider()` under
`_tls-rustls-ring-webpki` (the feature `tls-rustls-ring` expands to) and still
hands it to `builder_with_provider`. The `aws_lc_rs` arm next to it is
`cfg`-ed out by that same feature, and `deny.toml` bans the crate outright, so
both halves of the invariant are checked.

```text
let provider = Arc::new(rustls::crypto::ring::default_provider());
let config = ClientConfig::builder_with_provider(provider.clone())...
```

`builder_with_provider` never consults the process-wide default, so a `sqlx`
Postgres connection negotiating TLS cannot hit the "no default installed" panic
regardless of whether `install_default()` was ever called anywhere in the
process. **So this crate does not call `install_default()`, deliberately.** The
requirement in the root `Cargo.toml` is real but belongs to the dashboard-auth
work, and each binary installs the provider at boot
([vpay-config.md](vpay-config.md#the-boot-sequence), step 2).

---

## CrateStack

`vpay-db` compiles `schemas/vpay.cstack` with
[CrateStack](https://cratestack.dev)'s `include_server_schema!` macro
(`cratestack = { package = "cratestack-pg", version = "=0.12.0" }`) and runs
**thirty-two** statements through the generated data layer, spread over
**twelve** tables.
This section says which twelve, what deliberately did not move, and which of
CrateStack's behaviours vpay has had to work around rather than adopt. It is
the application's side of `schemas/vpay.cstack`'s own header, which carries
the schema's side.

**Corrected 2026-09-10 (issue #87), and the correction is two things.** These
two lines read ~~"**twenty-two** queries … over ten"~~ and ~~"**twenty-four**
queries … over nine"~~ _one after the other_, contradicting each other in
consecutive sentences — a conflict resolved by keeping both halves — and
neither was right. **Thirty-two over twelve** is measured, by the rule the
registry below states, and the running totals in the paragraphs that follow
were never re-derived after S4b, S5 or ADR-0017: read them as the history of
what each change _said_, not as arithmetic that adds up to today's number.

It said "**one** query" until 2026-09-06, when the two `disabled_clients`
writes followed the read; "**three**" until later the same day, when migration
0032 made `currencies` modellable and `ConfigReconcile::reconcile`'s currency
pass moved; and "**five**" until migration 0033 dropped the `providers`
capability defaults and the provider pass moved with them. It said "**six**"
until the outbox landed, and "**eight** over five tables" from 2026-09-06,
when S4a's `customers` added `touch_last_used` and `delete`.

~~**"Twenty-two over ten" since 2026-09-07** (S4b's `invoices` and
`invoice_items`, one query each — see "Migration 0036" below). It was
**"twenty over eight"** earlier the same day
([ADR-0017](../adr/0017-staff-authentication.md)),~~ ~~**"Twenty-four over
nine" since 2026-09-07** (S5, the money tables), and the +4 is
`checkout_sessions`' four reads~~ — **two totals written the same day on
branches that did not see each other, both kept, left contradicting each
other in consecutive sentences, and struck 2026-09-10 for their arithmetic
and not for their substance.** The substance is right, and the sections below
expand it: S4b added `invoices` and `invoice_items`, one statement each;
ADR-0017 added three whole tables; and S5 added `checkout_sessions`' four
reads — the first and so far only CrateStack statements on a table money
moves through. The section "The money tables: what moved, and what stays raw
forever" below is the whole account, including why the other three money
tables moved **nothing** and what would have to change upstream before they
could.

ADR-0017's increment is different in kind from every one before it: it is not
a method here and a method there, it is **three whole tables whose every
repository method runs through the generated layer** — sixteen of the
thirty-two statements in the registry below, exactly half, on tables created
for the purpose. `staff_members`
(**seven**, corrected 2026-09-10 — `create`, `find_by_email`, `find`,
`enrol_totp`, `record_totp_step`, `set_password`, `record_sign_in`; this said
six from the day it was written), `staff_sessions` (six) and
`oauth_authorization_codes` (**three** generated calls — `store_code`'s
`create`, and `consume_code`'s `find_unique` followed by the `update_many`
that is the swap; this said two, counting only the pair inside `consume_code`)
have no raw `sqlx` statement between them.

That is a property of migration `0035` rather than of ambition. Everything
this section records as a reason for staying on raw `sqlx` was designed out
of those three tables before they were created: no `jsonb` (which is what
keeps five of `Customer`'s seven hand-written and `Event`'s insert blocked),
no `bytea`, no native enum (the defect migration 0032 had to convert
`providers.flow` out of), no `DEFAULT` on any column a writer names (0033's
problem), and no `seq` cursor (whose correlated sub-select no delegate
expresses). The cost is stated where it is paid: the encrypted TOTP secret is
base64url `TEXT` rather than `BYTEA`, and `staff_members.last_totp_step` is
`NOT NULL` seeded to 0 because `NULL < step` is NULL in SQL and a nullable
column would refuse every staff member's _first_ TOTP code forever.

**The measured drift is the evidence.** The three tables cost 17 lines
(`EXPECTED_DRIFT_CHANGES` 113 -> 130) and every one of them is a hand-named
CHECK or an undeclared index — the two kinds 0.11.1 structurally cannot close.
Not one `column ... type differs`, not one `column ... default value differs`,
not one `column ... is declared in the schema but does not exist`, and
`EXPECTED_UNMAPPABLE_COLUMNS` does not move at all. Every earlier modelled
table carries at least one of those.

One more thing worth carrying: **the table name is decided by the model
name.** 0.11.1 derives it with
`cratestack_core::route_naming::pluralize(to_snake_case(model))` and has no
`@@map`, so `model Staff` reads and writes a table called `staffs`. The first
draft of migration 0035 created `staff`, every query answered `relation
"staffs" does not exist`, and **no gate said anything** — not `cargo build`,
not `just check-schema`, not `clippy`, not any of the ten `just verify` gates —
until a container-backed test ran. The model is `StaffMember` and the table is
`staff_members`; the Rust trait is still `vpay_db::Staff`, because it is a
trait about staff and not about a table.

Everything here was measured against the 0.11.1 sources on 2026-09-06;
`docs/plans/exp14-notes/opus.md`, `docs/plans/exp16-notes/opus.md` and
`docs/plans/exp17-notes/opus.md` have the transcripts.

### Re-checked at 0.12.0 (2026-09-07), and nothing below changed

The pin moved `=0.11.1` → `=0.12.0` (CLI and library together, as they must
be). The version numbers in this section moved with it, and they were
re-derived rather than substituted, because "we upgraded and the prose still
says what it said" is the failure this section is otherwise wide open to.

Five measured upstream gaps are named below. Four were measured at 0.11.1 and
**all four are still open at 0.12.0**; the fifth was found at 0.12.0 on
2026-09-07, when S5 declared relations on four money tables and every one of
the ten foreign keys came back as drift. The cheapest honest proof is file identity: each file the claim
rests on is byte-identical between the two releases (`md5sum`), and the
whole of `cratestack-core`, `cratestack-sqlx`, `cratestack-sql`,
`cratestack-pg` and `cratestack-policy` is unchanged at the source level. The
fifth gap needs no such argument: it is documented by the tool itself, in the
module that would implement it.

| Gap                                                                                                            | Where it lives at 0.12.0                                                                                                                                                                                  | State                                         |
| -------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| `@default(...)` fields are in neither `Create{Model}Input` nor `upsert_update_columns`                         | `cratestack-macros-0.12.0/src/model/inputs.rs:20-23` (`create_input_fields` filters `is_generated_on_create`), which is `has_default` at `src/shared/attrs.rs:91-93`; `model/descriptor/columns.rs:85-91` | **open** — both files md5-identical to 0.11.1 |
| `upsert(..)` gates the update policy on a _second_ pooled connection                                           | `cratestack-sqlx-0.12.0/src/query/write/upsert_exec.rs:45` and `upsert_resolve.rs:161-169` (`row_passes_update_policy(runtime.pool(), …)`)                                                                | **open** — whole crate's `src/` unchanged     |
| `Value::from_plain_json` demotes any non-`i64` number to `f64`                                                 | `cratestack-core-0.12.0/src/value.rs:95-106`                                                                                                                                                              | **open** — whole crate's `src/` unchanged     |
| `jsonb`, `bytea`, `int2`/`int4` have no read-back mapping, so introspection excludes those columns             | `cratestack-migrate-0.12.0/src/introspect/postgres/types.rs:16-36`, with the tool's own test asserting `map_scalar("int4", …) == None` at line 57                                                         | **open** — file md5-identical                 |
| Foreign keys are not introspected at all, so every `.cstack`-declared relation reports as a missing constraint | `cratestack-migrate-0.12.0/src/introspect/postgres/mod.rs:21-26` ("Known gaps"), `TableProjection::foreign_keys` set to `Vec::new()` at `:109`                                                            | **open** — found 2026-09-07 by S5             |

What 0.12.0 _did_ change, and why none of it reaches this document: the
breaking change gives `SchemaError` file identity, so `render()` takes no
arguments (`cratestack-macros-0.12.0/src/include/parse.rs:30-36`,
`cratestack-cli-0.12.0/src/migrate/baseline_cmd.rs:56-60`) — internal to the
macro and the CLI, and `just check-schema` reads an exit code rather than the
diagnostic. `cratestack-macros` also grew enum-typed query filters on
generated list routes (`src/shared/enum_query_parser.rs`, new), which vpay
compiles and does not call, and `cratestack-migrate` gained one doc comment
about `@computed` fields. The drift measurement was re-derived against a
fresh `postgres:16-alpine` at 0.12.0 and is unchanged at **101 changes / 16
relations / 17 unmappable columns**.

### What runs through it today

**Thirty-two statements, twelve tables, twelve models** — the whole list,
measured 2026-09-10 rather than accumulated. The rule, so it can be
re-derived: every `find_unique`/`find_many`/`create`/`upsert`/`update_many`/
`delete_many` chain in `backends/crates/vpay-db/src/*.rs` outside a
`#[cfg(test)]` module, each ending in exactly one `run(ctx)` or
`run_in_tx(tx, ctx)`. (`migrations.rs` has a `.run(&self.pool)` that is
`sqlx::migrate!`'s, not a builder's, and is not one of the thirty-two.)

**What this table used to be, because the shape of the error is the useful
part.** It listed sixteen statements over nine tables. It omitted
`staff_members`, `staff_sessions` and `oauth_authorization_codes` entirely —
the three tables the section above calls wholly generated, sixteen statements
between them, exactly half the total. It carried `get_for_merchant` twice,
once without its `run(ctx)`. And a blank line sat between the twelfth and
thirteenth rows, which ends a GitHub-flavoured table: the five
`checkout_sessions` rows rendered as a second table whose _header_ was the
first of them, so the column titles a reader used to read those rows were
never on screen. Every row below was re-read off the source.

| Method                      | Table                       | CrateStack builder                                                                                                                                                                                                                                                                                                                                      | Policy slot it needs      |
| --------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| `is_client_disabled`        | `disabled_clients`          | `find_unique(client_id).run(ctx)`                                                                                                                                                                                                                                                                                                                       | `read`                    |
| `disable_client`            | `disabled_clients`          | `upsert(CreateDisabledClientInput).run(ctx)`                                                                                                                                                                                                                                                                                                            | `create` **and** `update` |
| `enable_client`             | `disabled_clients`          | `delete_many().where_(client_id.eq(..)).run(ctx)`                                                                                                                                                                                                                                                                                                       | `delete`                  |
| `reconcile`, per currency   | `currencies`                | `find_unique(code).for_update().run_in_tx(tx, ctx)`                                                                                                                                                                                                                                                                                                     | `read`                    |
| `reconcile`, per currency   | `currencies`                | `upsert(CreateCurrencyInput).run_in_tx(tx, ctx)`                                                                                                                                                                                                                                                                                                        | `create` **and** `update` |
| `reconcile`, per provider   | `providers`                 | `upsert(CreateProviderInput).run_in_tx(tx, ctx)`                                                                                                                                                                                                                                                                                                        | `create` **and** `update` |
| `create_in_tx` (the outbox) | `webhook_deliveries`        | `upsert(CreateWebhookDeliveryInput).do_nothing().on_conflict(&["event_id","endpoint_id"]).run_in_tx(tx, ctx)`                                                                                                                                                                                                                                           | `create` **and** `update` |
| `mark_fanned_out_in_tx`     | `events`                    | `update_many().where_(id).where_(fanout_state).set(UpdateEventInput).run_in_tx(tx, ctx)`                                                                                                                                                                                                                                                                | `update`                  |
| `touch_last_used`           | `customers`                 | `update_many().where_(id).where_(last_used_at.lt(now)).set(UpdateCustomerInput).run(ctx)`                                                                                                                                                                                                                                                               | `update`                  |
| `delete`                    | `customers`                 | `delete_many().where_(id).where_(merchant_id).run(ctx)`                                                                                                                                                                                                                                                                                                 | `delete`                  |
| `mark_uncollectible`        | `invoices`                  | `update_many().where_(id).where_(merchant_id).where_(status.eq(open)).set(UpdateInvoiceInput).run(ctx)`                                                                                                                                                                                                                                                 | `update`                  |
| `items_for_invoice`         | `invoice_items`             | `find_many().where_(invoice_id).order_by(seq.asc()).run(ctx)`                                                                                                                                                                                                                                                                                           | `read`                    |
| `get_for_merchant`          | `checkout_sessions`         | `find_many().where_(id).where_(merchant_id).limit(1).run(ctx)`                                                                                                                                                                                                                                                                                          | `read`                    |
| `get_by_id_unscoped`        | `checkout_sessions`         | `find_unique(id).run(ctx)`                                                                                                                                                                                                                                                                                                                              | `read`                    |
| `find_open_by_intent`       | `checkout_sessions`         | `find_many().where_(payment_intent_id).where_(status).limit(1).run(ctx)`                                                                                                                                                                                                                                                                                | `read`                    |
| `find_latest_by_intent`     | `checkout_sessions`         | `latest_by_intent_query(..).run(ctx)` — `find_many().where_(payment_intent_id).order_by(seq.desc()).limit(1)`, extracted so `the_latest_session_query_orders_by_seq_and_takes_one` previews the builder the method runs                                                                                                                                 | `read`                    |
| `create`                    | `staff_members`             | `create(CreateStaffMemberInput).run(ctx)`                                                                                                                                                                                                                                                                                                               | `create`                  |
| `find_by_email`             | `staff_members`             | `find_many().where_(email).limit(1).run(ctx)` — `find_many`, not `find_unique`, because `email` is not the primary key                                                                                                                                                                                                                                  | `read`                    |
| `find`                      | `staff_members`             | `find_unique(id).run(ctx)`                                                                                                                                                                                                                                                                                                                              | `read`                    |
| `enrol_totp`                | `staff_members`             | `update_many().where_(id).where_(totp_enrolled_at.is_null()).set(UpdateStaffMemberInput).run(ctx)` — the second filter is the compare-and-swap that makes a re-enrolment lose                                                                                                                                                                           | `update`                  |
| `record_totp_step`          | `staff_members`             | `update_many().where_(id).where_(last_totp_step.lt(step)).set(UpdateStaffMemberInput).run(ctx)` — monotonic by filter, which is what refuses a replayed code                                                                                                                                                                                            | `update`                  |
| `set_password`              | `staff_members`             | `update_many().where_(id).set(UpdateStaffMemberInput).run(ctx)`                                                                                                                                                                                                                                                                                         | `update`                  |
| `record_sign_in`            | `staff_members`             | `update_many().where_(id).set(UpdateStaffMemberInput).run(ctx)`                                                                                                                                                                                                                                                                                         | `update`                  |
| `create`                    | `staff_sessions`            | `create(CreateStaffSessionInput).run(ctx)`                                                                                                                                                                                                                                                                                                              | `create`                  |
| `load`                      | `staff_sessions`            | `find_unique(id).run(ctx)`                                                                                                                                                                                                                                                                                                                              | `read`                    |
| `touch`                     | `staff_sessions`            | `update_many().where_(id).where_(last_seen_at.lt(now)).set(UpdateStaffSessionInput).run(ctx)`                                                                                                                                                                                                                                                           | `update`                  |
| `mark_authenticated`        | `staff_sessions`            | `update_many().where_(id).where_(state.eq(pending_totp)).set(UpdateStaffSessionInput).run(ctx)` — the state filter is the swap                                                                                                                                                                                                                          | `update`                  |
| `record_access_token`       | `staff_sessions`            | `update_many().where_(id).set(UpdateStaffSessionInput).run(ctx)` — writes `access_token`, `access_token_expires_at` (migration `0040`) and `last_seen_at` in **one** statement, which is what makes `staff_sessions_token_expiry_is_paired` a property rather than a convention: there is no instant at which the token is stored and its expiry is not | `update`                  |
| `delete`                    | `staff_sessions`            | `delete_many().where_(id).run(ctx)`                                                                                                                                                                                                                                                                                                                     | `delete`                  |
| `store_code`                | `oauth_authorization_codes` | `create(CreateOauthAuthorizationCodeInput).run(ctx)`                                                                                                                                                                                                                                                                                                    | `create`                  |
| `consume_code`              | `oauth_authorization_codes` | `find_unique(code_hash).run(ctx)`                                                                                                                                                                                                                                                                                                                       | `read`                    |
| `consume_code`              | `oauth_authorization_codes` | `update_many().where_(code_hash).where_(consumed_at.is_null()).set(UpdateOauthAuthorizationCodeInput).run(ctx)` — the swap: exactly one concurrent caller sees one row                                                                                                                                                                                  | `update`                  |

**Twelve tables, and therefore twelve of the file's seventeen models.** The
five with no statement at all are `PaymentIntent`, `Charge`, `Refund`,
`LedgerTransaction` and `LedgerEntry`. That split is checkable against the
schema without reading any of this: exactly those twelve models declare an
`@@allow` arm in `schemas/vpay.cstack` and the five declare none, so no
permission in the file is one no caller asked for. `schema.rs`'s
`the_three_money_models_answer_no_rows_to_every_action` holds that emptiness
for `PaymentIntent`, `Charge` and `Refund`; the two ledger models are in the
same position with no test watching them.

Plus one that is **test-only and says so**: `vpay-db`'s own
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx` reads a
`providers` row through `find_unique`. No production path _reads_ `providers`
through CrateStack — `model Provider`'s `read` arm is there for that test, and
migration 0033 changed the write, not the read. The test exists because
migration 0032's native-enum conversion has no other witness — see "The enum
conversion no report can see" below.

#### The transaction seam, now measured rather than argued

The last two rows are different in kind from the ones above them, and the
difference is the whole point of the outbox landing on this layer.

`reconcile` opens its own `self.pool.begin()`, so when it hands that
transaction to `run_in_tx` it is handing CrateStack a transaction `vpay-db`
opened for CrateStack's benefit — nothing else was ever going to be in it.
`create_in_tx` and `mark_fanned_out_in_tx` are reached through
`UnitOfWork::transaction`: the transaction is opened by
`TransactionSource::begin_transaction`, filled by
`vpay_worker::webhooks::fan_out_one` with a mix of CrateStack writes and
hand-written `jobs` inserts, and committed or rolled back by the `TxOutcome`
the closure returns. `run_in_tx<'tx>(self, tx: &mut sqlx::Transaction<'tx,
Postgres>, ctx)` cannot tell the two situations apart, which is exactly what
makes the seam work and exactly what makes it unprovable by reading.

`PendingTransaction` grew one private accessor for it. It already owned a
`Transaction<'static, Postgres>`; it now also carries a clone of the
`Cratestack` handle, because a delegate borrows from that value and a
`TxRepositories` method has no other reference in scope to borrow one from.
`Cratestack` is `Clone` over an `Arc`-shaped `SqlxRuntime`, so the clone adds
no connection — it is the same pool the transaction is already holding one of.
The accessor returns the pair `(&Cratestack, &mut Transaction)` in one call
because the borrow checker will not allow two `&mut self` methods to be held
across each other.

The two writes are `run_in_tx` and **not** `run`, and the difference is
proven rather than asserted:
`an_abandoned_fan_out_leaves_no_delivery_and_the_event_still_pending`
performs both writes inside a transaction that then returns
`TxOutcome::Abandon`, and reads the database back. Swapping either call to
`.run(&ctx)` makes it red in about a second, naming which one:

| Mutation                                | Measured                                                                                                                                         |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `create_in_tx` -> `.run(&ctx)`          | the delivery row **survives** the abandoned transaction                                                                                          |
| `mark_fanned_out_in_tx` -> `.run(&ctx)` | `fanout_state` is `done` on an event whose deliveries were rolled back — an event no drain will pick up again and no merchant is ever told about |

Neither mutation **hangs**, unlike the currency upsert's equivalent
(see "`reconcile`: read first, then write"), and the reason is the lock
shape rather than luck: on the branch these mutations take, this transaction
takes no `FOR UPDATE` on a row it then asks a second connection to probe. The
`do_nothing()` conflict probe finds no existing delivery and so locks nothing,
and the `events` row is held only by the `FOR KEY SHARE` the delivery's
foreign key takes, which does not conflict with the `FOR NO KEY UPDATE` a
non-key `UPDATE` wants.

**"On the branch these mutations take" is a correction the review made, and
the general claim it replaces was wrong.** `.do_nothing()` locks nothing only
on the `Inserted` branch. On the `Existing` branch `resolve_pre_probe` _is_ a
`SELECT … FOR UPDATE` on the caller's transaction, and `authorize_existing_row`
_does_ then ask a second, pooled connection about that same row. It still does
not hang, because a plain `SELECT 1` does not block on a `FOR UPDATE` row lock
in Postgres — the conclusion survives, its stated reason did not.

**The consequence that had gone unrecorded: on the `Existing` branch,
`create_in_tx` holds two connections at once** — the transaction's, and the
one `row_passes_update_policy` takes from the pool. Every write in this crate
before 2026-09-06 needed exactly one. That interacts with two numbers chosen
when a transaction meant one connection: `vpay_db::pool::MAX_CONNECTIONS` (10)
and the worker's `--worker-concurrency` (default 4, operator-settable). At the
default it fits (4 + 4 ≤ 10). At a concurrency of 10 it does not, and the path
that would queue on `ACQUIRE_TIMEOUT` is the `Existing` branch — i.e. crash
recovery, the one that only runs when something has already gone wrong.
~~Nothing enforces the relationship and nothing measures it; it is listed as a
maintainer decision in
[../plans/exp18-notes/opus-review.md](../plans/exp18-notes/opus-review.md) §3
rather than decided by a persistence-layer swap.~~ **Both halves of that
sentence stopped being true on 2026-09-10 (issue #63).** `vpay-server worker`
refuses a `--worker-concurrency` above `MAX_CONNECTIONS / 2` at boot, as exit
78, before it opens the pool — which is the only reason `MAX_CONNECTIONS` is
`pub` — and the ratio is measured against a real pool by
`the_boot_guards_maximum_concurrency_fits_the_pool_and_a_saturated_one_starves_the_reaper`
in `backends/tests/integration/tests/webhooks.rs`.

What that measurement says, and it is not what the arithmetic above assumes:
five simultaneous fan-outs on the `Existing` branch fit the pool with the
lease reaper still running alongside, and the reaper only starves when all ten
connections are pinned. Two connections per fan-out is a _peak_, held for the
width of the policy probe rather than the width of the transaction, and
`fan_out_events` is a **singleton** job — one row, claimed under a lease — so
a worker process has at most one fan-out in flight however high the
concurrency is set. `MAX_CONNECTIONS / 2` is therefore the conservative
ceiling the issue chose, not a measured cliff; the reasoning, and the one
question left open (whether the reaper and gauge loops should be reserved out
of it), is in
[../plans/exp45-worker-pool-bound-notes/opus-review.md](../plans/exp45-worker-pool-bound-notes/opus-review.md).

#### What the move narrowed: `create_in_tx`'s `Ok(None)` now means "an earlier **committed** pass"

The seam works. One thing behind it changed anyway, it is not visible in any
call site, and the review measured it rather than reading it off the
signature.

`create_in_tx`'s contract is that a repeat creation for one
`(event_id, endpoint_id)` answers `Ok(None)` — the quiet answer an
at-least-once drain needs. Through CrateStack that holds only when the
earlier row was **committed**. A second call inside the _same, still-open_
transaction is refused with `PersistenceError::Denied`:

```
first  = Ok(Some(f2ba579c-…))
second = Err(Persistence(Denied { model: "WebhookDelivery", action: "upsert",
                                  detail: "forbidden: update policy denied this upsert" }))
```

The statement it replaced, run twice in one transaction against the same
database, answered `Some(…)` then `None`.

The cause is that `.do_nothing()` decides its branch across two connections.
`upsert_do_nothing_probe.rs::resolve_pre_probe` runs `SELECT … FOR UPDATE`
**on the caller's transaction**, so it sees the uncommitted row and takes the
`Existing` branch; `upsert_do_nothing_authorize.rs` then re-checks the update
policy with `row_passes_update_policy(runtime.pool(), …)` — a `SELECT 1` on a
**pool** connection, which cannot see that row, finds nothing, and reads the
absence as a denial.

**Nothing in vpay reaches it**, and by two guards that live in other crates:
`vpay_config` refuses a duplicate webhook endpoint `id` at boot and
`EndpointRegistry::from_pairs` dedups by id, so `fan_out_one`'s loop cannot
call this twice for one pair. That is a real dependency the fan-out did not
have before — config validation in one crate now keeps a persistence call
correct in another — so it is stated in both places and pinned by
`a_repeat_creation_inside_one_transaction_is_refused_rather_than_reported_missing`,
which asserts the refusal _and_ the unchanged committed-row `None`. It is
reported upstream rather than worked around here.

#### `events.data`: the one write that did not move, and why

`TxRepositories::insert_in_tx` is still one hand-written `sqlx` statement,
inside the same transaction as everything else. It is the honest, ugly,
reversible half of this change and it is worth being precise about, because
"CrateStack cannot model a JSONB column" would be **false**.

0.12.0 has a `Json` scalar: `emit/postgres/columns.rs` maps it to `JSONB` and
`shared/types.rs` maps it to `::cratestack::Json<::cratestack::Value>`. Two
measured costs are why `model Event` still does not declare `data`:

1. **The drift is worse, not better.** `introspect/postgres/types.rs::map_scalar`
   does not map `jsonb` back onto any scalar. An _undeclared_ `jsonb` column is
   therefore invisible to the comparison in both directions — declaring the two
   models moved `EXPECTED_UNMAPPABLE_COLUMNS` not at all and produced no
   `column data exists in the live database` line. Declaring `data Json` would
   leave the live column invisible while adding a `[blocking] column data is
declared in the schema but does not exist in the live database` line, which
   is what `currencies.exponent` did before migration 0032.
2. **The conversion is lossy, and this is the column it must not be lossy on.**
   `cratestack::Value` is not `serde_json::Value`. `Value::from_plain_json`
   routes every JSON number through `Number::as_i64()` with an
   `as_f64().unwrap_or_default()` fallback, so a `u64` above `i64::MAX`
   anywhere in the payload returns as a float. `events.data` is the wire
   object that is stored, **signed** and delivered to a merchant
   ([../flows/webhooks.md](../flows/webhooks.md)), and it embeds
   merchant-authored `metadata` — arbitrary JSON vpay does not choose.

Because `data` is `JSONB NOT NULL` with no `DEFAULT`, a model without it
generates a five-column `INSERT` that Postgres refuses with `23502`. That is
not left as prose: `the_events_insert_cannot_move_until_a_json_column_can_be_modelled`
pins the rendered statement with no database, and
`a_generated_events_insert_is_refused_by_the_not_null_on_data` runs exactly
that statement against a real one and asserts the SQLSTATE. Both go red the
day the situation changes, which is the point of them.

The reads stay raw too, and for a second reason on top of `data`:
`Events::list_page`'s cursor is a correlated sub-select (`seq < (SELECT seq
FROM events WHERE id = $2 AND merchant_id = $1)`) that no delegate expresses.

#### The event vocabulary stays a hand-named CHECK, and that is a live hazard

`events.type_is_a_documented_event` and `events.fanout_state_is_known` are
multi-value single-column CHECKs, and 0.12.0 has no validator that expresses
"one of these eight strings" on a `String` column. Both candidates were
measured and both rejected:

- A `.cstack` enum **would** match in principle — `providers.flow` proves
  Postgres deparses `IN (...)` into the shape `reconstruct_enum` reads back —
  but only under the generated name `events_type_enum_check`. `diff/checks.rs`
  matches by name first, and the live constraint is
  `type_is_a_documented_event`, so the report would grow a drop-and-add pair
  rather than converge. Renaming needs a migration this change is scoped not
  to add.
- `@db_enforce` on a length validator expresses nothing about a vocabulary.

**The hazard, measured 2026-09-06.** Because the schema does not declare the
constraint, a generated `cratestack migrate diff` would emit DDL dropping it.
Nothing runs `migrate diff` in this repository, so that half is latent rather
than live — and it applies equally to every other `[safe] CHECK ... exists in
the live database but is not declared` line in the report.

**What notices, corrected on 2026-09-06 by the review.** This section said
the drift report was "structurally incapable of complaining" and that deleting
the constraint "fails no drift assertion at all". The first half of that is
right and reproduces — a lost CHECK removes one `[safe] ... is not declared`
line, so the count goes **down**, 101 to 100 — but the conclusion drawn from
it was wrong. `EXPECTED_DRIFT_CHANGES` is an exact `assert_eq!`, not a floor,
so `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` fails
on a _lower_ count exactly as loudly as on a higher one (`left: 100,
right: 101`), and its own assertion message already names the diagnosis: "If
it shrank without an edit to the schema, find out what the report stopped
seeing before moving anything." That is the general defence for every
undeclared-CHECK line above, and it was already there.

Two signals, saying different things, which is why
`an_undocumented_event_type_is_refused_by_the_database` is still worth having
and is not a duplicate. The count says _a constraint the database had is
gone_, without saying which or whether it mattered. The test says _the
vocabulary is still closed_, which is the property four documents actually
rest on — and it is the signal that survives someone re-pinning the constant,
which is the cheapest way to make a red drift assertion go away. It was
written here because the constraint was cited by four documents and asserted
by nothing.

Nothing about the trait changed for any of them — the signatures, the
deliberate absence of a cache, and `enable_client`'s "a no-op, not an error"
contract are all as they were. What did change is the `# Errors` contract:
all three now return `DbError::Persistence` rather than `DbError::Query`. A
caller branching on the _classification_ sees nothing (a `23505` is still a
`409 resource_conflict`); a caller matching the variant would have silently
stopped matching, which is why both trait doc comments say so.

`disabled_clients` went first because the migration needed to model it is
empty. It is three columns (`TEXT PRIMARY KEY`, `TIMESTAMPTZ NOT NULL DEFAULT
now()`, `TEXT`), every one of which is inside `cratestack-migrate`'s
`map_scalar`; it has no CHECK constraint and no enum column. `currencies`
needed `exponent INT` widened to `BIGINT` first (`int4` is unmapped), and
`providers` needed its native `provider_flow` enum converted to `TEXT` +
CHECK, because CrateStack reads every enum column with `try_get::<String>`
and a native Postgres enum decodes as an error. **Migration 0032 did both**,
and the three sub-sections below are what it cost and what it bought.

The read went a day before the writes on purpose, and the sequencing is worth
recording because it is the only reason the parity test proves anything:
moving a read and a write together produces a test that cannot say which of
the two it is testing. `a_disabled_client_reads_the_same_through_both_paths`
was written while the write was still hand-written sqlx; when the write moved,
that test's seed became an inline `INSERT`/`DELETE` rather than a call to
`disable_client`, for exactly the same reason. Its sibling
`a_client_disabled_through_cratestack_is_visible_to_both_paths` covers the
writes, reading every assertion back through a plain `SELECT`.

Both were executed against a real Postgres on 2026-09-06 and both pass. So
did the six mutations in "What a missing policy costs, per action" below.

#### `disable_client`: the same statement, and one extra round trip

`.upsert()` renders

```text
INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2)
ON CONFLICT (client_id) DO UPDATE SET reason = EXCLUDED.reason
RETURNING client_id AS "client_id", disabled_at AS "disabled_at", reason AS "reason"
```

which is the hand-written statement it replaced plus a `RETURNING` nobody
reads. Two properties of that rendering carry the trait's documented
behaviour, and both come from the same predicate in `cratestack-macros`'
`model/descriptor/columns.rs` and `model/inputs.rs`: a `@default(...)` column
is in neither `CreateDisabledClientInput` nor `upsert_update_columns`. So
`disabled_at` is left to the column default on insert, and is never assigned
on conflict — which is what makes "a second disable leaves the original
`disabled_at` untouched" true rather than hoped for. `reason` is assigned, and
assigned from `EXCLUDED` rather than a `COALESCE`, so `disable_client(id,
None)` clears the note.

The cost is a round trip. `.upsert()` is transactional by construction: it
opens its own transaction, probes the conflict target with a `SELECT ... FOR
UPDATE`, and may run an `ON CONFLICT DO NOTHING` before the real statement
(`upsert_resolve.rs`, cratestack#745), all to tell a create from an update for
the event and audit fan-out that `model DisabledClient` has neither of. That
is CrateStack's fixed shape for `upsert` and it is accepted rather than worked
around: an operator disabling a client is not a hot path. `is_client_disabled`
is — it is on the token-issuance path — and it stayed a single `find_unique`.

**On the conflict branch the cost is a round trip on a _second_ pooled
connection**, which is a different kind of cost and was not written down until
the review of 2026-09-06 measured it out of the source.
`UpsertRecord::run` begins its transaction on `runtime.pool()` and holds that
connection for the whole call; `upsert_resolve.rs::gate_update_policy` then
calls `row_passes_update_policy(runtime.pool(), …)`, which `fetch_optional`s
on the **same pool** while the transaction's connection is still checked out.
So a second disable of an already-disabled client needs two of
`pool.rs`'s `MAX_CONNECTIONS = 10` at once. Ten concurrent ones would each
hold the first and wait `ACQUIRE_TIMEOUT` (5 s) for the second, and all ten
would then fail as `PersistenceError::Backend` → `Category::Storage`.

Not reachable today and recorded rather than worked around: `disable_client`
has no shipping caller at all yet (see [roadmap.md](../roadmap.md)), and the
_insert_ branch takes only one connection, because `auth().isSystem()` is not
a relation predicate and `evaluate_create_policies` therefore issues no query
for it. It is the first thing to re-check if a route or an admin surface ever
calls this method concurrently, and it is on the list of things worth sending
upstream ([docs/plans/exp16-notes/opus-review.md](../plans/exp16-notes/opus-review.md) § 6):
the probe could run on the transaction's own connection, which already holds
the row lock. Still `runtime.pool()` at 0.12.0 (2026-09-07).

#### `enable_client`: why `delete_many` and not `delete`

`.delete(pk)` is the builder the primary key invites, and it is wrong here.
`cratestack-sqlx` 0.12.0's `query/write/delete_exec.rs` runs

```text
DELETE FROM disabled_clients WHERE client_id = $1 AND (<delete policy>) RETURNING ...
```

and, when `fetch_optional` returns `None`, answers
`CratestackError::Forbidden("delete policy denied this operation")`. There is
no version column on this model, so the one disambiguation that function has
(a stale `If-Match`) does not apply: **it cannot tell "the policy refused you"
from "there was no such row", and it names the first.** `enable_client`'s
contract is the opposite — "a no-op, not an error, if `client_id` was not
disabled to begin with" — so `.delete()` would turn every re-enable of an
already-enabled client into a `Category::Internal` error, and break both
`disabled_client_lookup_reflects_disable_and_enable` and
`find_client_reflects_the_disabled_clients_kill_switch`.

`delete_many` returns `BatchSummary { total: 0, .. }` instead. The count is
dropped rather than asserted on, because `client_id` is the primary key so it
is only ever 0 or 1, and requiring 1 is the same thing as failing on an absent
row.

The price is stated here rather than left to be discovered: `delete_many` puts
its policy clause in the `WHERE` (`push_action_policy_query`), so this is the
one write whose policy mistake is **silent**. See the table below.

#### What a missing policy costs, per action

The three write actions do not behave alike, and the difference is the most
useful thing this adoption has measured. All six rows were produced by
deleting the named line and running the named test against a real Postgres on
2026-09-06.

| Missing `@@allow` | What happens at runtime                                                                                                 | What goes red                                                                                                               |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `read`            | Read returns `Ok(None)` for every client — **kill switch silently OFF**                                                 | `a_disabled_client_reads_the_same_through_both_paths`: `CrateStack says false, sqlx says true`                              |
| `create`          | `disable_client` **errors** on the insert branch: `Forbidden` → `PersistenceError::Denied` → `Category::Internal`       | the write parity test, at the first `disable_client`                                                                        |
| `update`          | `disable_client` **errors** on the _conflict_ branch only — the first disable of a client succeeds and the second fails | the write parity test, at the _second_ `disable_client`; a test that disabled once and stopped would have passed            |
| `delete`          | `enable_client` removes nothing and returns `Ok` — **silent**                                                           | the write parity test's enable assertion, which reads the row back through a plain `SELECT` rather than trusting the return |

The `create` and `update` rows are loud because `upsert_exec.rs` evaluates
`create_allow_policies` in Rust, before it builds any SQL, and
`upsert_resolve.rs::gate_update_policy` does the same for the conflict branch;
an empty allow list is `Ok(false)` there and becomes a `Forbidden`. The `read`
and `delete` rows are silent because those two paths compile the policy into a
`WHERE` clause, where an empty allow list renders `FALSE`
(`query/support/policy.rs::push_allow_policy_query`) and a refused row is
indistinguishable from an absent one.

**Both silent failures are fail-safe in direction**, which is worth stating
but is not why they are acceptable: a missing `read` policy leaves every
client _admitted_ (dangerous, and the reason the read parity test exists), and
a missing `delete` policy leaves a client _revoked_ (safe). The reason both
are acceptable is that a container-backed test makes each red, and neither is
detectable by `just check-schema`, `cargo build`, `just clippy` or any of the
ten `just verify` gates.

**And, since 2026-09-06, by a test that needs no container.** The four
mutations above were re-run by the sabotage review, and every one of them left
`cargo nextest run -p vpay-db --lib` green at 26 passed — the whole
database-free half of the gate was blind to every policy hole.
`ModelDescriptor` publishes one `&'static [ReadPolicy]` per action slot and
`push_allow_policy_query` renders the literal `FALSE` for an empty one, so
"is this slot empty" is the runtime question itself, askable in a unit test:
`disabled_clients::tests::every_action_this_crate_calls_has_an_allow_arm`
asserts all four are occupied and no `@@deny` has appeared, and is red under
each of the four mutations in about 4 ms. It is not a substitute for the
container tests — a non-empty slot does not say the policy admits _this_
caller, which is the thing `auth().isSystem()` has to get right — it removes
the wait to learn that a slot is empty. Any second model this crate reaches
through CrateStack should copy it.

Two other things that follow from the same mechanism and are not obvious:

- **`@@allow("create", ...)` alone is not enough for an upsert.** Both slots
  are consulted, and the `update` one only on the branch that a fresh database
  never takes. Mutation 2 above is what makes that concrete.
- **Four arms rather than one `@@allow("all", ...)`, and not for the reason
  first written here.** `cratestack-macros`' `parse_policy_expression` treats
  `"all"` as matching every action, so one line would compile and work. This
  section claimed until the review of 2026-09-06 that `"all"` "would also
  grant `list` and `detail`, which nothing in vpay calls" — it would not grant
  them _additionally_. `model/descriptor.rs:45-47` compiles a `read` arm into
  **both** `&["list", "read"]` and `&["detail", "read"]`, so the four arms and
  `@@allow("all", …)` populate an identical set of slots. The genuine and
  sufficient reason for four lines is that each is separately droppable, which
  is what makes the four rows of the table above four measurements rather than
  one. `every_action_this_crate_calls_has_an_allow_arm` pins the corrected
  fact: `detail_allow_policies.len() == 1` while `model DisabledClient`
  declares no `detail` arm at all.

### Migration 0032: what `currencies` and `providers` cost

Three statements, and their effects on the drift report are wildly unequal.
All three numbers below were measured with
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` on
2026-09-06, before and after; `docs/plans/exp17-notes/opus.md` has both
reports in full.

| Change                                                                      | Drift effect                        | Why it was made                                                                                                                     |
| --------------------------------------------------------------------------- | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `currencies.exponent` `INT` → `BIGINT`                                      | **-1 change, -1 unmappable column** | `Int` emits `int8` and the introspector refuses to map `int4` back onto it, so the column was excluded from the comparison entirely |
| The two `currencies` CHECKs renamed to `<table>_<column>_<validator>_check` | **0**                               | The names are the half that _can_ converge at 0.12.0; the kinds cannot (below)                                                      |
| `providers.flow` native enum → `TEXT` + `providers_flow_enum_check`         | **0**                               | Nothing to do with drift: CrateStack cannot _decode_ a native enum column                                                           |

#### The rename that buys nothing, and why it is still right

`diff/checks.rs` matches CHECK constraints by name first and compares kinds
second. A hand-named CHECK is therefore proposed for `DROP` however correct
its predicate is, which is what `code_is_iso4217_shape` and
`exponent_in_range` were doing in the report. Renaming them to
`currencies_code_iso4217_check` and `currencies_exponent_range_check` — with
the predicates the generator emits, `code ~ '^[A-Z]{3}$'` byte for byte and
`exponent >= 0 AND exponent <= 4` in place of `BETWEEN 0 AND 4` — fixes the
name half and **moves the count by zero**.

The reason is a deliberate upstream decision, not a bug and not something a
better rename could avoid. Introspection reports _every_ validator-derived
CHECK as `CheckKind::Raw(<deparsed text>)`; it reconstructs only
`CheckKind::Enum`, and never `Iso4217`, `Range` or `Length`. `ir/checks.rs`
says why: "design doc §2.2 notes the compiled SQL for e.g. `@range(0, 150)` is
indistinguishable from hand-written `CHECK (age >= 0 AND age <= 150)` once it
reaches the catalog. Rather than guess which validator (if any) produced it,
introspection always reports it as opaque text." So a matching name turns two
unrelated report lines ("this CHECK is in the database and not the schema"
plus "this one is in the schema and not the database") into a same-named
drop-and-add pair. Clearer report, same number.

It is still right, for two reasons that are about the future rather than the
count. The database now carries the name a generated `migrate diff` would
emit DDL against. And when upstream grows validator reconstruction, the names
will already be in place and the count will fall on its own — whereas leaving
them hand-named would mean doing this rename later, on a table with rows.

**The general lesson for anyone moving the next table:** do not expect a
CHECK rename to move `EXPECTED_DRIFT_CHANGES`. Only a _type_ fix or a whole
table entering the schema does.

`providers.code_length` and `providers.display_name_length` were deliberately
**not** renamed. The count would not have moved for the reason above;
`display_name` carries no `@db_enforce` so its CHECK has no authored
counterpart to converge on at all; and `postgres_smoke.rs` asserts the report
still carries ``CHECK `code_length` ...`` as its proof that the report says
_something_ about `providers` beside the invisible cross-column CHECK.

#### Migration 0033: dropping a default moves no drift, and skipping it moves five

The mirror of the rename finding above, and the reason migration 0033 and the
`schemas/vpay.cstack` edit are one commit. Measured on 2026-09-06 with the same
test:

| Variant                                                                      | Report                                                                       |
| ---------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Five `@default(...)` in the schema, five `DEFAULT`s in the database (before) | **84 / 16 / 17**                                                             |
| Neither (after 0033)                                                         | **84 / 16 / 17**, `providers` block byte-identical                           |
| Schema half only — the five `@default(...)` removed, 0033 absent             | **89 / 16 / 17**, five `column … default value differs` lines on `providers` |

So a `DEFAULT` costs drift only when the two sides disagree, and the general
lesson is the one the CHECK rename taught in the other direction: **do not
expect a DDL change to move `EXPECTED_DRIFT_CHANGES` just because it is real.**
What 0033 buys is not a smaller number; it is that
`Create{Model}Input` can carry the five columns at all.

#### The enum conversion no report can see

`providers.flow` was `provider_flow`, a native Postgres enum. It is `TEXT`
with `CHECK (flow IN ('push', 'redirect'))` now, named
`providers_flow_enum_check` — `naming.rs::check_name(table, column, "enum")`.

The conversion had to happen and has nothing to do with drift. CrateStack
never emits a native enum on any backend: its generated row decoders read an
enum column with `try_get::<String>()` and `.parse()`, so a native enum column
**fails to decode on every read** (`emit/postgres/columns.rs`, upstream issue
#228). Any CrateStack query touching `providers` would have errored.

And the drift report is structurally blind to the whole thing, which is the
finding worth carrying:

- `introspect/postgres/enums.rs` _already synthesised_ an
  `AddCheck { name: "providers_flow_enum_check", kind: Enum { .. } }` out of
  `pg_enum` for the native column, so the CHECK matched before and matches
  after.
- `introspect/postgres/columns.rs::resolve_column` maps a native enum and a
  `TEXT` column onto the _same_ `ColumnType::Scalar("String")`.

So `providers` reports exactly the same four lines before and after. One of
those four, `column flow type differs (live: Scalar("String"), schema:
Enum("ProviderFlow"))`, is **permanent at 0.12.0** and is not a defect in
`schemas/vpay.cstack`: the enum's _name_ has no catalog representation to
recover it from, which `enums.rs`'s own doc comment calls documented
lossiness. Every enum-typed column in the schema carries one such line
(`charges.state`, `charges.failure_code`, `payment_intents.status`,
`ledger_entries.account`, `ledger_entries.direction`), and no migration can
remove them.

This is the mirror image of the multi-column-CHECK blind spot: there, the
report cannot see a constraint that exists; here, it cannot see a conversion
that happened. Both mean the same thing operationally — **a green drift report
is not evidence about either**, and a hand-written test is. For this
conversion that test is
`a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx`
(`vpay-db`'s own `config_reconcile::tests`), and reverting the `ALTER COLUMN
flow TYPE TEXT` is what makes it red.

`providers.flow` is the **first of vpay's seven native enums** to be
converted. The other six — `intent_status`, `charge_state`, `failure_code`,
`account_kind`, `direction`, and `payment_intents.last_payment_error_code` —
are on tables no CrateStack query touches, and each will need the same
treatment before one can.

### `reconcile`: read first, then write, and why that is the guard

`ConfigReconcile::reconcile`'s currency pass is two CrateStack calls where it
used to be one hand-written statement, and the extra round trip is not a
regression to be optimised away later. It is the invariant.

The statement it replaced was:

```text
INSERT INTO currencies (code, exponent) VALUES ($1, $2)
ON CONFLICT (code) DO UPDATE SET exponent = currencies.exponent
RETURNING exponent
```

`SET exponent = currencies.exponent` is a deliberate **no-op** write: it locks
the row and makes `RETURNING` yield the _stored_ value, so the comparison
behind `DbError::CurrencyExponentConflict` could happen in Rust. CrateStack's
`upsert` renders `SET exponent = EXCLUDED.exponent` — measured with
`preview_sql` and pinned by
`the_currency_upsert_would_overwrite_a_stored_exponent_on_its_own` — which is
the _overwrite_ that error exists to refuse. Letting it land would reinterpret
every amount already stored in that currency, silently, with no write to any
of those rows.

So the shape is:

```text
SELECT code, exponent FROM currencies WHERE code = $1 LIMIT 1 FOR UPDATE   -- find_unique(code).for_update()
   -> Some(row) with a different exponent?  CurrencyExponentConflict, transaction rolls back
   -> otherwise
INSERT INTO currencies (code, exponent) ... ON CONFLICT DO UPDATE SET exponent = EXCLUDED.exponent
```

Both `run_in_tx` on the transaction `reconcile` opened, after the same
`pg_advisory_xact_lock`, in the same sorted order. CrateStack joins vpay's
transaction; it never opens one of its own on this path. This is the first use
of `run_in_tx` anywhere in vpay.

**Two guards, and they are not the same guard.** The advisory lock serialises
`reconcile` against every other `reconcile` — that is what
`reconcile_waits_for_the_boot_lock_and_proceeds_once_it_is_released` proves,
and it still passes with `.for_update()` deleted. The row lock binds a writer
that does _not_ go through this function. Measured on 2026-09-06: with
`.for_update()` deleted, **every test in the repository stayed green**, which
is why this change adds
`reconcile_reads_the_exponent_under_a_row_lock_and_cannot_clobber_a_concurrent_writer`
— an outside transaction updates the row uncommitted, `reconcile` blocks on
the read, and the assertion is that the outside writer's value _survives_.
Without the row lock the plain `SELECT` returns the pre-commit value, the
comparison passes, and the upsert writes over a committed change. See
`docs/plans/exp17-notes/opus.md` § 2.

**Two pooled connections on the conflict branch**, as with `disable_client`:
`upsert_resolve.rs::gate_update_policy` runs its policy probe on
`runtime.pool()` while this transaction holds a connection of its own. It does
**not** deadlock against the row this transaction just locked, and that is
worth stating explicitly because it is the obvious worry:
`row_passes_update_policy` emits a plain `SELECT 1 ... AND (<policy>)` with no
`FOR UPDATE`, and Postgres's MVCC reads are not blocked by writers. Two of
`MAX_CONNECTIONS = 10` per in-flight reconcile, on a boot step the advisory
lock already admits one at a time.

### `providers` is written through CrateStack (D7, resolved 2026-09-06)

`reconcile`'s **provider** pass is
`upsert(CreateProviderInput).run_in_tx(tx, ctx)`. It was a hand-written
`INSERT ... ON CONFLICT` for one day, and the reason it was is the reason
migration 0033 exists, so both are recorded here rather than only the outcome.

**The blocker.** `cratestack-macros`' `model/inputs.rs::create_input_fields`
and `model/descriptor/columns.rs`'s `upsert_update_columns` both drop every
field carrying a `@default(...)`, and `model Provider` carried one on all five
capability booleans because the live table did (migration 0002: `DEFAULT
FALSE` ×4, `DEFAULT TRUE`). Measured with `preview_sql` on 2026-09-06:

```text
INSERT INTO providers (code, display_name, flow) VALUES ($1, $2, $3)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name, flow = EXCLUDED.flow
```

`supports_refunds`, `supports_partial_refunds`, `delivers_callbacks`,
`requires_ip_allowlist` and `enabled` were in neither list. Boot step 4 would
have inserted every rail with the column defaults regardless of what the
deployment configured, and would never have carried a capability change to an
existing row, so a rail an operator had just disabled would come back enabled.

**The decision.** Two ways out were measured, and the maintainer's delegate
took the first: remove the five `@default(...)` from `model Provider` **and**
`ALTER TABLE providers ALTER COLUMN … DROP DEFAULT` on all five. The argument
is not that a code generator should get to shape vpay's DDL — it is that the
default was never doing anything for the only writer there is. `reconcile` is
the sole writer of those five columns and always writes all five from
configuration; a default cannot help it. What a default _can_ do is invent a
capability for some other writer that forgot one, and a rail silently recorded
as "does not refund", or silently recorded as enabled, is worse than an
`INSERT` that refuses. The second way out — upstream growing a way to include
a defaulted column in an upsert input — remains open (re-checked at 0.12.0
on 2026-09-07: `create_input_fields` still filters every `@default(...)`
field) and would now be a simplification rather than an unblocking.

**The two halves are one commit, and that is measured.** With the five
`@default(...)` removed from the schema and the DDL untouched, the drift report
goes **84 → 89**: exactly five `column … default value differs` lines on
`providers`. With both halves done it stays at **84 / 16 relations / 17
unmappable**, and the `providers` block is byte-identical to what it was
before — the two sides agreed when both had the defaults and they agree now
that neither does. Dropping a default moves no drift by itself.

```text
INSERT INTO providers (code, display_name, flow, supports_refunds,
    supports_partial_refunds, delivers_callbacks, requires_ip_allowlist, enabled)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (code) DO UPDATE SET display_name = EXCLUDED.display_name,
    flow = EXCLUDED.flow, supports_refunds = EXCLUDED.supports_refunds, … ,
    enabled = EXCLUDED.enabled
```

is what renders now, pinned by
`the_provider_upsert_carries_all_eight_columns`.

**Two things notice a regression, and the compiler is the first.** The five
columns are `CreateProviderInput` _fields_, not merely SQL: restore a
`@default(...)` in `schemas/vpay.cstack` and `vpay-db` stops building with
`error[E0560]: struct `inputs::CreateProviderInput`has no field named`supports_refunds`` at `reconcile`'s struct literal (measured). Delete the
migration instead and the failure is at test time, three ways: the drift test
at 89 vs 84, the migration-count test at 32 vs 33, and
`a_hand_written_provider_insert_must_now_name_every_capability_column`, which
finds the three-column `INSERT` accepted again.

**What the drop costs an operator.** All five columns are `NOT NULL` with no
default, so a hand-written `INSERT INTO providers (code, display_name, flow)`
is a `23502` rather than a row with invented capabilities. That is the whole
cost, it is a refusal rather than a silent difference, and it is asserted in
both directions by the test named above. Three fixtures in `postgres_smoke.rs`
had to name the columns in the same commit.

**No `find_unique(...).for_update()` ahead of the provider upsert.** This is
the asymmetry with the currency pass and it is deliberate. The currency read is
the _guard_: `upsert` renders `SET exponent = EXCLUDED.exponent`, a stored
exponent must never be overwritten, so the value has to be read under a row
lock and compared before the write. A provider has no such value — every one of
the eight columns is owned by configuration and overwriting it is the point, so
the read would return a row nothing compares. The row lock itself is taken
anyway, in the same transaction, by `upsert`'s own conflict probe
(`upsert_exec.rs::run_upsert_in_tx` calls
`select_for_update_by_conflict_target` on `tx`). One measured consequence is
worth writing down because it inverts the currency finding: with no
`providers` row lock held when the upsert runs, swapping `run_in_tx` for `run`
**fails in 1.2 s** rather than hanging.
`a_provider_written_through_cratestack_is_rolled_back_with_the_rest_of_the_transaction`
is the assertion that fails; the currency equivalent had to be written
specifically because its mutation produced `SLOW [>480.000s]` and no error at
all.

What the conflict probe needs in exchange is **READ COMMITTED**, and that is a
real constraint rather than an incidental one: the transaction has to see a
`providers` row another writer committed while it was waiting on
`lock_keys::CONFIG_RECONCILE`. `pool.begin()` opens Postgres's default and
that is what makes this work.
`a_reconcile_that_waited_for_the_boot_lock_overwrites_what_the_holder_committed`
(`vpay-db/tests/repositories.rs`, added in the review pass on 2026-09-06) is
the only test that says so: under `SET TRANSACTION ISOLATION LEVEL REPEATABLE
READ` the snapshot is taken when the advisory-lock statement starts, before
the holder commits, the probe cannot see the row, and boot fails with `40001`
— measured, with every other reconcile case still passing.

**A classification changed on purpose.** `CreateProviderInput::flow` is the
schema's `ProviderFlow` enum, so `reconcile` parses `ProviderSeed::flow` — a
`String`, and left one, because Step 2's D4 says `vpay-db` binds strings —
before it builds a statement. An unparseable label is
`DbError::ProviderFlowUnknown`, `Category::Configuration`, exit `78`. It used
to reach `providers_flow_enum_check` and come back as `DbError::Query` →
`Category::Storage` → exit `69`, which told a supervisor to wait for a
database that was working perfectly. The CHECK is untouched and still refuses
a writer that is not `reconcile`. The parse is a `match` rather than
`unwrap_or_default()`, because `cratestack-macros` derives `Default` on every
generated enum with the _first_ variant as the default
(`types/enums.rs::variant_tokens`) and the first variant of `ProviderFlow` is
`push` — so `unwrap_or_default()` would have stored a typo'd rail as a push
rail and returned `Ok`.

**`model Provider` gained `create` and `update` arms**, in the commit that
moved the write and not before it, and has no `delete` arm: `reconcile`
disables a dropped rail and never removes one, and every `charges` and
`provider_requests` row references this table. Deleting the `update` arm is
loud but only from the _second_ boot — the first boot of a fresh database
takes the insert branch — which is why
`every_action_this_module_calls_has_an_allow_arm` asserts the slot without a
database.

### The money tables: what moved, and what stays raw forever

S5 (2026-09-07) is the last rung of the CrateStack ladder and the one the
whole adoption was pointed at: `payment_intents`, `charges`, `refunds` and
`checkout_sessions`. It landed **migration 0037**, four models, and **four
queries** — all four on `checkout_sessions`, and none at all on the other
three tables. That asymmetry is the finding, so it is stated before anything
else.

#### One blocker decides three tables, and it is `jsonb`

Every read and every write in `vpay_db::payment_intents`, `vpay_db::charges`
and `vpay_db::refunds` returns a row struct — `PaymentIntentRow`,
`ChargeRow`, `RefundRow` — and each of those carries a `jsonb` column:

| Table             | Column(s)                          | What is in them                                                       |
| ----------------- | ---------------------------------- | --------------------------------------------------------------------- |
| `payment_intents` | `payment_method_types`, `metadata` | a JSON array of rail codes, and **merchant-authored** key/value pairs |
| `charges`         | `provider_ref_extra`               | rail key material (Orange's `pay_token`)                              |
| `refunds`         | `metadata`                         | merchant-authored, as above                                           |

`Value::from_plain_json` routes every JSON number through `Number::as_i64()`
with an `as_f64().unwrap_or_default()` fallback
(`cratestack-core-0.12.0/src/value.rs:95-106`), so a `u64` above `i64::MAX`
anywhere inside a merchant's `metadata` comes back as a float. That is the
same argument `events.data` has carried since 2026-09-06, and on `metadata`
it is stronger rather than weaker: `events.data` is at least vpay-shaped
around a merchant payload, while `metadata` **is** the merchant payload.

So the models do not declare those columns, and a generated read that omitted
them would answer a struct the repository cannot build. The two ways around
that were both considered and both rejected:

- **A second statement for the `jsonb` columns.** Two round trips and a
  consistency window, on the money path, to replace one statement that works.
- **Declaring `metadata Json`.** Strictly worse for drift, in the direction
  the report cannot help with: an _undeclared_ `jsonb` column is invisible to
  the comparison in both directions, so it costs nothing; a _declared_ one
  adds a `[blocking] column metadata is declared in the schema but does not
exist in the live database` line while leaving the live column just as
  invisible. `EXPECTED_UNMAPPABLE_COLUMNS` did not move at all across S5,
  which is that prediction tested.

**Nothing on those three tables moved, and nothing will until upstream maps
`jsonb` in both directions.** The three models carry no `@@allow` arm at all
— deny-by-default — because an arm for a call site that does not exist is a
standing permission nobody asked for.

`refunds` has a second, independent blocker worth recording because the brief
that produced this work assumed otherwise: **there is no refund create to
move.** `vpay_db::refunds` is two reads and no write, because
`ProviderAdapter::refund` is `NotImplemented` on MTN and `Unsupported` on
Orange (`docs/status.md`). Both reads are also merchant-scoped through a JOIN
onto `payment_intents` — the table carries no `merchant_id` of its own, and
migration 0017 argues why it should not — and a generated read filters
columns of one table.

#### `checkout_sessions` moved because migration 0028 gave it nothing to trip on

No `jsonb`, no `bytea`, no native enum. Four of its nine repository methods
are now `find_unique`/`find_many` calls, and `EXPECTED_ASSERT_SITES` in
`sql_audit.rs` falls 45 → 41 for the first time in its history. The other five
did not move, and each has its own reason:

| Statement                                                     | Why it stays raw                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `list_page`                                                   | the cursor is a correlated sub-select (`seq < (SELECT seq FROM checkout_sessions WHERE id = $2 AND merchant_id = $1)`); no delegate expresses it, exactly as for `Events::list_page`                                                                              |
| `expire`, `due_for_expiry`, `expire_due`, `settle_for_intent` | each carries `NOT EXISTS (SELECT 1 FROM charges …)` over a **second** table, and a generated builder filters columns of one. That guard is the statement's whole point: it is what stops a session being expired out from under a payer who has already confirmed |
| `create`                                                      | expressible, and deliberately deferred — see below                                                                                                                                                                                                                |

**`create` is the one that could have moved and did not.** Moving it needs
migration 0037 to `DROP DEFAULT` on `created_at` and `updated_at`, because
`cratestack-macros` filters every `@default(...)` field out of
`Create{Model}Input` (`model/inputs.rs:20-23`) and the writer supplies
`created_at` today. That is migration 0033's trade applied to a money table,
and 0033's own argument does not carry over cleanly: on `providers` the
defaults were doing nothing for the only writer there was, whereas here
`updated_at`'s default is what `create` relies on. It is one migration and
one struct field away, and it is written down rather than done, because a
column default dropped on a money table is a `23502` waiting for the writer
that forgets. `model CheckoutSession` has no `@@allow("create", …)` arm, and
`every_action_this_module_calls_has_an_allow_arm` asserts that slot is
**empty** — so adding one is a deliberate edit rather than a silent one.

**One of the four reads is pinned on its rendered SQL rather than on its
behaviour, and the reason is a measurement.** `find_latest_by_intent` asks for
`ORDER BY seq DESC LIMIT 1`. Deleting that `order_by` left a container test
that seeds two sessions and asserts _which_ one comes back **green**: the live
`checkout_sessions_intent_seq_idx` is `(payment_intent_id, seq DESC)`, so
Postgres answers an unordered `LIMIT 1` out of that index in seq-descending
order anyway. The behaviour was right by accident — dependent on a planner
choice, on that index continuing to exist, and on the row count staying small.
The chain therefore lives in `latest_by_intent_query` so that
`the_latest_session_query_orders_by_seq_and_takes_one` can preview **the same
builder the query runs**; a test that rebuilt the chain itself would assert
the generator back to itself and catch nothing.

#### Migration 0037, and the two things it deliberately did not do

It converts the last four native enums on a money table — `intent_status`,
`charge_state`, `refund_status`, and the `failure_code` shared by
`charges.failure_code`, `payment_intents.last_payment_error_code` and
`refunds.failure_code` — to `TEXT` plus a `<table>_<column>_enum_check`.
`account_kind` and `direction`, on the ledger tables, are the two that remain.

Three measurements from writing it, none of them guessable from the source:

1. **A partial index blocks the `ALTER` outright.** `charges_live_idx` is
   `ON charges (state) WHERE state IN (…four labels…)`, and
   `ALTER COLUMN state TYPE TEXT USING state::TEXT` fails with
   `ERROR: operator does not exist: text = charge_state`. The migration drops
   and rebuilds it. `charges_state_idx`, a plain index on the same column, is
   rebuilt by Postgres on its own and needs nothing.
2. **The drift count moved by zero**: 130 before, 130 after, measured against
   the same freshly migrated database with only the migration applied. That is
   what "The enum conversion no report can see" predicts, and this is the
   second table set to confirm it. `introspect/postgres/enums.rs` had already
   been synthesising an `AddCheck { name:
"payment_intents_last_payment_error_code_enum_check", kind: Enum }` out of
   `pg_enum` for the native column — the _same name_ migration 0037 then
   created for real, which is visible in the before-report as a `[safe] CHECK
… exists in the live database but is not declared` line on a database where
   no such constraint existed.
3. **A nullable enum column takes a bare `IN (...)`, not `IS NULL OR … IN
(...)`.** `NULL IN (…)` is NULL and a CHECK fails only on FALSE, so the
   NULL case needs no spelling out — and spelling it out would deparse to
   something `reconstruct_enum` cannot read back (it matches
   `<column> = ANY (ARRAY[…])` and nothing else), turning a `CheckKind::Enum`
   into a `CheckKind::Raw` and a converged constraint into a kind mismatch.
   The generator relies on the same fact and says so
   (`emit/postgres/checks.rs`: "NULL passes … Nullable enum columns therefore
   need no special casing").

What it did **not** do, both deliberate:

- **It drops no column default.** 0033's argument applies to a column a
  CrateStack write must name, and no write on these four tables moved.
  `PaymentIntents::insert` does not name `amount_received`, `amount_refunded`,
  `amount_refund_pending` or `updated_at`; dropping those defaults would turn
  every intent creation into a `23502` and buy nothing. The models declare the
  defaults instead, so the two sides agree and the columns cost no drift.
- **It renames no hand-written CHECK.** `checkout_sessions`' three closed
  vocabularies were never native enums — migration 0028 created them as `TEXT`
  plus `ui_mode_is_known`, `status_is_known` and `payment_status_is_known` —
  and a rename moves the count by zero (0032's measurement). The model
  declares those three columns as plain `String` rather than `.cstack` enums,
  so there is no generated name to converge on, and `vpay-db` carries all
  three as `String` regardless.

#### One behaviour changed, and it is not visible in any signature

`PaymentIntents::transition` and `Settlement::set_live_state` both take an
`expected` label. It used to be bound as `$3::intent_status` /
`$2::charge_state`, so a label outside the vocabulary was a Postgres error and
reached a caller as a `500`. It is a plain `TEXT` comparison now, so such a
label matches no row and answers `Ok(None)` / `Ok(false)` — the same answer a
genuine lost race gives, which a caller turns into a `409`.

Every caller passes a label from `vpay_core`'s state machine, so the
difference is between two spellings of "a vpay bug". It is recorded because a
`409` is quieter than a `500` and nothing else in the system would say so. The
`new` label is unaffected: outside the vocabulary it is a `23514` rather than a
`22P02`, and `classify_write` leaves both as `DbError::Query`.

#### What the four models cost, and the ten report lines that are false

Drift moves **130 → 141**, per table: `payment_intents` 31 → 19, `charges`
23 → 16, `checkout_sessions` 1 → 21, `refunds` 1 → 11, and nothing else by a
line. `EXPECTED_DRIFTED_RELATIONS` does not move and neither does
`EXPECTED_UNMAPPABLE_COLUMNS`.

The two falls are the report describing rot rather than drift: `model
PaymentIntent` still declared `last_payment_error`, a column migration 0014
**dropped**, and neither it nor `model Charge` knew about `seq`,
`description`, `customer_id`, `client_secret_suffix`, `provider_txn_id`,
`return_url` or `updated_at`. After the rewrite, **not one `column … is
declared in the schema but does not exist` and not one `column … exists in the
live database but is not declared` line survives on any of the four tables.**

The two rises are `customers`' lesson repeated: a column's marginal drift
depends on whether its table is modelled at all, and a table nobody declares
collapses to one line however wrong it is.

The 67 lines the four tables carry are five kinds, and every one is a kind
0.12.0 structurally cannot close — 37 hand-named CHECKs, 12 undeclared
indexes, 6 permanent enum `type differs`, 2 identity-column `seq` defaults,
and **10 foreign keys that the report says do not exist and that all do**.

That last group is the fifth upstream gap in the table at the top of this
section, and it is documented by the tool rather than inferred from its
output: `introspect/postgres/mod.rs`'s own "Known gaps" says "**Foreign keys
are not introspected.** … so `TableProjection::foreign_keys` is always empty
here. A table with `.cstack`-declared relations will show every foreign key as
'missing' drift until a follow-up phase adds this."

The relations are declared anyway. All ten constraints exist,
`cratestack-parser` requires both sides of a relation to be declared, and
removing a true declaration to make a false report line disappear would be
optimising `EXPECTED_DRIFT_CHANGES` instead of the schema — the exact move
that constant's own assertion message warns about. What it leaves is the
mirror of the undeclared-CHECK hazard recorded above: a generated
`migrate diff` would emit `ADD CONSTRAINT … FOREIGN KEY` for ten constraints
that already exist, and Postgres would refuse each one. Nothing runs
`migrate diff` in this repository, so it is latent rather than live.

#### Three tables are out of scope forever unless a reason appears

`jobs`, `idempotency_keys` and `provider_requests` keep working raw `sqlx`
implementations and are **not** pending work. `docs/status.md` says so in the
same words, so that neither list reads as a gap:

- **`jobs`** — `Jobs::claim` needs `FOR UPDATE SKIP LOCKED` and
  `FindMany::for_update()` emits a bare `FOR UPDATE`. A lease that silently
  lost `SKIP LOCKED` would turn every worker's claim into a queue behind every
  other worker's. `jobs.payload` is `jsonb` and `jobs.attempts` is `int4`
  besides.
- **`idempotency_keys`** — the merchant-scoped `claim_id` semantics have no
  delegate equivalent, and three of its columns (`request_hash` `bytea`,
  `response_status` `int2`, `response_body` `jsonb`) are unmapped types.
- **`provider_requests`** — `latest_submit_attempt` orders `sent_at DESC, id
DESC` and pairs `status_code`/`responded_at`; both `attempt` and
  `status_code` are `int4`.

### Why the generated module is private, and what keeps it that way

`include_server_schema!` expands to `pub mod cratestack_schema { … }` **at the
invocation site**. Put it in `lib.rs` and every generated model struct, every
query delegate and a whole generated `pub mod axum` become `vpay_db::…` — a
wholesale reversal of ADR-0016 standard 5 in a one-line diff. So the
invocation lives in `src/schema.rs`, declared `mod schema;`, and nothing is
re-exported from it.

That could not be left to review, because the module the macro creates does
not exist in any source file: no lint, no rustdoc pass and neither of
`verify-repositories`' two original signals can see a
`pub use schema::cratestack_schema;`. **Measured on 2026-09-06: with the macro
in place, both `pub mod schema;` and `pub use schema::cratestack_schema;` made
`cargo xtask verify-repositories` print `ok`.** The gate now carries a third
check that fails both, and `DB_HANDLE_TYPES` has learned `Cratestack` and
`SqlxRuntime` so that a future store holding the runtime instead of a `PgPool`
is recognised as an implementation.

### What stays raw sqlx, and why

Everything else. That sentence used to begin "Everything else, **including
the two `disabled_clients` writes**"; those moved on 2026-09-06, and the
`currencies` and `providers` upserts followed the same day, so the line was
~~three tables wide~~ **— twelve tables wide as of 2026-09-10, the count this
clause stopped tracking after S4b, S5 and ADR-0017; the registry above is the
list** — plus one statement on a table that has otherwise moved.
That statement is `reconcile`'s disable pass, `UPDATE providers SET enabled =
false WHERE code <> ALL($1) AND enabled`: it addresses rows by their
_absence_ from a list, which no generated builder expresses, and it is the
statement that makes "configuration is the authority" true for a rail the
deployment dropped. Three properties of 0.12.0 decide where it
falls, and none of them is a matter of taste:

- **Model policies are compiled into the SQL.** `@@allow`/`@@deny` become
  predicates in the `WHERE` clause of every generated read and write, and a
  model with no `@@allow` is deny-by-default. A policy mistake therefore does
  not raise an error — it returns zero rows. On the kill-switch that means
  "no client is disabled", silently. `model DisabledClient` carries
  `@@allow("read", auth().isSystem())` and nothing else, and the parity test
  above is what makes a missing clause red.
- **Multi-column CHECK constraints are invisible to the migration tooling.**
  `cratestack-migrate` introspects only single-column CHECKs
  (`AND array_length(c.conkey, 1) = 1`), and the grammar has no
  `@@check(expr)` at all. vpay's ten cross-column CHECKs — `no_over_refund`,
  `partial_refunds_imply_refunds`, `lock_is_paired` and the rest — cannot be
  expressed here and would not be noticed missing. `backends/migrations/*.sql`
  stays the authoritative schema; the `.cstack` file is compared against it,
  never generated from it.
- **Six Postgres types are mapped, and vpay uses more than six.** `int4`,
  `int2`, `numeric`, `jsonb`, `bytea` and arrays are not in `map_scalar`, so
  seventeen live columns are excluded from the drift comparison outright — the
  number `EXPECTED_UNMAPPABLE_COLUMNS` pins in `postgres_smoke.rs`. Since
  2026-09-06 four of those seventeen sit inside tables the schema models
  _fully_ (`events.data`, `events.fanout_attempts`,
  `webhook_deliveries.attempt`, `webhook_deliveries.status_code`), which is a
  sharper version of the same point: unmeasured drift can hide inside a table
  the report otherwise compares column by column, and only that constant says
  so.
- **`FOR UPDATE SKIP LOCKED` has no delegate.** `FindMany::for_update()`
  emits a bare `FOR UPDATE`, so `Jobs::claim` — and therefore the whole
  `jobs` table — stays hand-written. A lease mechanism that silently lost
  `SKIP LOCKED` would turn every worker's claim into a queue behind every
  other worker's. `jobs.payload` is `jsonb` and `jobs.attempts` is `int4`
  besides.

A fourth reason used to be recorded here — that moving a read and a write
together produces a parity test that cannot say which of the two it is
testing. That was an argument for _sequencing_, not for staying, and it was
honoured: the read landed on 2026-09-06 and the writes a change later, with
the read's test re-seeded from an inline `INSERT` so it kept testing the read.
See "What runs through it today" above.

### What compiling the schema does not prove

`cargo build` now parses and type-checks `schemas/vpay.cstack`, and
`just check-schema` runs the CLI over it. Neither of them knows anything about
the database, and it is worth writing down exactly how little that leaves,
because "it compiles" reads like more than it is. Both measured on
2026-09-06 (`docs/plans/exp14-notes/opus-review.md`, M7 and M8):

- **A modelled column may name a type the live table cannot produce.** Change
  `reason String?` to `reason Json?` on `DisabledClient` — the column is
  `TEXT` — and `just check-schema` says `schema OK`, `cargo build` succeeds,
  and `just clippy` and all ten `just verify` gates stay green. The failure
  arrives as an `sqlx` decode error at the first read, i.e. in production, or
  in the container-backed drift test and the parity test, which are the only
  two things that would have caught it.
- **A missing `@@allow` is invisible to every one of them.** Delete any of
  the four on `DisabledClient` and the same list stays green — including the
  three added on 2026-09-06 for the writes, re-measured then. That is true
  even for the two whose absence _does_ raise an error at runtime
  (`create`, `update`): the check happens in `cratestack-sqlx` at query time,
  not at macro-expansion time, so nothing a compiler or the CLI can see is
  different.

So the guard on all of them is container-backed by construction. Adding a
`model` to this schema is not free, and the drift test and the two parity
tests are what make it safe — not the compiler.

The transaction seam is untouched, and can stay untouched: every CrateStack
builder exposes `run_in_tx(&mut sqlx::Transaction<'_, Postgres>, &ctx)`, so a
write that needs to join vpay's own transaction can, and neither of the two
that moved on 2026-09-06 does — `disable_client` and `enable_client` are each
one statement with nothing to be atomic with, so both call `.run(ctx)` and let
CrateStack own whatever transaction it wants underneath (the upsert opens
one; `delete_many` opens one). `UnitOfWork::transaction` and
`TxOutcome::Abandon` survive — `UnitOfWork::transaction` and `TxOutcome::Abandon` survive —
CrateStack's own `transaction` combinator commits on `Ok` and rolls back on
`Err` with no third ending, and `Abandon` is what the confirm path's
duplicate-charge `409` is built on. The first write that _does_ need to be in
a vpay transaction is where `run_in_tx` gets exercised; nothing exercises it
today. This is also why the root `Cargo.toml`
pins `sqlx = "=0.9.0"` exactly rather than `"0.9"`: `run_in_tx` only accepts
vpay's transaction while both halves resolve the same `sqlx-core`.

### Errors: why vpay classifies them itself

`PersistenceError` (`src/persistence.rs`) is the leaf, `DbError::Persistence`
`#[from]`s it and delegates, and `classify_cratestack` is the single place a
`CratestackError` is read — the mirror of `error::classify_write`, branching
on the same SQLSTATEs and carrying the same constraint names.

CrateStack's own `status_code()`/`public_message()` are **not used anywhere**,
and that is load-bearing rather than stylistic: its `DatabaseTyped` variant is
a `500` with the canned message `"internal error"`, so a duplicate charge
would reach a merchant as an outage. Here a `23505` is a `409` with
`error.code = "resource_conflict"` and `Retry::Never`, exactly as the sqlx
path already made it —
`a_duplicate_key_classifies_the_same_through_cratestack_as_through_sqlx`
asserts the two agree _and_ that neither matches what CrateStack would have
said.

Two honest limits on that mapping:

- **A read never carries a SQLSTATE.** `FindUnique::run` maps its
  `sqlx::Error` with `CratestackError::Database(error.to_string())` rather
  than through `cratestack_error_from_sqlx`, so today every failure of the one
  query vpay runs lands on `PersistenceError::Backend` → `Category::Storage`.
  That is the same answer `DbError::Query` gave for the `SELECT` it replaced,
  so nothing regressed; the SQLSTATE arms exist for the writes that have not
  moved yet and are unit-tested rather than exercised.
- **`Denied` cannot be produced by the read path either.** A policy refusal on
  a read is a `WHERE` clause, not an error; `CratestackError::Forbidden` comes
  only from the write and batch paths. It classifies `Internal`, not
  `Forbidden`: every CrateStack call in this crate runs as the system
  principal, so a refusal means the schema and the call site disagree — a
  deploy bug that pages, never something to tell a merchant they are not
  allowed to do.

  **`Denied` stopped being unreachable on 2026-09-06**, when `disable_client`
  became an upsert: that path evaluates its policies in Rust and raises a real
  `Forbidden`, so the variant is now exercised by a container-backed mutation
  and not only by `persistence.rs`'s unit test. `enable_client` still cannot
  produce it — `delete_many`'s policy is a `WHERE` clause like the read's.
  The `Unique`, `ForeignKey` and `Check` arms remain unit-tested rather than
  exercised: `disabled_clients` has no foreign key and no CHECK, and the one
  unique constraint it has is the primary key the upsert exists to absorb.

### The context, and the policy it satisfies

`system_context()` returns `SystemContext::for_service("vpay-db")`'s inner
context. `SystemContext` is the only producer of a context that satisfies
`auth().isSystem()`, it has no `From<CratestackContext>` and no constructor
taking one, and `CratestackContext::system` is `#[serde(skip)]` — so the
marker cannot arrive over a wire and no request-derived context can become
one. That asymmetry is why the schema names `auth().isSystem()` rather than
`auth() != null`: the weaker predicate would be satisfied by any authenticated
caller if this table ever were reached on behalf of a request.

The service name reaches `actor.id = "system:vpay-db"` in
`cratestack_sqlx::audit::actor_from_context`. Nothing in vpay audits through
CrateStack yet — `model DisabledClient` carries no `@@audit`, so
`descriptor.audit_enabled` is `false` and neither write calls
`ensure_audit_table`; the same is true of `@@subscribe` and the event outbox,
so no generated write in this crate creates a table. The name is fixed now so
it does not have to be chosen retroactively for rows already written, and
2026-09-06 is when it first attributed one: until the writes moved, every
CrateStack call in this crate was a read.

### What this added to the dependency graph

`Cargo.lock` goes from 469 packages to 497 (+28), of which twelve are
`cratestack-*` and all twelve are MIT. `syn` moves 3.0.3 → 3.0.5. The feature
set is `default-features = false, features = ["postgres"]`, which drops
`decimal-rust-decimal` (this schema declares no `Decimal`; vpay's money is
integer minor units) and `codec-json` (vpay generates no client).

Two of the twenty-eight are **BlueOak-1.0.0** — `minicbor` and
`minicbor-serde` — and they are not optional. `cratestack-pg` declares
`cratestack-axum` and `cratestack-client-rust` as non-optional dependencies
with no feature gating either, and both take `minicbor` unconditionally;
dropping down to `cratestack-sqlx` + `cratestack-macros` directly does not
help, because `include_server_schema!` emits
`pub mod axum { use ::cratestack::HttpTransport; … }` unconditionally.
`deny.toml` therefore carries a **scoped** exception for those two crate names
— not an entry on the `allow` list — with the maintainer's decision and the
reasoning beside it. `cargo deny check` reports
`advisories ok, bans ok, licenses ok, sources ok`, and no other new licence or
ban appeared. `cargo tree -i aws-lc-rs` is still empty and there is still
exactly one `sqlx`.

The generated `pub mod axum` compiles and is never referenced: vpay keeps its
own router and its Stripe-shaped `/v1`. `crypto-aws-lc-rs` must never be
enabled — `deny.toml` bans `aws-lc-rs` (ADR-0005, ADR-0007), and at 0.12.0 the
feature is a `compile_error!` rather than a working mode in any case.

One ergonomic consequence, recorded because the failure message does not
explain itself: `UnitOfWork::transaction`'s `E: From<DbError>` bound used to
have exactly one candidate (the reflexive `impl<T> From<T> for T`), so `E`
fell out of inference. `DbError::Persistence(#[from] PersistenceError)` adds a
second, and a call site that named neither now needs
`-> TxFuture<'_, Result<TxOutcome<T>, DbError>>` on the closure. One site in
this workspace was affected (`repository::closure_shape`); the annotation is
there with a comment.

## Migration 0038: `rate_limit_windows`, and the one model with no `@@allow`

### What runs the statement, added 2026-09-10 by the exp36 review

`count_attempt` is one statement, and three of the things it decides are
decided **inside** it — whether the window has elapsed, what the answer resets
to, and which rows the sweep takes. As delivered, nothing exercised any of
them against a database. `vpay_api::staff::rate_limit`'s unit tests cover
`Verdict::of`'s arithmetic over an integer the statement hands back, the
statement's own test asserts the _text_ of six fragments, and every case over
a booted server runs inside one 300-second window. **A `CASE` that never reset
would have passed all of them** — and its symptom in production is a staff
member locked out of the dashboard for good by ten wrong passwords, which is
the durable lockout ADR-0017 refuses by name.

Three cases in `vpay-db/tests/repositories.rs` now run it, each with a
measured mutation:

| Case                                                                   | Mutation                                          | As mutated                                                                                                     |
| ---------------------------------------------------------------------- | ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `an_elapsed_rate_limit_window_is_replaced_rather_than_extended`        | `attempts = attempts + 1`, dropping the reset arm | reads `5` where it demands `1`                                                                                 |
| `the_rate_limit_table_grows_by_one_window_and_is_then_swept`           | the sweep takes nothing                           | 1040 rows where it demands ≤ 60                                                                                |
| `the_rate_limit_id_is_the_budget_and_the_scope_column_is_only_a_label` | —                                                 | pins that `scope` separates nothing, so a caller that stopped hashing it into the `id` would merge two budgets |

### The bound, stated as it actually is

Migration 0038 and this module both say a caller spending fresh keys "drains
the table faster than they fill it". That is true **in the limit** and not
true inside one window: nothing has elapsed yet, so there is nothing to sweep.
Measured — 1000 fresh keys inside one window leave **1000 rows**, and 40
attempts past the boundary take the table back down.

So the honest bound is **two rows per attempt for the width of one window,
then flat**: at _r_ attempts per second and a 300-second window, roughly
`2 × r × 300` rows at steady state. It is a bound, which is what the objection
ADR-0017 raised needed answering; it is not "the table never grows".

**172 changes over 24 relations, unmappable 19** (from 167 / 23 / 19). The +5
is four hand-named CHECKs and one index, and **not one column-level line** —
no `column … type differs`, no `column … default value differs`, no undeclared
column. `model RateLimitWindow` declares all five columns and the table was
shaped so it could: `TEXT`, `TIMESTAMPTZ` and `BIGINT` only, no `jsonb`, no
`bytea`, no `int4`, no native enum, no `DEFAULT`.

That is the floor a modelled table can reach in this schema at 0.12.0, and it
is worth stating why the two remaining classes are permanent rather than
outstanding: the grammar has no declaration for a hand-named `CHECK` and none
for an `INDEX`, so each is invisible to the schema in one direction and
visible to the live introspection in the other. `attempts` is `BIGINT` and not
`INT` for exactly the reason `currencies.exponent` had to be widened by
migration 0032 after the fact — `int4` is one of the types `map_scalar`
declines, and a column it declines is excluded from the comparison outright.
`EXPECTED_UNMAPPABLE_COLUMNS` is unmoved at 19, which is the assertion that
makes the +5 mean what it says.

### Why the model carries no `@@allow` arm

Every other modelled table in this file has four arms, or (for the money
tables) none because nothing queries them. This one has none for a third
reason, and it is narrower: **the statement has no builder, not the row.**

```text
INSERT INTO rate_limit_windows (…) VALUES (…)
ON CONFLICT (id) DO UPDATE SET attempts = … + 1 …
RETURNING attempts
```

A CrateStack 0.12.0 `UpdateRateLimitWindowInput` carries **values**;
`attempts = attempts + 1` is an **expression over the row's own column**.
There is no generated builder that renders it, and the alternative —
`find_unique`, decide, `update_many` — is precisely the read-then-write race
this table exists to remove: two replicas reading 9 both write 10 and both
admit.

So `vpay_db::rate_limits` holds one hand-written `sqlx` statement, a
`const &'static str` with no `format!` and therefore nothing for
`crate::sql_audit` to audit, and
`the_rate_limit_window_model_answers_no_rows_to_every_action` pins the arms'
absence in the direction that matters. An arm appearing here would be a
standing permission over a table with **no caller at all** — and this is the
one table in the schema whose keys an unauthenticated caller chooses.

### The three properties of the statement, and what each one stops

They are asserted by name in
`the_statement_keeps_the_three_properties_that_make_it_safe`, because none of
them is obvious from the text and each is a plausible tidy-up:

| Fragment                 | Deleting it                                                                                                                               |
| ------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `id <> $1`               | lets the in-statement sweep name the row the `INSERT` is about, through one snapshot                                                      |
| `FOR UPDATE SKIP LOCKED` | lets two concurrent sign-in attempts take the same row locks in different orders — a deadlock, and a `500` for somebody typing a password |
| `LIMIT $5`               | makes the first attempt after an idle period pay for every row that accumulated during it                                                 |
| `attempts + 1`           | makes the limiter count to one forever and admit everything                                                                               |

The sweep removes up to 32 elapsed rows per call against the two one attempt
adds, which is what bounds a table an attacker fills by choosing fresh keys.
The key itself is the SHA-256 of `<action>:<dimension>:<value>`, so the value
— an email address that, on the interesting attempts, has no account — is
never written down.
