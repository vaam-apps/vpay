# exp49 — an address in vpay is the formal address AND the GPS point

Notes for the commits added on top of the reviewed head `a4aa095` of
`claude/exp46-customer-address` ([PR #112](https://github.com/vaam-apps/vpay/pull/112)),
which was held open for this change. [opus.md](opus.md) is the draft's
account of the address and the erasure; [opus-review.md](opus-review.md) is
the sabotage review that fixed the partial-erasure CHECK, the event-body
redaction and the rail's verbatim failure text. Nothing here weakens any of
those three — the marker CHECK got **stronger** (nine columns to eleven), the
event-body redaction got **wider** (six address keys to eight), and the
`failure_raw` statements were not touched.

## The requirement

> "address in our system means both formal as well as GPS" — the maintainer,
> 2026-09-11.

Formal addressing is unreliable across the markets vpay serves. A street with
no sign, a quarter with no postcode, a building known by the shop on its
corner: a coordinate is how a place is actually found, and for many payers it
is the only half they can give. So the address is one object with two halves
rather than an address and a separate location, and it is replaced whole,
cleared whole and erased whole.

## The five decisions, and what each one cost

### 1. Integer microdegrees, never floating point

`address_latitude_microdeg` / `address_longitude_microdeg`, `BIGINT`, added to
migration `0041` itself rather than in a `0042` — the same reading the
review's F3 fix was made under: `0041` exists only on this unmerged branch and
has been applied to no database that outlives a test container, so
`backends/migrations/README.md`'s rule (a **shipped** migration is never
edited) is not the rule it is under. Its manifest line is updated by hand,
which is what `just migrations-manifest` refuses to do for the shipped case.

Two measurements decide the type, both from this repository and neither an
aesthetic preference:

- `Value::from_plain_json` routes every JSON number through `Number::as_i64()`
  and demotes anything else to `f64` (`docs/reference/vpay-db.md`,
  `cratestack-core-0.12.0/src/value.rs:95-106`). A decimal degree is a value
  CrateStack cannot carry without rounding it.
- the money layer's precedent is integer minor units with the scale named, and
  ADR-0007 denies float arithmetic workspace-wide.

A microdegree is ~0.11 m, two orders of magnitude finer than consumer GPS.
`BIGINT` rather than `INTEGER` — `INTEGER` would hold ±180,000,000, and the
column beside it would then be the one that had to explain why one pair is two
types.

### 2. Both or neither, and the ranges

Three CHECKs. `address_latitude_microdeg_range` and
`address_longitude_microdeg_range` are per axis, because the bounds differ and
"which axis, which bound" is the fact a merchant needs — the same reason the
six `address_*_length` bounds are separate.

`address_coordinates_are_both_or_neither` is
`(a IS NULL) = (b IS NULL)`, whose two sides are booleans that are never NULL
— which is the marker CHECK's own lesson read from the other direction.

It is **widened by `anonymized_at IS NOT NULL`**, exactly as `0041`'s two
shape CHECKs are. That buys no behaviour on its own (an erased row holds NULL
in both columns and satisfies the pair rule anyway); what it buys is that
`anonymized_customers_carry_the_marker` is the **only** constraint that can
fire on a row claiming to be erased — which is what lets the marker CHECK's
coverage of the coordinate be tested one column at a time. Without it Postgres
names the pair rule for such a row (constraint checks are evaluated in name
order) and that mutation becomes undetectable.

### 3. The unit is in the wire field name

`latitude_microdeg` / `longitude_microdeg`, integers. A field called
`latitude` would be read as degrees by every merchant who has used another
API, and the first `4.061` would be a value nothing here can store. Named for
its unit, the only sensible value to send is an integer, and
`checked_microdeg` refuses everything else with a sentence saying how to spell
4.061 rather than rounding it.

**A deliberate divergence from Stripe**, whose `address` has no coordinate at
all. Said so in `docs/flows/customers.md`, `docs/api/README.md`,
`vpay_api::model::AddressObject`, `vpay_db::CustomerAddress` and both SDKs'
types. A Stripe-shaped decoder meets two keys it does not know, which is
additive; code ported the other way loses them and should read it here.

**Alternatives considered and not taken** (the brief asked for these to be
surfaced rather than silently chosen):

- `latitude_e6` / `longitude_e6` — shorter, and the spelling some geo APIs
  use. Rejected because `microdeg` names the unit and `e6` names an encoding
  of it; a reader who has not seen the convention can guess the first.
- a nested `coordinates: { latitude_microdeg, longitude_microdeg }` object
  inside `address`. It would express the pair rule in the shape rather than in
  a CHECK, which is genuinely better — a `null` object is unambiguous where
  two nullable siblings are not. Not taken because it puts a third level of
  nesting into a form-encoded request
  (`address[coordinates][latitude_microdeg]=…`), and because the flat shape
  matches the eight columns one-to-one. **This is the one alternative worth a
  maintainer's second look.**
- a PostGIS `geography(Point)`. Rejected without much hesitation: it is an
  extension the deployment does not have, invisible to `cratestack migrate
baseline`, and its text form is a float.

### 4. The coordinates join the erasure, per column type

The nine text identifier columns carry `[redacted]`. The two coordinate
columns carry **NULL**, because there is no integer that is not a possible
place — a marker value would be a coordinate, somewhere real, on a row
claiming the payer is gone. `anonymized_customers_carry_the_marker` now
states both rules, using `IS NOT DISTINCT FROM` throughout (Postgres stores
`IS NOT DISTINCT FROM NULL` as `IS NULL`, which it is).

The argument that makes the nine carry a value rather than a NULL — "which
fields did this payer fill in?" is information about them — is not lost on the
two that cannot: `anonymized_at` is non-NULL on exactly the rows the
constraint applies to.

A payer's coordinates are the most sensitive field on this object. `CustomerRow`'s
hand-written `Debug` counts the coordinate as **one** component (the pair is
the value) and never prints it: a coordinate in a `tracing` field is a payer's
home in vpay's logs for the life of the log retention.

### 5. The stored event bodies, and the scanner's third limit

`REDACT_CUSTOMER_KEY`'s `address` arm replaces the whole key rather than
walking into it, so there is no path by which a nested coordinate survives —
the nested object is never read. The replacement is `redacted_address_json()`,
extracted so it can be asserted: it has to be key-for-key what
`AddressObject` renders, because a merchant reads a stored body back on a
replay or a redelivery and a key mismatch is a shape they meet only there.

`scan_for` reads `text`, `character varying` and `jsonb`. The coordinate
columns are `BIGINT` and outside it **in principle**, and widening it to
numerics would not help: every integer is a possible coordinate, so a hit
would mean nothing and a miss would mean nothing. So the third limit is stated
on `scan_for` beside its other two, and the two columns are asserted
**directly, by name, as NULL** — in the scanner's own case and in
`a_customer_with_payment_history_is_anonymised_rather_than_deleted`. What the
scan does cover is the **rendered** copies: a `jsonb` column cast to `TEXT`
renders a number as its digits, so the fixture's latitude is in the literal
list and is findable in `events.data` and `idempotency_keys.response_body`
before the erasure.

## Mutations

Every one run on this branch's own head, restored afterwards. The four the
brief named are M1–M4; M5–M7 are additions.

| #   | Mutation                                                                                                       | Caught by                                                                                                                                                                                                                                 |
| --- | -------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1  | drop the latitude conjunct from `anonymized_customers_carry_the_marker`                                        | `an_anonymised_customer_carries_the_marker_in_every_identifier_column` — "`address_latitude_microdeg` survived an anonymisation as 4061000"                                                                                               |
| M2  | drop `address_latitude_microdeg_range`                                                                         | `a_half_written_coordinate_is_refused_by_the_database` — "a value outside the axis is not a place: `PgQueryResult { rows_affected: 1 }`"                                                                                                  |
| M3  | leave the coordinate out of the event-body redaction (redact the six formal components and preserve the point) | `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table` — `{"4061777": ["events.data", "idempotency_keys.response_body"]}`                                                                                                     |
| M4a | drop `latitude_microdeg` from the Rust SDK's `AddressParams::to_form`                                          | `create_customer_sends_the_documented_body_and_decodes_the_object`                                                                                                                                                                        |
| M4b | drop `latitude_microdeg` from the Node SDK's `addressBody`                                                     | `customers.create: exact path, method, Idempotency-Key, and body`                                                                                                                                                                         |
| M5  | drop `address_coordinates_are_both_or_neither`                                                                 | `a_half_written_coordinate_is_refused_by_the_database`, **and** the multi-column CHECK inventory in `the_cstack_schema_drifts_…` — which is the only thing that could see it go, since a multi-column CHECK moves the drift count by zero |
| M6  | remove the API's pair refusal                                                                                  | `half_a_coordinate_is_refused_by_the_api_and_not_by_the_check`                                                                                                                                                                            |
| M7  | make `CustomerAddress::redacted` keep the payer's point                                                        | `an_erased_payers_coordinates_are_null_and_not_a_marker`                                                                                                                                                                                  |

M2 and M5 were first run on a host whose Docker could not start a container at
all (a 120 s testcontainers `StartupTimeout`, the inotify-saturation mode the
project memory records). A timeout is not a caught mutation, so both were
re-run with `--retries 3` after confirming the **unmutated** case passes on
retry, and the failures recorded above are assertion failures with the message
quoted.

## What was NOT done

- **No `0042`.** `0041` was edited in place, for the reason above. If the
  maintainer reads `backends/migrations/README.md`'s rule as absolute, the
  same three CHECKs and two columns move to a new file unchanged.
- **No reverse geocoding, no distance, no proximity search, no index on the
  coordinate.** vpay stores the point and echoes it back; nothing resolves it
  to anything. An index chosen before any query exists is one nobody can
  justify dropping — `0041`'s own argument against a partial index on
  `anonymized_at`.
- **No coordinate on any other object.** Checkout sessions, intents and
  invoices carry no address.
- **The dashboard and the demo stack render no customer**, so neither shows a
  coordinate; unchanged by this work.
- **The nested `coordinates` object** of decision 3 is surfaced, not taken.
