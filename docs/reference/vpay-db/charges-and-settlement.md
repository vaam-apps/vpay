# `vpay-db` — `charges` and `settlement`

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

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
[crash-safety.md](../../flows/crash-safety.md) requires the charge row — carrying
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
[vpay-worker.md](../vpay-worker.md#nothing-younger-than-the-window-is-recovered)),
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
`state = 'submitting'`. [crash-safety.md](../../flows/crash-safety.md)'s
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
([failures.md](../../flows/failures.md)), while a submit moves it to a live one and
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
rail has answered. All three of [crash-safety.md](../../flows/crash-safety.md)'s
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
[payment-lifecycle.md](../../flows/payment-lifecycle.md) is explicit that such a
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
