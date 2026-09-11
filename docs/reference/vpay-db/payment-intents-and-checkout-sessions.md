# `vpay-db` — `payment_intents` and `checkout_sessions`

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

## `payment_intents`

Two rules the module exists to keep.

**Every query is merchant-scoped in SQL, not in Rust.** There is no
`get(id)`: the merchant is a parameter of the lookup itself, so a handler
cannot forget to filter and cannot leak another merchant's object by reading
first and comparing afterwards. A foreign id therefore comes back as `None` —
indistinguishable from a missing one, which is what
[merchant-auth.md](../../flows/merchant-auth.md) requires, because an
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
([crash-safety.md](../../flows/crash-safety.md)). So there is a real, reachable
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
