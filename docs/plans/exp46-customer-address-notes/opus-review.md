# exp46 — the sabotage review: what the erasure was attacked with, and what held

Reviewer's notes for the branch `claude/exp46-customer-address` (issues
[#67](https://github.com/vaam-apps/vpay/issues/67),
[#68](https://github.com/vaam-apps/vpay/issues/68),
[#96](https://github.com/vaam-apps/vpay/issues/96) item 2). The implementer's
notes are [opus.md](opus.md); this file records the attack, not the design.

The subject is a promise made to a person who is not vpay's customer and
cannot check it. So the review's whole shape is: **try to find a surviving
identifier**, and refuse to accept the absence of one as evidence unless the
same instrument found it a moment before.

## The rebase came first, and it carried one silent hazard

Master moved by 24 commits under this branch —
[#108](https://github.com/vaam-apps/vpay/pull/108) (exp44, which took `0040`)
and [#109](https://github.com/vaam-apps/vpay/pull/109) (a repo-wide prettier
reformat that also made `pnpm exec prettier --check .` a `just fmt-check`
gate). Four documents and two source files conflicted.

The hazard was not a conflict. `postgres_smoke.rs`'s
`assert_eq!(applied, 40, …)` had been moved from 39 to 40 by **both** sides —
by master for `0040` and by this branch for `0041` — so git merged it clean at
`40` when the truth was now 41. A conflicted line is a line somebody reads; an
auto-merged one is not. It is fixed, and the branch's own header prose ("NOTE
THE GAP: there is no 0040 on this branch") was deleted rather than reworded,
because the gap it explained closed when exp44 landed.

The doc conflicts were resolved by re-merging **prettified** copies of the
base and branch versions against master's formatted one — taking master's
formatting and re-applying this branch's content, rather than re-typing either
— and `docs/status.md`'s giant table was merged with the padding stripped and
re-added, because prettier's column widths change with the widest cell and a
line-wise merge sees every row as modified.

## Findings

Severity is the brief's: _gate-hole_ / _correctness_ / _rule-break_ /
_misleading-claim_ / _nit_.

| #   | Severity                         | What                                                                                                                                                                                                            | Fixed in  |
| --- | -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------- |
| F1  | gate-hole                        | The branch's own notes failed `just fmt-check` once #109 landed. `just ci` stopped at `fmt-check-web` on one file.                                                                                              | `a81ad32` |
| F2  | correctness (rebase)             | The migration count assertion auto-merged to `40` when both sides moved it; the truth was 41.                                                                                                                   | `f13b353` |
| F3  | **correctness**                  | `anonymized_customers_carry_the_marker` was spelled with `=`, and a CHECK passes when its expression is NULL. An `anonymized_at` row with a NULL identifier column was accepted — the partial erasure it names. | `839d3fe` |
| F4  | **correctness**                  | Redacting `events.data` under a live delivery **dead-letters the webhook** that announces the erasure: `payload_sha256` no longer matches the re-rendered body and `refuse_a_re_rendered_body` poisons it.      | `b14ed81` |
| F5  | **correctness / misleading**     | `charges.failure_raw` and `refunds.failure_raw` hold the rail's message verbatim and can quote the payer's MSISDN. Not in the "copies that survived, in full" enumeration, and not in the scanner's fixture.    | `94a030b` |
| F6  | misleading-claim                 | `vpay_api::v1::customers`' module header still stated "**`DELETE` is a hard delete**" and "**cannot be deleted** … answers a `409`", plus a seven-key object list.                                              | `f00dac0` |
| F7  | misleading-claim                 | `docs/api/README.md` — the merchant-facing reference — wrong in five places about `customer.deleted`, the key count, `address` ("a gap"), the `409`, and the DELETE route row.                                  | `f00dac0` |
| F8  | misleading-claim                 | `docs/reference/vpay-api.md`, `docs/flows/merchant-auth.md`, `docs/flows/README.md`, `v1::mod`'s route comment, `vpay_worker::jobs`' `SweepIdleCustomers` doc.                                                  | `f00dac0` |
| F9  | correctness (bounded, not fixed) | `PostRequest::finish` stores a response body **after** the handler's transaction, so an update racing an erasure can re-introduce identifiers into `idempotency_keys.response_body`. Bounded at 24 h.           | stated    |
| F10 | nit (not fixed)                  | An over-long address component answers "`address` must be at most 256 characters" without naming which component. The `param` must stay `address`; only the sentence could improve.                             | —         |

### F3 — the CHECK that made a partial erasure "unrepresentable" did not

The migration's own comment says a row with `anonymized_at` set carries the
marker in every identifier column "with no exceptions and no NULLs", and that
this makes a partially completed erasure unrepresentable rather than merely
discouraged. Against a real Postgres, with the migrations as delivered:

```sql
INSERT INTO customers (…, anonymized_at, name, email, phone, …)
VALUES (…, now(), NULL, '[redacted]', '[redacted]', …);
-- INSERT 0 1
```

A CHECK is violated only when its expression evaluates to FALSE. `name =
'[redacted]'` over a NULL `name` is NULL, and NULL passes. The delivered test
held each of the nine columns back as **another value** (`'Ada Ngo'`) and never
as NULL, so the gate could not see it — and NULL is the likelier half, because
the assignment an erasure misses is most easily one for a column the payer
never filled in. `IS NOT DISTINCT FROM` is FALSE rather than NULL when one side
is NULL.

### F4 — the erasure dead-lettered the delivery that announces it

`events.data` is rewritten for every `customer.*` body of the erased payer.
`webhook_deliveries.payload_sha256` is the digest the **first signed attempt**
recorded, compared against every later attempt because the envelope is
re-rendered rather than stored. A `customer.created` mid-ladder — a merchant's
receiver having an outage, which is what the ladder exists for — therefore
fails that comparison the moment the payer is erased:

```
Poisoned { reason: "delivery … re-rendered event `evt_…` to a different body
than the one the first signed attempt signed (stored digest b6db…, now 939b…);
a renderer changed under a live delivery" }
```

The merchant is never told, an operator is sent hunting a deploy that never
happened, and un-parking a dead letter is manual. Nothing could see it: every
case in `tests/webhooks.rs` succeeds on its first attempt, and every case in
`tests/customers.rs` runs with no endpoints configured.

The fix clears `payload_sha256` on the deliveries that are still `pending` or
`failed`, in the erasure's own transaction, so the next attempt signs and
sends the redacted body — which is also the privacy-correct outcome. It
narrows the digest guard in the one place vpay changes bytes on purpose, and
`refuse_a_re_rendered_body`'s own doc now says so.

### F5 — the rail's own words are a copy of the payer

`ChargeStatus::Failed`'s `raw` is `"{code}: {message}"`, assembled by MTN's
`wire::Reason::raw` and Orange's `mapping::raw_reason` from a body vpay does
not author, and stored verbatim because
[failures.md](../../flows/failures.md) requires an unmapped decline to survive
for whoever fixes the mapping table. A mobile-money rail refusing a collection
names the subscriber it refused it for.

Neither column is an _identifier column_, which is exactly why an enumeration
of identifier columns — however careful — did not have them, and why the
decisive test could not catch it: its fixture wrote the payer's literals only
into the places the design already knew about. The fixture now writes one into
a failed charge and its refund, and the `before` assertion names both, so a
future change that stops storing it there has to say so rather than making the
test quietly easier.

## The scanner: what it covered, what it did not, and what was added

`an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table` is the
right instrument and was already close to sound as delivered. It reads
`information_schema` for every `text`, `character varying` and `jsonb` column,
`position($1 in col::TEXT) > 0` per column per literal, asserts the column
count is over 100, and — the part that matters — **runs the same scan before
the erasure and requires every literal to be found**, plus requires named
places among the hits. A scan that scanned nothing fails, and so does one
whose literals were never written.

| Question the brief asked                        | Answer                                                                                                                                           |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| every text/varchar/jsonb column of every table? | Yes, in `public`. `authkestra.*` is out of scope and now says so on the function: every subject there is a merchant credential, never a payer's. |
| `events.data` nested objects?                   | Yes — the scan is `::TEXT` over the whole JSONB, so nesting is irrelevant to it.                                                                 |
| `webhook_deliveries` payload columns?           | There are none. `payload_sha256` and `response_excerpt`, no body (`0022`). Verified against the live table, not the doc.                         |
| `provider_requests`?                            | No bodies at all (`0016`): status code, attempt, `error_kind`.                                                                                   |
| `jobs` payloads?                                | Covered by the scan. `jobs` rows are `DELETE`d by `finish`, and no payload of the four kinds carries an identifier.                              |
| `idempotency_keys` response bodies?             | Covered, and in the fixture, and redacted. See F9 for the window that remains.                                                                   |
| the audit / sql-audit tables?                   | There are none. `vpay_db::sql_audit` is a source scanner, not a table; `model Customer` is not `@@audit`-enabled.                                |
| `staff_sessions`?                               | Covered by the scan; holds no payer data.                                                                                                        |
| does it fail when it should?                    | Yes — proved three times, below.                                                                                                                 |

**Added by the review:** a failed charge and a refund carrying the payer's
MSISDN in `failure_raw` (F5), the two `before` entries that name them, and the
statement of the scan's two limits on `scan_for` itself.

**Still not in the fixture, and why:** an invoice. `invoices` carries
`customer_id` and no payer identifier snapshot (checked against the live
table), so there is nothing for it to hold — but it is the shape of thing that
would change, and it is named here rather than assumed away.

## The attack table

| #   | Attack                                                                        | Result                                                                                                                                       |
| --- | ----------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| A1  | Erase a customer whose MSISDN is in `charges.failure_raw`, then scan          | **Survived** → F5                                                                                                                            |
| A2  | Same, in `refunds.failure_raw`                                                | **Survived** → F5                                                                                                                            |
| A3  | `anonymized_at` set with one identifier column left NULL                      | **Accepted by the database** → F3                                                                                                            |
| A4  | `anonymized_at` set with one identifier column left as another value          | Refused by `anonymized_customers_carry_the_marker`, all nine, as claimed                                                                     |
| A5  | Erase a customer whose `customer.created` delivery is mid-ladder              | **Dead-lettered** → F4                                                                                                                       |
| A6  | Read the delivered body after the erasure                                     | Redacted, once F4 is fixed — the payer does not go out on a retry                                                                            |
| A7  | Erase, then scan every `text`/`varchar`/`jsonb` column in `public`            | Nothing survives (after F5)                                                                                                                  |
| A8  | Erase a customer whose identifiers are in an `idempotency_keys.response_body` | Redacted in the same transaction. But see F9: a body stored _after_ the erasure is not                                                       |
| A9  | Update an erased customer                                                     | `409`, and `at_least_one_identifier` would not have objected — the refusal has to be above the statement, and is                             |
| A10 | Attach an erased customer to a new intent                                     | `409`                                                                                                                                        |
| A11 | Second `DELETE`                                                               | `200`, no second event                                                                                                                       |
| A12 | `address=Douala` (a scalar where an object belongs)                           | `400` naming `address`, rather than silently storing nothing                                                                                 |
| A13 | `address[country]=CMR` / `237` / `Cameroon`                                   | `400` naming `address`; `cm` is accepted and stored `CM`                                                                                     |
| A14 | Partial address update (`address[line1]` alone over a stored city)            | Replaces whole, as documented — no address assembled out of two                                                                              |
| A15 | Does the address join the erasure?                                            | All six components, and the CHECK names all nine columns. Verified by the scanner finding `customers.address_line1` before and nothing after |

## Mutations

Every one run against a real Postgres in a container; the delivered thirteen in
[opus.md](opus.md) were not re-run, these are the review's own.

| #   | Mutation                                                     | Caught by                                                                                                           |
| --- | ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------- |
| M-A | Spell the marker CHECK `=` instead of `IS NOT DISTINCT FROM` | `an_anonymised_customer_carries_the_marker_in_every_identifier_column` — "`name` survived an anonymisation as NULL" |
| M-B | Drop `failure_raw` from the `charges` redaction              | the scanner — `{"237600000771": ["charges.failure_raw"]}`                                                           |
| M-C | Drop the `refunds` redaction entirely                        | the scanner — `{"237600000771": ["refunds.failure_raw"]}`                                                           |
| M-D | Drop the `webhook_deliveries.payload_sha256` clear           | `an_erasure_mid_ladder_redelivers_the_redacted_body_instead_of_dead_lettering` — `Poisoned { … }`                   |

M-A is worth one note of its own: the delivered version of that test passed
under the delivered CHECK **and** under the corrected one, because it never
tried the NULL. A test that exercises one shape of a failure is not a test of
the constraint; it is a test of that shape.

## Verdicts the brief asked for

### Deviation 1 — `at_least_one_identifier` left strict, a new CHECK added

**Right, and the reasoning is right.** Proved directly: a row with all nine
columns set to `[redacted]` inserts cleanly, because the marker is not NULL,
so the brief's proposed relaxation (`anonymized_at IS NOT NULL OR (…)`) would
have bought no behaviour and lost a live constraint. Left strict, it refuses an
erasure that NULLed all three — verified: that insert fails on
`at_least_one_identifier`.

Is the new CHECK strictly stronger than what the brief asked for? Yes: the
brief's version constrained nothing about an anonymised row, this one pins
every one of nine columns. **But it was not as strong as its own comment
claimed** until F3 — it admitted every NULL. Stronger than the brief as
delivered; stronger than the brief _and_ true to its comment now.

### Deviation 2 — a `400`, not the `422` the brief named

**Right, and the brief was simply wrong.** `Category::http_status` has no
`422` in it at all — `InvalidRequest | Idempotency => 400` — and all 97
`invalid_param` call sites in `vpay-api` therefore answer `400`. A `422` on
this one parameter would have been the only one on the API.

### Default 1 — the list still returns erased customers

**Keep.** Filtering them out makes `GET /v1/customers` deny a `cus_…` that
`GET /v1/customers/{id}` answers `200` for, which is two routes disagreeing
about whether an object exists; and it makes `has_more` describe a different
set from the rows the cursor walks. Stripe filters deleted customers out of
its list, but Stripe's retrieve is a `404` — the two decisions go together and
vpay took the other one deliberately. It is stated as a choice in
[customers.md](../../flows/customers.md) with the shape a filter would take.
No change.

### Default 2 — `customer.deleted` carries no identifier

**Right as taken, and the branch's own note overstates the loss.** The body
carries the `cus_…` in `data.object.id`, and the event envelope's `created` is
the instant of the erasure. So the merchant _can_ tell exactly which record of
theirs to erase; what they cannot do is read the payer's name out of the event
a second time, which is the point. Nothing needs adding — in particular not
"the `cus_…` plus the erasure timestamp", because both are already there.

### Default 3 — `409` on update and on attach

**Right.** `Category::Conflict` is "the object's state forbids it", which is
exactly the fact. A `404` would contradict the `200` the retrieve answers; a
`400` would suggest the parameter was malformed. One message from one function
for both routes, so a merchant cannot tell which refused.

### (f) The merchant's own copy, and whether to add a `customer.redacted`

**Declined, and recorded with the reason.** The mechanism a merchant needs
already exists and already fires: `customer.deleted`, in the erasure's
transaction, on both branches, carrying the id and the instant. A second type
for one transition is what [webhooks.md](../../flows/webhooks.md)'s standing
rule refuses — a Stripe-shaped handler has no branch for it — and shipping one
would read as an enforcement vpay cannot perform.

What is actually missing is not a mechanism but a **contract**: whether a
merchant is obliged to act on `customer.deleted`, and within what window, is a
data processing agreement. That is a maintainer's decision and is surfaced,
not taken.

## Maintainer decisions surfaced

1. **F9's 24-hour window.** A response body stored a few milliseconds after an
   erasure re-introduces identifiers into `idempotency_keys.response_body`
   until `sweep_expired` removes the row. Closing it means a customer-shaped
   exception inside the generic idempotency store. Acceptable, or not?
2. **The DPA question above**: is a merchant obliged to erase their copy on
   `customer.deleted`, and in what window? vpay can state it; it cannot
   enforce it.
3. **Editing an unshipped migration.** `0041` was edited in place for F3 and
   its manifest line updated by hand, on the reading that
   `backends/migrations/README.md`'s rule is about a migration that has
   **shipped** and `0041` exists only on this unmerged branch. The alternative
   was a `0042` on the same branch dropping and re-adding a constraint `0041`
   had just added. If the maintainer reads the rule as absolute, the fix is
   the same three lines in a new file.

## What this review did not check

- The two SDKs' own test suites beyond `just ci`'s `test-web`; the body-
  asserting cases were read, not mutated. `verify-sdk-parity` still checks
  method and test **names** and not fields, as the branch's notes say.
- The dashboard and the demo stack. Neither renders a customer.
- Anything about a real rail. No HTTP call to one has ever been made.
- The delivered thirteen mutations were not re-run; the four above are the
  review's own and each one moves a line this branch wrote.
