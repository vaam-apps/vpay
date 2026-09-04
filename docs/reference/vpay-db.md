# `vpay-db` reference

Why the code in `backends/crates/vpay-db` looks the way it does. The crate's
own doc comments say *what* each item is and link here; this page carries the
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
- [`webhook_deliveries`](#webhook_deliveries)
  - [`record_attempt`: what each column is allowed to say](#record_attempt-what-each-column-is-allowed-to-say)
  - [`pending_due` is a backstop, never a scheduler](#pending_due-is-a-backstop-never-a-scheduler)
- [`jobs`](#jobs)
  - [The lease is the whole design](#the-lease-is-the-whole-design)
  - [`enqueue_in_tx` exists only in the transactional form](#enqueue_in_tx-exists-only-in-the-transactional-form)
  - [`pull_forward_in_tx` is the exception, and it has to be asked for](#pull_forward_in_tx-is-the-exception-and-it-has-to-be-asked-for)
  - [Why claiming does not consider lease expiry](#why-claiming-does-not-consider-lease-expiry)
  - [Why a dead letter is parked and not deleted](#why-a-dead-letter-is-parked-and-not-deleted)
- [TLS: no `CryptoProvider` is installed here](#tls-no-cryptoprovider-is-installed-here)

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
back *without* that being an error, and the confirm path's duplicate-charge
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
Three call sites raise their *own* layer's error from inside the unit of work —
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
writer actually takes its lock has to be able to *hold* that lock from outside
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

`SqlClientAssertionStore` is the one place a *foreign* trait
(`authkestra_op::client::ClientAssertionStore`) is implemented over the
database, and it was the precedent this seam followed.

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
under concurrency; this one *is* the write statement.

### `cancel` checks for a live charge inside the statement

`requires_payment_method` is not on its own enough to make a cancel safe. A
`confirm` commits its charge row — carrying the `provider_reference_id` it is
about to submit under — *before* it calls the rail, and leaves the intent's
status alone until it knows what happened
([crash-safety.md](../flows/crash-safety.md)). So there is a real, reachable
window in which the status still says `requires_payment_method` while a live
charge exists.

Cancelling there would tell a merchant the payment was withdrawn while the rail
may hold it. A check in the *caller* would not fix it: between reading "no
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

### `set_payload` is a separate write from `reschedule`

The recovery table keeps per-job state in the payload — the `not_found_streak`
and `first_not_found_at` that decide when a charge the rail claims never to have
seen is resubmitted. That state has to survive the *current* attempt even when
the job is not being rescheduled at all (it is being finished, or it is about to
fail), so it cannot ride along on the rescheduling statement.

The two writes are therefore not atomic with each other, deliberately: the worst
a crash between them can do is lose one increment of a counter whose only effect
is *when* a resubmit happens. Making them one statement would mean either a
`reschedule` that silently rewrites a payload its caller did not mean to touch,
or a payload update that cannot happen without also moving the schedule. Neither
trade is worth the atomicity of a retry heuristic.

## `checkout_sessions`

Migration `0028`. One *checkout attempt* driven through a page vpay serves —
`cs_…`, referencing an existing `pi_…`, carrying the merchant's forward URLs
and **two** payer credentials of its own. The three rules the module keeps are
`payment_intents`' two (merchant-scoped in SQL; compare-and-swap on status)
plus one: the single unscoped read is named `get_by_id_unscoped` so a `/v1`
handler that reaches for it has to type the word.

**Two credentials, not one, and that is the whole of D6.**
`client_secret_suffix` joins with the row's `id` into `cs_…_secret_…` and
rides in a URL *fragment*, which never leaves the browser; presenting it buys
the intent's own `client_secret`, and therefore the ability to confirm.
`return_token` rides in a *query string* — it has to, because a fragment does
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

`ORDER BY seq DESC LIMIT 1`, and one row is enough because an *open* session
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
be added for it: 0028's only lookup by intent is *partial*
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
silently ignores. The filter is applied *in the statement*, beside the tenant
filter: applied after `LIMIT` it would return short pages and a `has_more`
describing the wrong set.

### Why the settlement flip is `pub(crate)` and not a trait method

`checkout_sessions.payment_status` denormalises what the intent says, so a
payer's page can render an outcome from one read. That is only safe while the
two cannot disagree — and they cannot only if the session's write lands in the
*same transaction* as the intent's.

So `checkout_sessions::settle_for_intent(tx, intent_id, paid)` takes a
`&mut PgConnection` and is `pub(crate)`, reachable from `settlement` and
nowhere else. The visibility is what enforces that rather than this paragraph:
a caller elsewhere — a handler, a repair script, a worker hook written after
the fact — would have to move a `pub` in the same diff, which is the moment
the argument has to be re-made. Same device, same reasoning, as
`payment_intents::succeed_after_submission`.

The Step 9 plan calls this "the worker hook" and locates it in
`vpay-worker/src/handlers.rs`. The *decision* is indeed the worker's —
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
and a `confirm` commits its charge *before* it calls the rail. Expiring there
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
an event *is*: `events.data` holds the **rendered wire object**, which only
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
is a property of *which transition this is*, and a caller free to choose it
could report an abandoned checkout as a settled payment.

**The guard is evaluated twice, on purpose.** `due_for_expiry` carries the
same `status`/horizon/`NOT EXISTS` predicate the write does. The write needs
its own copy because the read's answer is stale the moment it returns — a
payer can confirm in between, and `Ok(None)` on that path is the *normal*
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
reached from a URL the *rail* holds — built once at submit, stored, and
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
second copy that can drift. The real rule — *the key belongs to this session's
merchant* — is the registration list, which no constraint can see either
(there is no merchants table; ADR-0003), so
`vpay_api::v1::checkout_sessions::chosen_publishable_key` is what enforces it.

**`CheckoutSessionRow::return_page_url(checkout_base)`** builds
`{base}/c/{id}/return?t={return_token}&key={publishable_key}`. It is a method
on the row rather than a `format!` in `vpay-api` because two callers construct
it — the confirm path, when a session drives the charge, and the return trip —
and every character has to be identical between them, since the *rail* holds
the copy that matters. Both values are URL-safe by construction (`vpay_core`'s
base32 alphabet, and `pk_` plus `[A-Za-z0-9]`), so it is a `format!` and not
an escaping routine; a future alphabet that needed escaping would break
`vpay_core::ids`' own test first. A trailing slash on `checkout_base` is
absorbed, so `//c/…` — a protocol-relative URL naming a different host — is
not reachable through it.

## `charges`

Three writes, and only one of them is unguarded. `insert_for_intent` opens the
charge before the rail is called; `mark_submitted` and `mark_failed` record what
the rail answered, and both are compare-and-swaps out of `submitting` rather
than blind updates, so a recovery pass and a live confirm cannot overwrite each
other's answer.

The writes that take a charge to a *terminal* state from anywhere in the live
set — what the worker's poll ladder decides — are not here. They move the
charge, the intent and an `events` row together and therefore belong to the one
transaction that does all three (see [`settlement`](#settlement)); splitting
them across this module would have made it possible to call one without the
others, which is the specific thing that transaction exists to prevent.

`insert_for_intent` takes a connection and not a pool because
[crash-safety.md](../flows/crash-safety.md) requires the charge row — carrying
the `provider_reference_id` the rail will be given — to be committed *before*
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
a *friendly* `409` without attempting the write — but that read is an
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
rail answered the *second* submit with, and a push rail answers with an empty
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
the caller because *every* transition passes through those six statements and
only some of them pass through the worker: a confirm opens and submits a charge
inside `vpay-api`, so a counter mounted on the worker's settlement points would
be silently blind to the busiest half of the state machine.

Two rules make the count mean what it says.

**Every label is read back off the returned row**, never off the caller's
argument, and the recording happens only after the statement returned a row — a
compare-and-swap that matched nothing is a transition that did not happen.

**A transition is counted after it is committed, never before.** The three
writes in `settlement` own their own transaction, so they record after their own
`COMMIT`. The three in `charges` run inside a *caller's* transaction — that is
the whole point of taking a connection — so they cannot record at all: a
`ROLLBACK` after the insert, from a later statement in the same transaction
failing, would leave a counter claiming a charge that does not exist. Instead
each returns its row and the caller calls `record_opened` or
`record_left_submitting` **after** the commit. The seam is still this module —
the label vocabulary and the metric name are here and the callers pass no
strings — but the *timing* has to belong to whoever owns the commit, because
nothing inside a transaction can know whether it will be committed.

Until 2026-09-03 all three recorded inline, and the module claimed the metric
"cannot claim a transition the database refused" while a rolled-back insert was
counted. What that timing costs now: a caller can *forget* to record, which an
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
`UPDATE` on the charge still being in a *live* state. A re-run after a commit —
the poll job was rescheduled because the worker died between committing and
deleting the job, which is a normal outcome and not an error — matches zero rows
and returns `Ok(None)`; the caller finishes the job. Nothing is written twice,
and in particular no second `events` row, so at-least-once job execution does
not become at-least-twice webhook delivery for distinct event ids. That guard
has to be in the statement: a `SELECT` that checked the state first would leave a
window in which two workers — one holding a stale lease, one that just claimed
the reaped job — both see a live charge and both settle it.

**The charge is the record of a confirm; the intent may lag it.** A confirm
commits the charge (and its poll job) in one transaction *before* calling the
rail, and moves the intent only afterwards, in a second transaction, once the
rail has answered. All three of [crash-safety.md](../flows/crash-safety.md)'s
kill points therefore leave a live charge against an intent still reading
`requires_payment_method` — not a corrupt database, but the ordinary state a
crashed confirm leaves and the one the recovery pass exists to resolve. So the
question these functions answer is never "does the intent's status agree that a
confirm happened": the charge answers that, because the compare-and-swap has
already matched a row in the live set and only a confirm writes one. The intent
write follows over a *wider* set — the two confirmed statuses **and**
`requires_payment_method` — so a settlement lands whether or not the confirm
survived long enough to move the intent.

**Where the settlement's `from` label comes from.** The two settlement
statements need a `from` label their `WHERE` clause cannot supply, since it
matches a *set* of live states rather than one, so each `RETURNING` carries an
extra `(SELECT prev.state FROM charges prev WHERE prev.id = charges.id)`. That
sub-select reads the statement's own snapshot — an `UPDATE` never sees its own
writes — so it yields the state the charge was in *before* this statement. It
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
the *set* rather than on a single expected status supplied by the caller,
because the worker settling a charge does not know, and must not have to know,
which rail's flow put the intent where it is: branching on that in the caller
would be exactly the rail-shaped branch ADR-0002 forbids, while naming the legal
*values* is not.

`requires_payment_method` is in the set because a crash puts it there.
Excluding it made the settlement of a crashed confirm unreachable: the charge
compare-and-swap would fire, the intent guard would match nothing, and the whole
transaction became `DbError::WriteMatchedNoRow` → `Category::Internal` →
`Retry::Never` → a dead-lettered poll job, with the charge left live and nothing
ever driving it again. A charge the rail may have collected is exactly what must
not be parked. It is safe because the settlement writers are never called on
their own — they run inside `settlement`'s transaction, *after* a charge
compare-and-swap over the live states has already matched a row, and a live
charge is proof a confirm happened whatever the intent's status says. That is
also why they are `pub(crate)`.

`fail_after_submission` therefore performs a real
`requires_payment_method` → `requires_payment_method` write: the status does not
move and the write is the error pair alone, and counting that as *applied* is
the point. It sits next to `record_payment_error` because the two are different
moments — that one is for a rail that declined at submit, where the intent never
left `requires_payment_method`; this one is for a decline the *poll* discovered
after the intent had already moved, and
[payment-lifecycle.md](../flows/payment-lifecycle.md) is explicit that such a
failure returns the intent to `requires_payment_method` with
`last_payment_error` populated. The status change and the error pair must happen
in the same statement: an intent back at `requires_payment_method` carrying no
error reads to a merchant as one that was never attempted.

A merchant polling `GET` then sees a resolved intent that *looks* confirmable
again. It is not — "one charge per intent, forever" means the failed charge
still blocks a second `confirm`, which answers `already_charged` and tells the
merchant a retry is a new intent. That guard is what makes this transition safe.

`succeed_after_submission` sets `amount_received = amount` rather than taking a
parameter. Neither rail vpay speaks to can settle *part* of a submitted amount —
`ChargeStatus::Succeeded` carries a transaction identifier and no amount at all
— and taking one here would invite a caller to derive it from the charge, which
is already required to equal the intent's amount. When a rail that can partially
collect arrives, this becomes a parameter *and* `succeeded` stops being the
right status; that is a change to the state machine, not a missing argument
today.

Neither writer is merchant-scoped, unlike every other query in
`payment_intents`. The caller is the worker settling a charge, not a merchant
addressing their own object, and there is no request whose authorisation could
be checked. Taking a `merchant_id` the worker would have to look up from the
intent it is already holding would *look* like an authorisation check while
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
because it is the *fan-out's* closing write and is meaningless without the
inserts it commits beside: a caller that could reach it from the events module
could mark a backlog fanned out without creating a single delivery, which is
precisely the failure the shared transaction exists to make unreachable.

**Every column but `created_at` describes the most recent attempt.** `attempt`,
`state`, `status_code`, `response_excerpt`, `sent_at`, `responded_at` and
`next_attempt_at` are all rewritten by `record_attempt` and `record_success`.
This is a *state* row with the latest attempt's outcome on it, not an
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
comparing against a body that never existed. *Rendered and signed*, not
*received*: a transport failure passes `Some`, because the signature was
computed over those exact bytes before the socket was ever opened.

When `sha` is `Some` it is `COALESCE`d rather than assigned, so the digest of
the *first* attempt that rendered and signed a body survives — which is not
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
disagree. They can: the transaction makes the job *exist*, and nothing makes it
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

A job is *claimed* by an `UPDATE` that stamps `locked_at`/`locked_by` on exactly
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
transaction is what makes *all three* of that document's kill points leave a job
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

Step 8 lane C added one write that *does* move a scheduled job back to now:
`UPDATE jobs SET run_at = now() WHERE dedupe_key = $1 AND locked_at IS NULL
AND run_at > now() + $2 AND run_at < 'infinity'`. Its only caller is
`vpay_api::provider_callback` — a rail said something happened, and the point
of a callback is to ask the rail *now* instead of at the ladder's next rung,
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
  fastest rung, and it is a *parameter* because the ladder is
  `vpay_worker::poll_delay` — a policy about how often a rail is asked
  anything, which this crate must not hold (ADR-0002). The caller passes
  `vpay_api::provider_callback::PULL_FORWARD_FLOOR`, and
  [vpay-api.md](vpay-api.md#what-an-anonymous-caller-can-and-cannot-get-out-of-it)
  states what it does and does not bound.
- `run_at < 'infinity'` — a dead letter stays parked. The section below states
  that the occupied `dedupe_key` is what keeps a scan *or a callback* from
  re-creating work a human has to look at first; this is the clause that makes
  the "or a callback" half true.

`Ok(false)` is therefore "nothing to do" in all of these cases and never a
failure, which matters because the caller always calls `enqueue_in_tx` first:
a job it just inserted at `now()` is the ordinary `false`.

### Why claiming does not consider lease expiry

`claim`'s predicate is `locked_at IS NULL`, full stop, so it matches
`jobs_claimable_idx` exactly. "Unlocked *or* the lease has expired" depends on
`now()` and cannot be an index predicate, so it would turn every claim into a
scan over every leased row. Expiry is therefore a separate, periodic pass —
`reap_expired_leases` — which frees a stale lease *once* and lets the ordinary
claim path pick the row up on its next turn. Its callers are described in
[vpay-worker.md](vpay-worker.md#two-lease-reapers-on-purpose).

### Why a dead letter is parked and not deleted

A job that is done is deleted (`finish`); a job that is not done is rescheduled
with its error recorded (`reschedule`). A job that *cannot* be done —
`JobError::Poisoned`, or anything else `Classify::retry` answers `Retry::Never`
for — is neither, and `dead_letter` is the third write.

It exists because deleting one is not safe for a *payment* queue. `poll_charge`
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
and to every other `run_at`-ordered query, so the *only* way an operator learns
one exists is the alert the loop raises when it parks it, and
`SELECT * FROM jobs WHERE run_at = 'infinity'`. Requeuing one is an
`UPDATE jobs SET run_at = now()` by hand, which is deliberate: it should follow
a human deciding the underlying data is fixed.

`last_error` carries `vpay_core::error::source_chain` and not `Display` alone
(ADR-0011's amendment) — `ProviderError::Transport` keeps the `reqwest` error as
a `#[source]`, so the column would otherwise say "the request to the rail
failed" and never "operation timed out".

## TLS: no `CryptoProvider` is installed here

The root `Cargo.toml`'s comment on the `authkestra-*` dependencies documents a
real requirement: those crates build `reqwest` clients with `rustls-no-provider`,
so the *first* one constructed panics unless a process-wide default
`CryptoProvider` was already installed. `sqlx` looks like the same hazard and is
not.

`sqlx` is configured with `tls-rustls-ring`, which vendors Mozilla's CA bundle
via `webpki-roots` (the runtime image is `FROM scratch` per ADR-0004, so there
is no OS trust store for the `-native-roots` alternative to read). Reading
`sqlx-core` 0.8.6's own TLS setup (`src/net/tls/tls_rustls.rs`) shows it never
calls `rustls::crypto::CryptoProvider::get_default()` — the call that panics
without an installed default. It builds its own provider inline and passes it
explicitly:

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
