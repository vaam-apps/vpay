# Customers — the address is the formal address **and** the GPS point

_Split out of [docs/flows/customers.md](../customers.md) on 2026-09-11 by exp57, which broke a 888-line flow document into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

### The address is the formal address **and** the GPS point (2026-09-11)

> "address in our system means both formal as well as GPS" — the maintainer,
> 2026-09-11.

That sentence is the whole of this section. Formal addressing is unreliable
across the markets vpay serves: a street with no sign, a quarter with no
postcode, a building known by the shop on its corner. A coordinate is how a
place is actually found. So an address in vpay is **one object with two
halves**, not an address plus a separate location — it is replaced whole,
cleared whole and erased whole, both halves together.

`address` is **one nested object with eight components**, every one nullable
and every one rendered, or `null` when the customer has no address at all:

|                                                             |                             |
| ----------------------------------------------------------- | --------------------------- |
| `line1`, `line2`, `city`, `state`, `postal_code`, `country` | Stripe's six, as strings    |
| `latitude_microdeg`, `longitude_microdeg`                   | vpay's own, as **integers** |

**The two coordinate keys are a deliberate divergence from Stripe**, whose
`address` has no coordinate at all. A merchant porting Stripe code to vpay
gains two keys, which is additive and safe; one porting the other way loses
them, and should read that here rather than discover it. It is stated in
[../api/README.md](../../api/README.md), on `vpay_api::model::AddressObject` and
in both SDKs' types.

Behind the object are eight columns and not one `JSONB` one, for one reason
stated twice: a JSONB column would be invisible to `cratestack migrate
baseline` in both directions **and** could not be declared on `model Customer`
without `Value::from_plain_json`'s number demotion
(`../reference/vpay-db.md`). The second half of that is also the first reason
the coordinate is an integer.

#### Microdegrees, and why there is no float anywhere

A microdegree is one millionth of a degree. 4.061°N is `4061000`; the unit is
in the field name, and that naming is load-bearing rather than pedantic — a
field called `latitude` would be read as degrees by every merchant who has
used another API, and the first `4.061` would be a value nothing in this stack
can store.

Two measurements decide it, both from this repository:

- `Value::from_plain_json` routes every JSON number through `Number::as_i64()`
  and demotes anything else to `f64` (`../reference/vpay-db.md`). A decimal
  degree is a value CrateStack cannot carry without rounding it.
- the money layer's own precedent is integer minor units with the scale named
  ([money.md](../money.md)), and ADR-0007 denies float arithmetic workspace-wide.
  A coordinate is the same kind of quantity: an exact count of a fixed unit,
  not a measurement for each layer to re-round.

The resolution is about **0.11 m** — two orders of magnitude finer than
consumer GPS — so the unit costs no precision anybody can observe. The columns
are `BIGINT`: longitude runs to ±180,000,000, which `INTEGER` would hold, and
a pair of columns with two different types would be the thing that needed
explaining.

`4.061` is a `400` naming `address`, with a sentence that says to send
`4061000`. It is refused rather than rounded, because a payer's position
silently rounded by an API that did not say so is exactly the failure this
shape is chosen to avoid.

#### Both or neither

A latitude on its own is a line right round the planet. Half a coordinate is
**worse** than none, because whoever "completes" it later produces a plausible
wrong place. So the pair is the value: sending one half without the other is a
`400` naming `address`, and `address_coordinates_are_both_or_neither` in
migration `0041` is the backstop for a writer that never passes the API. The
ranges are the definition of the units — ±90,000,000 and ±180,000,000 — and
they are symmetric: the South Pole is as legal as the North.

Five rules, and none of them is obvious from the shape:

**`country` is ISO 3166-1 alpha-2, upper-cased on the way in.** `cm` and `CM`
are one country, and storing them as typed would leave vpay holding two
spellings — the same wire contract `phone`'s canonicalisation is, for the same
reason. `CMR`, `237` and `Cameroon` are a `400` naming `address`. The _shape_
is checked and the code is not resolved against any list: the list changes
(South Sudan in 2011, the Netherlands Antilles out in 2010), nothing in vpay
resolves a country code to anything, and a CHECK that had to be migrated
whenever the world did would refuse a merchant's perfectly real address until
somebody shipped a release.

**An update replaces the address whole; it never merges components — and the
coordinate is one of them.** A request naming `address[line1]` and not
`address[city]` clears the city, and a request naming a street and no
coordinate clears the point. That is a decision rather than an omission, and
the argument is about failure modes: a merchant correcting a payer's street
who left `city` out meant "this is the address", and a component-wise merge
would keep the old city beside the new street — an address that was never
anybody's, assembled by vpay out of two requests, discovered by whoever
eventually posts something to it. Replacement fails visibly on the next read.

The coordinate makes that argument sharper rather than complicating it: a
merge would leave a payer's **previous position** attached to somebody else's
street, which is a plausible wrong place and the one wrong answer a merchant
would never see. One flag over all eight columns in
`vpay_db::customers::update_in_tx` is what makes it inexpressible.

**`address=` clears it**, exactly as `name=` clears a name; an absent key
leaves it alone. Both SDKs carry the three states (`Option<Option<…>>`,
`AddressParams | null | undefined`) and both prove them by asserting the
**body**.

---

## Two questions this shape does not answer, and neither does anybody else yet (2026-09-12)

_Added by [issue #113](https://github.com/vaam-apps/vpay/issues/113). The
section above is the **storage** half and it is settled; these two are not, and
they are **the maintainer's to take**. Nothing has been built in either
direction, and that is on purpose — a privacy and consent question on a payment
page is not a default an agent gets to pick._

1. **Does the hosted checkout page ask a payer for their location at all?**
   Whether it is optional, and what it is for: delivery, a fraud signal, or a
   merchant record.
2. **Does a merchant read back the coordinate at full precision, or a coarser
   one by default?**

The options for each, what each one costs in conversion, consent, regulatory
exposure and implementation, what "coarser" is worth in metres at Cameroon's
latitudes, and a recommendation for each clearly labelled as a recommendation,
are written up in one place:
**[../../plans/issue-113-notes/decision.md](../../plans/issue-113-notes/decision.md)**.

Three facts from it that this page should carry on its own, because they are
about the shape above rather than about the decision:

- **Nothing anywhere captures a coordinate from a payer, and no route could.**
  `navigator.geolocation` appears nowhere in this repository; the hosted
  checkout collects one field (`msisdn`), the shop demo one optional `email`;
  and the `/browser` surface a payer's page talks to has **no customer and no
  address on it at all**. Every coordinate in vpay is a value a merchant's
  server sent about their own payer.
- **A merchant reads it back exactly as they sent it**, to any token carrying
  `payments:read` that owns the customer, on both the retrieve and the list.
  There is no `expand`, no field-level scope and no per-merchant setting —
  `required_scopes` is method-based on every resource in vpay, so there is no
  field-level authorisation anywhere to hang one on.
- **The two questions are coupled, one way round.** While vpay never collects
  the point, returning it to the merchant who wrote it is an echo. If a payer
  ever supplies it, the same response becomes a disclosure by the party that
  asked — so answering the first question with "yes" reopens the second
  automatically, and answering the second first decides nothing durable.
