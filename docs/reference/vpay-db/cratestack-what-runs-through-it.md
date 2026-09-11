# `vpay-db` — CrateStack: what runs through it today

_Moved out of [docs/reference/vpay-db.md](../vpay-db.md) on 2026-09-11 by exp57, which split a 3 330-line reference into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### What runs through it today

**Thirty-two statements, twelve tables, twelve models** — the whole list,
measured 2026-09-10 rather than accumulated. The rule, so it can be
re-derived: every `find_unique`/`find_many`/`create`/`upsert`/`update_many`/
`delete_many` chain in `backends/crates/vpay-db/src/*.rs` outside a
`#[cfg(test)]` module, each ending in exactly one `run(ctx)` or
`run_in_tx(tx, ctx)`. (`migrations.rs` has a `.run(&self.pool)` that is
`sqlx::migrate!`'s, not a builder's, and is not one of the thirty-two.)

**The `procedure` added on 2026-09-11 moves none of these numbers, and that is
the rule rather than an exception.** `searchPaymentIntents` runs one
hand-written `&'static str` against `payment_intents`; it is not a builder
chain, it has no `run(ctx)`, and `payment_intents` is not one of the twelve
tables. A procedure never appears in this table — see "The `procedure` seam"
below for what it is instead.

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
[../plans/exp18-notes/opus-review.md](../../plans/exp18-notes/opus-review.md) §3
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
[../plans/exp45-worker-pool-bound-notes/opus-review.md](../../plans/exp45-worker-pool-bound-notes/opus-review.md).

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
   ([../flows/webhooks.md](../../flows/webhooks.md)), and it embeds
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
has no shipping caller at all yet (see [roadmap.md](../../roadmap.md)), and the
_insert_ branch takes only one connection, because `auth().isSystem()` is not
a relation predicate and `evaluate_create_policies` therefore issues no query
for it. It is the first thing to re-check if a route or an admin surface ever
calls this method concurrently, and it is on the list of things worth sending
upstream ([docs/plans/exp16-notes/opus-review.md](../../plans/exp16-notes/opus-review.md) § 6):
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
