# issue #113 — the payer's GPS point: two decisions, prepared and **not taken**

> **Neither decision in [issue #113](https://github.com/vaam-apps/vpay/issues/113)
> is taken here, and nothing on this branch moves either one.** No page asks a
> payer for a location, no merchant reads back anything different from what
> they read back on 2026-09-11, and no capability changed. This page exists so
> that the maintainer can take both decisions from one reading instead of from
> a source tour.
>
> The rule this follows is AGENTS.md's, and the repository's own precedent: a
> privacy and consent question on a payment page is reserved for a human, and a
> defensible default is still a default somebody else picked. Every
> recommendation below says the word **recommendation** and none of them is
> implemented.

---

## 1. What is actually true today, with the tests that say so

### 1.1 The storage half is settled, and all three of its properties are proven

The issue says so, and it holds. Each property is asserted by a test that
fails when the property is removed — the seven mutations are in
[exp49-gps.md](../exp46-customer-address-notes/exp49-gps.md), and this section
names the cases rather than re-running them.

**Fixed-point microdegrees, never a float.** `BIGINT` in migration
`0041_customers-address-and-anonymisation.sql`, `Int?` on `model Customer` in
`schemas/vpay.cstack`, `Option<i64>` on `vpay_db::CustomerAddress` and on
`vpay_api::model::AddressObject`, and a single parse — `checked_microdeg` in
`vpay_api::v1::customers` — that refuses a decimal degree rather than rounding
it. There is **no microdegree↔decimal-degree conversion anywhere in the
repository**; the same integer travels wire → column → wire.

| Proven by                                                                          | Where                                                     |
| ---------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `a_coordinate_is_a_whole_number_of_microdegrees_and_never_a_degree`                | `vpay-api`, `src/v1/customers.rs`                         |
| `each_axis_is_bounded_by_its_own_range_and_the_bound_is_symmetric`                 | `vpay-api`, `src/v1/customers.rs`                         |
| `the_coordinate_bounds_are_the_ones_the_migration_enforces`                        | `vpay-api`, `src/v1/customers.rs` (reads `0041` off disk) |
| `an_address_may_be_a_point_with_no_street_at_all`                                  | `vpay-api`, `src/model.rs` (asserts `is_i64()`)           |
| `a_gps_point_round_trips_as_integers_and_is_replaced_and_cleared_with_the_address` | `backends/tests/integration/tests/customers.rs`           |
| `a_malformed_or_half_written_coordinate_is_a_400_and_not_the_databases_503`        | `backends/tests/integration/tests/customers.rs`           |
| `create_customer_sends_the_documented_body_and_decodes_the_object`                 | `sdks/rust/tests/resources.rs`                            |
| `customers.create: exact path, method, Idempotency-Key, and body`                  | `sdks/nodejs/src/client.test.ts`                          |

**Both or neither.** Refused at the API with a `400` naming `address`
(`validated_address`, so that a merchant gets a `400` and not the `503` a
`23514` would classify into), with
`address_coordinates_are_both_or_neither` in `0041` as the backstop for a
writer that never passes the API.

| Proven by                                                                   | Where                                                     |
| --------------------------------------------------------------------------- | --------------------------------------------------------- |
| `half_a_coordinate_is_refused_by_the_api_and_not_by_the_check`              | `vpay-api`, `src/v1/customers.rs`, both directions        |
| `a_malformed_or_half_written_coordinate_is_a_400_and_not_the_databases_503` | integration, over real HTTP, `COUNT(*) = 0` afterwards    |
| `a_half_written_coordinate_is_refused_by_the_database`                      | `postgres_smoke.rs`, asserting the constraint **by name** |
| the doctest on `CustomerAddress::coordinate_is_paired`                      | `vpay-db`, `src/customers.rs`                             |

**Erased with the rest of the identity.** `anonymize` sets both columns
`NULL` in the same statement as the nine text markers;
`anonymized_customers_carry_the_marker` refuses a row claiming to be erased
that kept either half; `redacted_address_json` rewrites the stored event and
idempotency bodies.

| Proven by                                                                 | Where                                                          |
| ------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `an_anonymised_customer_carries_the_marker_in_every_identifier_column`    | `postgres_smoke.rs`, coordinate conjuncts one column at a time |
| `an_erased_payers_coordinates_are_null_and_not_a_marker`                  | `vpay-api`, `src/model.rs`                                     |
| `an_erasure_projects_the_coordinate_to_absent_and_the_rest_to_the_marker` | `vpay-db`, `src/customers.rs`                                  |
| `the_redacted_address_body_is_the_shape_the_wire_renders`                 | `vpay-db`, `src/customers.rs`                                  |
| `a_customer_with_payment_history_is_anonymised_rather_than_deleted`       | integration; asserts both columns `NULL` by name               |
| `an_erasure_leaves_no_payer_identifier_in_any_column_of_any_table`        | integration; the two `BIGINT` columns asserted directly        |

**One thin spot, named rather than closed.** `erase_idle` — the twelve-month
retention sweep — shares `erase_in_tx`/`anonymize` with `DELETE`, so the same
statement clears the coordinate on both paths. But the sweep's own case,
`the_sweep_deletes_an_idle_unreferenced_customer_and_anonymises_a_referenced_one`,
builds its three fixtures with a `name` and **no address at all**, so no test
drives a coordinate through the sweep. The property holds transitively (same
statement, and `anonymized_customers_carry_the_marker` would raise a `23514`
and roll the sweep's transaction back), which is why this is a thin fixture
rather than an unproven property. **The fix is a few lines** — give that
case's referenced fixture an `address` with a point in its
`CreateCustomerParams`, and read the two columns back as `NULL` after the
sweep, exactly as `a_customer_with_payment_history_is_anonymised_rather_than_deleted`
already does for the `DELETE` path. It
was not taken here because it is a container-backed case in the integration
binary and this branch was explicitly capped on build size (see § 6).

Also unasserted, and smaller: nothing reads the two columns' Postgres **type**
directly the way `postgres_smoke.rs` reads `currencies.exponent`. A change to
`DOUBLE PRECISION` would be caught only as a `type differs` line in
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`.

### 1.2 Nothing collects a coordinate from a payer, and there is no route that could

`rg -i "geolocation|getCurrentPosition|watchPosition"` over the repository
returns **no hit in any source file** — not in app code, not in tests, not in
either SDK. Its only two hits are prose: this sentence, and the same claim on
[../../flows/customers/address-and-gps.md](../../flows/customers/address-and-gps.md).

_This read "**zero hits** … not in the docs" until the review of 2026-09-12,
and was false the moment it was written: the command it names returns the
sentence making the claim. The substance was right and the wording was
checkable and wrong, which is the combination this repository treats as worse
than no figure at all._

| Surface                                                              | What it collects from the person in front of it                                                                                                                                                                                                      |
| -------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hosted checkout, `frontends/apps/checkout` (`/c/[id]` and `/e/[id]`) | **One field: `msisdn`**, in `MsisdnForm` (`src/components/screens.tsx`). Beside it: a rail button, an opt-in `remember` checkbox storing `{msisdn, rail}` in `src/lib/memory.ts`, and a `locale` select. No name, no email, no address, no location. |
| The vanilla example, `examples/checkout-browser/index.html`          | The same one `msisdn` input.                                                                                                                                                                                                                         |
| Shop demo, `examples/shop`                                           | **One optional field: `email`** (`src/components/checkout-form.tsx`); the persisted `Order` carries `email String?` and nothing else about the buyer. It never calls `/v1/customers`.                                                                |
| The Orange redirect                                                  | Nothing — the payer's number is collected on Orange's own page.                                                                                                                                                                                      |

And the server half matters more than the page half: **the `/browser` surface
has no customer and no address on it at all.** `vpay_api::browser` is two
handlers over a checkout session; the payer's confirm body carries `msisdn` for
MTN and a rail code for Orange. There is no field a payer-facing page could put
a coordinate into, and no writer other than the merchant's own
`POST /v1/customers`. A coordinate in vpay today is, without exception, a value
**a merchant's server sent about their own payer**.

`/dash/v1` exposes no customer resource either (`vpay_api::dash` is one module,
`payment_intents`), so no dashboard screen renders a coordinate.

### 1.3 What a merchant reads back today

Full precision, every key, no gating.

`AddressObject`'s `latitude_microdeg` and `longitude_microdeg` are plain
`Option<i64>` with no `skip_serializing_if`, so both keys render on every
customer that has an address, as the exact integers stored. `retrieve` and
`list` both go row → `CustomerObject` with no projection between them.

Authorisation is **method-based only**: `required_scopes` in
`vpay_api::v1` answers `[payments:read, payments:write]` for a `GET` and
`[payments:write]` otherwise. There is no `expand` parameter, no field-level
scope and no per-merchant setting anywhere in the repository — vpay has **no
field-level authorisation of any kind**, on any resource. The only boundary is
tenancy: `get_for_merchant`, and a foreign `cus_…` is the same uniform `404` as
a missing one.

So the blast radius of one leaked `payments:read` key is `GET /v1/customers`,
which pages every customer that merchant owns, each with a point good to about
11 cm.

The one place vpay does not return the value is an **erased** customer, where
the six formal components carry `[redacted]` and the coordinate renders
`null` — because, as `0041` puts it, there is no integer that is not a possible
place, so a marker would be a coordinate somewhere real.

---

## 2. What "coarser" costs, in metres, at Cameroon's latitudes

The field is microdegrees, so "a coarser precision" is a number of trailing
digits dropped, and the question is what each one names on the ground. A
microdegree of **latitude** is 0.1106 m everywhere in Cameroon — the meridional
degree varies by less than 0.1% over the country. A microdegree of
**longitude** shrinks with the cosine of the latitude: 0.1113 m at the southern
tip (1.65°N), 0.1110 m at Douala (4.05°N), 0.1094 m at Maroua (10.60°N) and
0.1084 m at the northern tip (13.08°N).

The cell is the square a coarsened value names. Round-to-nearest puts the true
point within **half** a cell of the reported one; truncation puts it within a
whole cell, and always on the same side.

| Digits kept | Grid           | Cell, latitude | Cell, longitude at Douala | Cell, longitude far north | What the cell is the size of |
| ----------- | -------------- | -------------- | ------------------------- | ------------------------- | ---------------------------- |
| 6 (today)   | 1 µdeg         | **0.11 m**     | 0.11 m                    | 0.11 m                    | a spot on a doorstep         |
| 5           | 10 µdeg        | 1.11 m         | 1.11 m                    | 1.08 m                    | a person                     |
| 4           | 100 µdeg       | 11.1 m         | 11.1 m                    | 10.8 m                    | a building                   |
| 3           | 1 000 µdeg     | 111 m          | 111 m                     | 108 m                     | a compound, a block          |
| 2           | 10 000 µdeg    | 1.11 km        | 1.11 km                   | 1.08 km                   | a quarter                    |
| 1           | 100 000 µdeg   | 11.1 km        | 11.1 km                   | 10.8 km                   | a city                       |
| 0           | 1 000 000 µdeg | 111 km         | 111 km                    | 108 km                    | a region                     |

Three consequences worth having in front of the decision:

**Round, never truncate.** Cameroon is entirely north of the equator and
entirely east of Greenwich, so truncating toward zero would move _every_ payer
south-west by up to a full cell — a systematic skew a merchant plotting their
customers would see as a map that is subtly wrong rather than as data that is
deliberately coarse. Round-to-nearest is unbiased and halves the error.

**Coarse is not anonymous.** At 1.11 km a point still names a quarter, and a
quarter plus a phone number is still one person. How many people share a cell
is a function of population density, not of grid size: the same 2-dp cell that
holds thousands in Akwa holds one compound outside Bertoua. Coarsening reduces
harm; it does not make the field safe, and Option 2 in § 4 should not be read
as if it did.

**Delivery needs 4 digits and fraud needs 2.** That is the whole spread, and it
is the reason § 3's "what is it for?" cannot be answered after the fact: a
decision to keep 4 digits is a decision that the field is for finding a door,
and a decision to keep 2 is a decision that it is for knowing a city.

---

## 3. Decision 1 — does the checkout page ask a payer for their location?

### What is actually being asked

Not a front-end question. Adding a prompt to the page is a few lines; the
missing part is the server, and it is much larger. There is no payer-writable
address field on `/browser`, no address on a checkout session, and no writer
for a customer other than the merchant's own `POST /v1/customers`. A payer
supplying a coordinate would be **the first personal-data field a payer writes
into vpay's own storage** — `msisdn` today goes to the rail, not into a
`customers` row.

### The options

**Option A — vpay never asks. The coordinate stays merchant-supplied.**

- _Conversion:_ nothing. No prompt, no extra screen, no extra field.
- _Consent:_ nothing new. vpay is a **processor** of a value the merchant
  collected under the merchant's own relationship with the payer.
- _Regulatory:_ unchanged. The merchant is the controller of a field they
  wrote.
- _Implementation:_ zero.
- _What it costs:_ the requirement that "an address in vpay is the formal
  address **and** the GPS point" is satisfied only for merchants who already
  have a point. Every payer who arrives through vpay's own hosted checkout
  produces a customer with no coordinate, for ever.

**Option B — a browser permission prompt on the checkout page, optional.**

- _Conversion:_ the real cost, and vpay has **no number for it and cannot
  honestly invent one** — no page in this repository has taken a real payment.
  What can be stated without a measurement: the prompt is browser chrome, it
  cannot be styled or pre-warmed without a user gesture, and it appears on top
  of the screen at the moment the payer is deciding to pay. And one structural
  fact that is easy to miss: **the hosted checkout is served from vpay's
  origin**, so a permission denial is sticky per-origin and one payer denying
  it once on one merchant's checkout silences the prompt for **every**
  merchant's checkout on that device.
- _Consent:_ a browser permission is not consent. It records that the payer let
  the browser read the sensor; it records nothing about vpay storing the value,
  sending it to a merchant, or keeping it twelve months. Consent needs a named
  purpose and a named recipient in the page's own words, so Option B is really
  "a prompt **and** a sentence", and the sentence is the part with legal weight.
- _Regulatory:_ the largest hidden cost. Asking a payer directly makes vpay the
  **controller** of the most sensitive field it holds, rather than a processor
  of the merchant's. That is a different legal posture — a different notice, a
  different lawful basis, a different party answering an access request — and
  it is a maintainer and counsel question, not an engineering one.
- _Implementation:_ page UI, a payer-writable field on `/browser`, a write path
  from the browser surface into `customers`, the address on the session object,
  the events, and both SDKs under the parity rule.

**Option C — the merchant asks for it, per session, with a stated purpose.**

A checkout-session parameter (`collect_location: off | optional | required`,
defaulting to `off`) and a merchant-supplied purpose string the page shows
above the prompt.

- _Conversion:_ paid only by the merchants who need it.
- _Consent:_ the purpose string is what turns a permission into a consent, and
  the **merchant** names it because the merchant is the controller.
- _Regulatory:_ keeps vpay a processor acting on the controller's documented
  instruction, which is the shape that survives a data processing agreement.
- _Implementation:_ the largest of the four, and it carries Option B's whole
  server-side cost plus a session parameter and its SDK parity rows.

**Option D — no sensor, no prompt: a map pin or a typed point, optional.**

- _Conversion:_ no permission prompt and no sticky denial, but a map on a
  payment page is a third-party tile origin — a CSP entry, a vendor, and a
  request that says a payer is on a checkout page. Typing a coordinate is not
  something a payer does.

### The part of the question that decides it: what is it for?

The issue offers three purposes, and **each one wants a different design.
Only one of them is the design that exists.**

- **Delivery** needs a _destination_, not a _position_, and needs about 4
  digits. A payer paying from work whose parcel goes home is the ordinary case,
  and a sensor read at checkout returns the wrong place with full confidence.
  A destination belongs in the merchant's own order flow, where the payer can
  correct it.
- **A fraud signal** needs 2 digits, needs to be collected **per payment**
  rather than stored on a customer, and needs to be read by **vpay** and not
  handed to the merchant. The current field is the opposite of all three:
  stored on the customer, returned to the merchant, and read by nothing —
  there is no risk engine in this repository and no code anywhere resolves a
  coordinate to anything.
- **A merchant record** is the purpose the current shape already fits exactly.
  The merchant wrote it, the merchant reads it, vpay stores and erases it.

### Recommendation (a recommendation, not a decision)

**Option A today; Option C if and when a merchant asks for it, and never
Option B on its own.** The reasons, in the order they weigh:

1. The field's current shape _is_ "merchant record", and that purpose needs no
   prompt at all.
2. A prompt makes vpay the controller of a payer's position. That is a change
   of legal posture, not a feature, and nobody has asked for it.
3. "The page asks" is not a front-end change — it needs the first
   payer-writable personal-data field in the system — so it should not be
   started before the purpose is chosen. Choosing the purpose first also
   chooses the precision, and § 2 shows those differ by a factor of a hundred.

---

## 4. Decision 2 — does a merchant read back the full coordinate?

### The options

**Option 1 — no change: full precision to the merchant that owns the customer.**

- _Gain:_ the round-trip is lossless and the rule is one sentence. A merchant
  reads back exactly the integer they sent, which is what
  `a_gps_point_round_trips_as_integers_and_is_replaced_and_cleared_with_the_address`
  currently asserts.
- _Cost:_ one leaked `payments:read` key is a bulk export of where a merchant's
  payers live, to about 11 cm, through `GET /v1/customers`.
- _Regulatory:_ the merchant is the controller and already holds the value —
  they sent it. Echoing it is not a new disclosure.

**Option 2 — coarsen by default; full precision behind a gate.**

- _Gain:_ the leaked-key blast radius drops from "where this payer lives" to
  "which quarter", at the grid § 2 prices.
- _Cost, and this is the sharp one:_ **read-modify-write silently destroys the
  stored point.** An update replaces the address whole — that is a recorded
  decision, and a good one — so a merchant doing the ordinary `GET`, change one
  field, `POST` the object back would write the _coarsened_ coordinate over the
  exact one, with no error and no way to recover it. Three ways out, all with a
  price: refuse a write whose coordinate is exactly a coarsened read (fragile,
  and wrong for a merchant who really did mean that point); give the coarse
  value a different key so it cannot be written back (a ninth address key, and
  a divergence from the eight-column mapping); or accept the loss and say so
  (which makes a documented footgun out of the most sensitive field).
- _Cost, second:_ the gate itself. A field-level scope such as
  `customers:location:read` would be the cleanest shape and would also be the
  **first** field-level authorisation anywhere in vpay — `required_scopes` is
  method-based on every resource.
- _Cost, third:_ § 2's second consequence. Coarse is not anonymous, so this
  buys a reduction in harm and not an exemption from anything.

**Option 3 — full precision on `GET /v1/customers/{id}`, coarse or absent on
the list.**

- _Gain:_ cheap, and it targets the actual bulk-export surface: the list is
  what turns one key into every payer, and a retrieve needs the id.
- _Cost:_ the object would have two shapes, which is what Stripe-shaped clients
  handle worst, and `the_customer_object_is_the_documented_nine_keys` is a
  tripwire built on the object having one.

**Option 4 — a per-merchant setting for the precision returned.**

- _Gain:_ the merchant decides, which matches who the controller is.
- _Cost:_ a new merchant-level setting, a surface vpay has almost none of, and
  it does not remove Option 2's read-modify-write problem for whoever turns it
  on. (It is not an ADR-0003 violation: per-merchant precision is data in a
  row, not a profile selecting a code path.)

### Recommendation (a recommendation, not a decision)

**Option 1 — for as long as, and only for as long as, decision 1 stays at
Option A.** The reasons:

1. vpay is echoing a value its own author sent. Withholding precision from the
   party that supplied it protects nobody from the party that already has it,
   and it buys that non-protection with a silent data-loss failure mode
   (read-modify-write) on the most sensitive field on the object.
2. The exposure Option 2 addresses is real but its shape is a credential
   problem, and there are cheaper places to spend on it than a coarsening rule
   — the list endpoint, key scoping, and whatever rate limiting the bulk-export
   case eventually gets.
3. Everything that is _not_ the owning merchant already sees nothing: no
   dashboard screen, no `/dash/v1` route, no third party, and
   `CustomerObject`/`AddressObject` refuse to print the value even into a log
   (issue #70, and now `vpay-db`'s write path too — § 6).

### The coupling, which runs one way

**Decision 2's answer depends on decision 1's, and not the other way round.**
While vpay never collects the point, returning it is an _echo_ and Option 1 is
easy. The moment a payer supplies it — Option B or C — vpay collected it, the
merchant did not, and handing over 11 cm becomes a _disclosure_ by the party
that asked for it. At that moment Option 1 stops being defensible on the
argument above, and the precision returned becomes part of the consent the
payer gave: "so this shop can deliver to me" is a 4-digit consent, not a
6-digit one.

So: **taking decision 1 as Option B or C reopens decision 2 automatically.**
Taking decision 1 as Option A closes decision 2 at Option 1 for as long as it
holds. Deciding 2 first, in either direction, decides nothing durable.

---

## 5. What a "yes" would need before it could be built

Listed so that neither answer looks cheaper than it is. None of this exists.

- A payer-writable field on `/browser`, and the first write path from the
  browser surface into `customers`.
- An address on the checkout session object, or a `customer` write that a
  session's credential is allowed to make.
- Consent text in both locales the checkout ships (`en`, `fr`), reviewed by
  someone who is not an engineer, and a record of what was consented to — a
  permission grant in the browser is not one.
- The event surface: `customer.created`/`customer.updated` already carry the
  address to the merchant's endpoint, so a payer-supplied point would start
  crossing that wire the day it is collected, before anyone decides it should.
- A precision decision, because § 2 shows delivery and fraud differ by a factor
  of a hundred and the field cannot serve both.
- Both SDKs, under the parity rule, or a dated gap in
  [../../sdks/parity.md](../../sdks/parity.md).

---

## 6. What this branch did do, and what it judged gated

**Done — one defect, and it is gated on neither decision.** `vpay-db`'s
customer **write path** printed everything the read path is careful not to.
`CustomerRow` has had a hand-written redacting `Debug` since 2026-09-10, and
`vpay_api::model::{CustomerObject, AddressObject}` gained one on 2026-09-11
([issue #70](https://github.com/vaam-apps/vpay/issues/70)) — but
`CustomerAddress`, `NewCustomer` and `CustomerPatch` still **derived** `Debug`,
so a `{:?}` on any of them wrote the payer's name, email, phone, street and GPS
point out in full. Those are exactly the values `insert_in_tx` and
`update_in_tx` hold in the frame that runs the statement, which is the frame an
`anyhow` chain or a `tracing` event carries when a CHECK fires or a pool times
out. `CustomerRow`'s own guard was also load-bearing on a type that had none:
it counted the address components itself over `self.address.*`, so anything
else holding the same `CustomerAddress` printed it whole.

All three now have hand-written impls, the component count lives in exactly one
place per crate (`CustomerAddress`'s own `Debug`, which `CustomerRow` now
delegates to), and `no_customer_type_ever_prints_a_payers_identifiers_street_or_gps_point` in
`vpay-db` asserts all four at once.

**Extended by the review of 2026-09-12, because the first pass stopped one
layer short.** The sentence above says this is "the hole #70 closed one layer
up, left open on the way **in**" — and the way in does not begin at `vpay-db`.
`vpay_api::v1::customers`' own request types still **derived** `Debug`:
`CreateParams` and `UpdateParams` (the merchant's body, with `name`, `email`,
`phone`), `AddressParam`/`AddressParams`, and `ValidCreate`. `AddressParams` is
the worst of them, and it is the one a `vpay-db`-only fix cannot reach: its
`latitude_microdeg` and `longitude_microdeg` are `Option<String>`, so a derived
`Debug` printed a payer's position **as the merchant spelled it**, before
`checked_microdeg` had parsed it — and therefore on exactly the rejection paths
most likely to be logged. All five are hand-written now, `ValidCreate`
delegates its address to `CustomerAddress`, and
`no_customer_request_type_ever_prints_a_payers_identifiers_street_or_gps_point`
in `vpay-api` asserts them the same way, in both directions. The exposure was
**latent rather than live** — these types are private to the module and nothing
formats one today — which is the same standing as the `NewCustomer` half, and
the reason both are worth closing rather than neither. It is negative **and** positive: a `Debug`
printing nothing would pass every substring search and fails the counts. The
five mutations, the commands that were run and — as importantly — the ones that
were **not** are in
[../../status/verification/2026-09-12-customer-debug-redaction.md](../../status/verification/2026-09-12-customer-debug-redaction.md).

Why it is not a decision: whatever the maintainer answers, a payer's
coordinate must not be in vpay's own logs for the life of the log retention.
It changes no wire contract, no merchant-visible behaviour and no payer
surface.

**Judged gated, and not done.**

- **Any capture of a location.** Not a line of it, including the "just wire the
  page up behind a flag" version — a flag defaulting to `off` still ships the
  prompt, the consent text and the field, which is the decision made in
  advance.
- **Any change to what a merchant reads back**, including the cheap-looking
  Option 3 (coarse on the list only). Every one of them is decision 2.
- **A precision constant, a rounding helper, or a `coarsen` function**, even
  unused. A default in the code is a default taken.
- **`collect_location` on the checkout session**, in either SDK or in the wire
  contract. Declaring the parameter is answering question 1.

**Not gated, but not done either, and honestly so:** the sweep's coordinate
fixture in § 1.1. It is three lines in a container-backed case in the
integration binary, and this branch was capped on build size because other
agents were building on the same host; adding a test that was never run is
worse than naming it. It is the one piece of § 1 that a next change should
pick up.
