# S4a — the Customer object, as built

Implementer's transcript for `claude/exp19-customers-opus`, base `aa912bb`
(master after PRs #55, #60, #62). Everything here was measured on this branch
on 2026-09-06 against `postgres:16-alpine`, under the pinned 1.98.0 toolchain,
Node 22.23.2 from `.nvmrc`, and `cratestack 0.11.1` on `PATH`.

The brief's design decisions were left to measurement wherever it could
answer. Where I deviated from the brief, § 6 says so and why.

---

## 1. The one column that shaped the whole persistence layer

The brief said "keep `metadata` JSONB raw, one hand statement, like the
outbox". Working out what that costs turned out to be the design.

`model Customer` does not declare `metadata`, for `model Event.data`'s two
measured reasons (exp18 § F5, exp17): `map_scalar` does not read `jsonb` back,
so a declared `metadata Json` would leave the live column invisible while
making the declared one a `[blocking]` line; and `Value::from_plain_json`
demotes any JSON number outside `i64` to `f64`, silently, on a column that is
merchant-authored and echoed back verbatim.

The consequence is **sharper than it was for `events`**, and I did not expect
it: an undeclared column is absent from the generated model *struct* as well
as from the create input, so a CrateStack **read** cannot render the wire
object at all. `events` lost one insert to this; `customers` loses create,
update, and every read — five of seven methods.

| Method | | Why |
|---|---|---|
| `touch_last_used` | **CrateStack** `update_many` | touches only `last_used_at` |
| `delete` | **CrateStack** `delete_many` | touches no column at all |
| `create`, `update`, `get_for_merchant` | raw `sqlx` | carry `metadata` |
| `list_page` | raw `sqlx` | correlated subquery for the `seq` cursor |
| `idle_since`, `delete_idle` | raw `sqlx` | correlated `NOT EXISTS` over two *other* tables |

I do not think this is a bad outcome and the module doc argues so rather than
apologising: the two that moved are the two where being wrong is irreversible.
`touch_last_used` is the write the twelve-month sweep reads, and `delete` is
the hard delete of personal data. Both `@@allow` failures are **silent** —
policy is compiled into the `WHERE` — so `every_action_this_module_calls_has_an_allow_arm`
asserts the compiled descriptor with no container, and asserts the *absence*
of the three arms the model deliberately does not grant.

### `list_page`'s cursor is not expressible, and the two-statement version is worse

Not "harder": *different*. The cursor is `seq < (SELECT seq FROM customers
WHERE id = $2 AND merchant_id = $1)`, a correlated subquery, and
`cratestack::Filter` has no constructor for one. Resolving the id to a `seq`
first and then filtering has a race — between the two reads the cursor row can
be **deleted**, and this table's rows are, by both the API and the sweep. The
one-statement form degrades to an empty page there, which is what
`paging::validated_cursor`'s documented behaviour already is.

### `touch_last_used` is monotonic by *filter*, because `GREATEST` is not renderable

`UpdateCustomerInput` renders a plain assignment. So the guard is the
predicate: `where_(last_used_at.lt(now))` makes a stale stamp match zero rows,
which is the same observable behaviour, and `Ok(false)` gains one more normal
meaning.

It matters because `now` is the *calling process's* instant, two vpay
processes do not share a clock, and the horizon is twelve months — a rewind is
the difference between surviving a pass and not.
`a_retention_stamp_never_moves_a_customers_clock_backwards` drives it through
the repository against a real Postgres.

It is also why migration `0034` carries **no**
`CHECK (last_used_at >= created_at)`. I wrote one, then removed it: both
instants come from process clocks, so a server a second behind would turn a
`POST /v1/payment_intents` into a `500` for a reason no merchant could act on.
The migration says so where the constraint would have been.

---

## 2. The bug the suite found on its first run

`JobKind::from_wire` held a **hand-maintained list**. `as_wire_str` is an
exhaustive `match`, so a variant added to the enum without a wire spelling
does not compile; `from_wire` iterated a private copy of the same list, and
`SweepIdleCustomers` was missing from it.

Measured, from the integration suite's own instrumentation:

```text
Settled { kind: "sweep_idle_customers", disposition: DeadLettered,
          error: Some("job … is poisoned and cannot be interpreted:
                       `sweep_idle_customers` is not a job kind this build knows;
                       the row was written by a different version"),
          alert: true }
```

For a kind this build ships. The sweep never ran, the customer survived, and
every log line blamed a phantom deployment skew. `cargo build`, `just clippy`,
`just check-schema` and all ten `verify` gates were green.

**Why nothing caught it.** `jobs.rs`'s own
`the_kinds_are_exactly_the_check_constraints` iterates a `KINDS` array — which
was a *second copy* of the same list, in the test module. I updated that copy
and `as_wire_str`, and the parser's copy was the one I missed. A test that
owns its own copy of the thing it is checking cannot catch a divergence
between two other copies.

**Fix.** The list is `JobKind::EVERY`, a `pub const`; `from_wire` iterates it;
the test's `KINDS` is now `= JobKind::EVERY` rather than a copy. Mutation, run
and reverted: delete `SweepIdleCustomers` from `EVERY` and

```text
FAIL [0.004s] vpay-worker jobs::tests::the_kinds_are_exactly_the_check_constraints
  left: [.., "scan_deliveries"]
 right: [.., "scan_deliveries", "sweep_idle_customers"]
```

4 ms, no container.

**What I did *not* claim.** There is no construction at 1.98.0 that makes a
string→enum parse exhaustive, so `EVERY`'s doc says plainly that this is
test-enforced and **not** compiler-enforced, and names the test. (The Rust
SDK's `KnownEventType::from_wire` carries a comment claiming a `match` over
strings gives exhaustiveness. It does not. I left it alone — it is outside
this change and the list there is short — but it is worth someone's attention.)

**What caught it** was the integration suite driving `vpay_worker::run_once`
rather than calling `delete_idle`, and it only caught it *loudly* after I made
`run_the_sweep` assert the **disposition**. "The row survived" reports a dead
letter by accident; it also reports a wrong horizon, a wrong guard, and a
missing seed identically.

---

## 3. Drift, measured

Driven directly — `cratestack migrate baseline --strict` against two freshly
built containers, one with this branch's migrations and schema and one with
`aa912bb`'s — rather than by editing the constants until the test passed.

| | changes | relations | unmappable |
|---|---|---|---|
| `aa912bb` | 101 | 16 | 17 |
| this branch | **113** | **17** | **18** |

The +12, line for line from `diff`:

* **10 on `customers`**, a new relation: six `[safe] CHECK … exists in the
  live database but is not declared` (`id_length`, `merchant_id_length`,
  `name_length`, `email_length`, `phone_is_a_canonical_msisdn`,
  `metadata_is_object`), three the same for its indexes, one
  `[safe] column seq default value differs`.
* **2 on `payment_intents`**: `[lossy] column customer_id …` and
  `[safe] index payment_intents_customer_idx …`.

**`checkout_sessions` gained the identical column and index and cost zero**,
because it is an undeclared *table* and the report collapses it to one line.
So the marginal drift of a column depends on whether its table is modelled —
the opposite of the intuition that modelling a table reduces drift, and worth
knowing before the next model lands. `model Customer` is what makes
`customers` cost ten lines; not declaring it would have cost one.

Each of the ten is a deliberate choice, argued on the model:

* the CHECKs are **hand-named**, so `@db_enforce` would emit a drop-and-add
  **pair** per constraint (exp17 § 1a). Ten lines would become sixteen.
* `@@index([merchant_id])` cannot express `(merchant_id, seq DESC)`.
  Declaring it adds a `[blocking] … declared in the schema but does not exist`
  line *beside* the `[safe]` one rather than closing it — two lines to
  describe one index, neither of them true. So the model declares no index at
  all and says why.
* `seq` is `model Event.seq`'s known trade: an identity column carries no
  `pg_attrdef` default, and dropping the `@default(...)` would make `seq` a
  required field of a create input for a `GENERATED ALWAYS` column.

`at_least_one_identifier` — the eleventh multi-column CHECK in the database —
contributes **nothing in either direction**, which is why `postgres_smoke`
asserts it directly and in *both* directions (the refusal, and that a
phone-only insert is accepted).

---

## 4. `sql_audit` fired twice, unprompted

Worth recording because one was a true positive on my design and the other is
a property of the scanner that will recur.

1. **True positive.** The sweep's `NOT EXISTS` pair started as
   `fn unreferenced(alias: &str) -> String` so the read and the write could
   name the table differently. They do not — both spell it `customers` — and
   the gate is what said so, by failing on `{guard}`. It is now
   `const UNREFERENCED`.
2. **False positive.** `!sql.contains(&format!("{column} = "))` inside a
   `#[cfg(test)]` assertion. The scanner finds statement-building `format!`s
   by looking for the word `sql` in the forty characters before the macro, and
   every line in that loop has `sql` in it — moving the `format!` out of the
   `contains(..)` argument was **not** enough, because the previous line's
   `{sql}` is still inside the window. The needle is built with `concat`
   now, and the comment says why.

`EXPECTED_ASSERT_SITES` 37 → 43, with the audit in
`docs/reference/vpay-db.md` re-done rather than incremented.

---

## 5. Mutations

Each applied to a clean tree, run, reverted; `git status` clean after every
one.

| # | Mutation | Measured |
|---|---|---|
| 1 | Delete `SweepIdleCustomers` from `JobKind::EVERY` | `the_kinds_are_exactly_the_check_constraints` **FAILS in 4 ms**, no container, naming both lists. This is the *real* bug of § 2, re-armed |
| 2 | (found, not injected) `from_wire`'s list missing the variant | the sweep is **dead-lettered**; `the_sweep_deletes_an_idle_unreferenced_customer_and_keeps_the_other_two` fails, and after the disposition assertion was added it fails **naming the dead letter** rather than only "the customer survived" |
| 3 | `preview_sql` of `customer().create(..)` — remove `metadata`'s absence assertion's premise by declaring `metadata Json` | not run as a schema edit; the *assertion* is the tripwire and is written to fail on the rendered statement. What was run: the first version of that test asserted on the whole statement including `RETURNING`, and **failed on its own `RETURNING`** — which is why it now splits on `RETURNING` and additionally asserts the split was real, so the negative assertions cannot go vacuous |
| 4 | Two `preview_sql` assertions about *filters* | **both failed**, correctly: `UpdateManySet::preview_sql` and `DeleteMany::preview_sql` render the predicate as the literal `<filters> AND <update_policy>`. So no unit test here can say anything about the `id`, `merchant_id` or `last_used_at` filters. A `contains("merchant_id")` would have **passed** against the `RETURNING` projection whether or not the filter existed — a test asserting nothing. Both tests now assert only what the preview proves, say in their own docs what they cannot see, and name the container test that proves the rest |

Mutation 4 is the one I would flag to a reviewer: I wrote two tests that
looked decisive, ran them, and found that one would have been *vacuously
green*. The tenancy filter on `delete` and the monotonicity filter on
`touch_last_used` are proved by behaviour in
`another_merchants_customer_cannot_be_deleted`'s path and
`a_retention_stamp_never_moves_a_customers_clock_backwards`, not by any
rendered string.

---

## 6. Where I deviated from the brief

**One, and it is deliberate.** The brief asked for `customer.created`,
`customer.updated` and `customer.deleted` in `type_is_a_documented_event`.
I added **only `customer.deleted`**.

Nothing writes the other two. `POST /v1/customers` and
`POST /v1/customers/{id}` are single statements on the pool; emitting an event
means putting the write and the event in one transaction, which is a change to
the shape of two repository methods rather than a line in a `CHECK`. Migration
`0023`'s own comment states the rule this repository holds itself to — *the
vocabulary moves in lockstep with the code that writes it, rather than being
written permissively ahead of it* — and `0029` followed it exactly, adding one
type for one writer.

`payment_intent.created`, `.processing` and `.canceled` **are** in the list
with no writer, and they are the precedent for *not* doing it again: they came
in together in `0018`, before the rule was written down, and
`docs/flows/webhooks.md`'s Status section has listed them as unwritten ever
since.

`customer.deleted` had to exist regardless of scope: a hard delete is
unobservable by polling, so without it a merchant whose customer was swept has
no way at all to learn it happened.

The gap is recorded in four places rather than left to be discovered:
migration `0034`'s own comment, `docs/flows/customers.md` "What is not built",
`docs/flows/webhooks.md`'s Status section, and a dated ⛔/⛔ row in
`docs/sdks/parity.md` **owned by the vpay maintainers rather than the SDK
ones** — because the SDKs are at parity with each other and short of Stripe,
which is a different statement from being short of vpay.

**`address` is not built**, which the brief permitted ("or omit address in the
first cut … DECIDE by measuring"). I measured and it *is* expressible — six
nullable text columns, no CrateStack obstacle at all — and left it out to keep
the first cut to the fields the maintainer's decisions are about. That is a
scope choice, not a technical finding, and it is recorded as a gap rather than
as a decision against an address.

---

## 7. Maintainer decisions surfaced, not taken

1. **`address`.** Above. Expressible, absent. Adding it later is additive in
   both SDKs and in the object (a new key, `null` on every existing customer).
2. **`customer.created` / `customer.updated`.** § 6. The cost is turning
   `Customers::create` and `Customers::update` into transactional methods that
   take an `event_id` and a rendered `event_data`, exactly as
   `CheckoutSessions::expire_due` and `Customers::delete_idle` do. The update
   path has a wrinkle worth knowing before deciding: the API already reads the
   row (to merge `metadata`), so it *can* render the post-patch object — but
   that read-modify-write has a window, and an event asserting a state that
   lost a concurrent race would be worse than no event.
3. **The retention horizon is a constant, not configuration**, and I made that
   call rather than surfacing it, on ADR-0003's own line: a retention period
   is a promise to a *payer*, and a deployment that could shorten it from a
   YAML file is one where that promise depends on an operator nobody audits.
   Recorded here so it can be reversed knowingly.
4. **"Delete this customer" is not a complete erasure of the payer**, because
   the `NO ACTION` foreign keys keep the payment record. That is the right
   trade for a payments system and it is not obviously the right trade for a
   privacy request; the alternative I rejected (`ON DELETE SET NULL`) is
   worse, but a third answer — deleting the customer *and* redacting the
   identifiers off the retained intents — is a real option nobody has costed.
   `docs/flows/customers.md` states the trade plainly rather than implying a
   full erasure.
5. **The 404 body echoes the caller's own id.** This is pre-existing and not
   mine, but S4a's tenancy test had to work around it (substituting each
   request's own id out, as `refunds.rs` already does) and it is worth someone
   deciding whether the echo is wanted. It leaks nothing — the caller sent the
   id — but it makes "byte for byte identical" a claim that needs a caveat
   every time it is made.

## 8. Not checked

* Nothing about `cratestack` 0.11.1 beyond the files exp17/exp18/exp20 cite
  and the four builders this change calls.
* No rolling-deploy test of migration `0034`. It is additive — a new table and
  two nullable columns — so a pre-`0034` binary is unaffected by it, unlike
  `0032`; but that is reasoning, not a measurement, and exp17 § 3's finding is
  the reason to say so rather than assume it.
* Cypress, the e2e compose stack, and the shop demo (PR #64, landing
  concurrently and deliberately untouched).
* No live rail. Nothing here calls one.
