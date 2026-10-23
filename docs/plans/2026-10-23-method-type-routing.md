# Method types, separate from rails: the slices

- **Date:** 2026-10-23
- **Request, verbatim.** The maintainer, on the Notch Pay gaps:
  - "#8 we need a way to differentiate mtn x notchpay and mtn x direct; same
    with orange and the future integrations."
  - "#10: A user should be proposed all methods, and he himself should choose
    orange or mtn or whatever, then we'll show him the correct rail. Simple".
- **Then, on how a rail is picked:** "I think routing should be though again.
  We should have a score of each payment solution for each checkout". And
  they confirmed the spellings `mtn_wallet` and `orange_wallet`.
- **What it builds on:**
  - [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
    D6, D7 and D10, accepted.
  - D12 and D13, which the maintainer's sentences confirm (ADR-0029's Status
    carries the dated note).
  - **D8 and D9 are being redesigned**, as a score per checkout, by
    [RFC-0008](../rfc/0008-payment-method-routing.md)'s 2026-10-23 revision.
    That revision was a proposal for the maintainer, not a decision. **Its
    open questions R8.1–R8.5 were answered on 2026-10-23** ("go with your
    defaults"); RFC-0008 § R8 records each answer and what is built.
  - [RFC-0012](../rfc/0012-notchpay-adapter.md) delivery step 8 is the Notch
    Pay half.
- **Status:** slice 1 is built in the change that carries this page,
  everything in it except the score's final formula. Slices 2 and 3 are not.

## The vocabulary

A **method type** is what the payer uses. A **rail** is the adapter that
processes it. One type per wallet network (D12):

| Method type     | Served by today            |
| --------------- | -------------------------- |
| `mtn_wallet`    | `mtn_momo`, `notchpay`     |
| `orange_wallet` | `orange_money`, `notchpay` |

`card` is reserved, unbuilt, for RFC-0010; `bank_transfer` is reserved for
D4's two bank shapes. Both are refused today with a message saying so, never
silently ignored.

## Slice 1: everything that does not depend on the formula (this change)

1. **`vpay_core::routing`.**
   - `MethodType` and `MethodTypes`, a set.
   - `route`: one pure function returning a **scored, explained ranking**.
     It names no rail (D9's test).
   - Eligibility is a hard filter (D7). A rail is excluded when it does not
     serve the type, is not enabled, does not settle the currency, or is not
     the pinned rail.
   - Every candidate is scored, the highest total wins, and ties go to the
     rail code.
   - **The score's formula is provisional (`provisional-1`):** one component,
     a configured weight, because that is the only input that exists today.
     Adding a component is a variant and a line.
2. **The port.**
   - `Capabilities::method_types`, a field beside `flow` rather than a new
     trait method, so `MeasuredAdapter` and every caller already carry it.
     The adapters declare: `mtn_momo` serves `mtn_wallet`, `orange_money`
     serves `orange_wallet`, and `notchpay` serves both. Boot refuses an
     enabled rail declaring none.
   - `ChargeRef::method_type` carries the payer's choice to the adapter.
   - D2 named RFC-0010 as `method_types`' first consumer; Notch Pay is,
     because it is the first rail to serve two types.
3. **Configuration:** optional `routing.weights`, each entry a rail with an
   optional method type and a weight from 0 to 1000.
   - With no entry, every rail weighs `100`, and the tie-break decides.
   - Boot refuses an unknown or reserved type, a rail that is not
     configured, a rail whose adapter does not serve the named type, a
     duplicate entry and a weight over 1000.
4. **The API.**
   - `payment_method_types` accepts method types beside rail codes. A rail
     code keeps exactly its meaning today, that rail pinned (D6), so no
     existing intent, SDK call or Stripe-compat case changes.
   - A confirm whose `payment_method_data[type]` is a method type is routed.
     `payment_method_options[<type>][rail]=<code>` on the **confirm** pins
     one candidate.
   - The payer's fields go under the type,
     `payment_method_data[mtn_wallet][msisdn]`.
5. **Persistence, migration `0053`.**
   - `charges.method_type`, set on every routed charge, and on a rail-code
     charge whose rail serves exactly one type.
   - `routing_decisions`, one row per routed charge, written in the
     transaction that opens the charge (D10). It holds the type, every rail
     considered with its exclusion reason or its score and breakdown, the
     chosen and pinned rails, the formula, and a configuration version.
   - Neither is on the `/v1` object (D10).
6. **Notch Pay** sends `cm.mtn` for `mtn_wallet` and `cm.orange` for
   `orange_wallet`. A rail-code charge keeps `settings.channel`
   (`cm.mobile`). This is RFC-0012 delivery step 8, proven against the stub
   only.
7. **The SDKs** spell `mtn_wallet` and `orange_wallet` in
   `payment_method_types` on a create. Neither can send a confirm by type or
   a pin: a dated gap in `docs/sdks/parity.md`, slice 2's.

The evidence is
[the verification page](../status/verification/2026-10-23-method-type-routing.md).

## Slice 2: the checkout (not built)

What the payer sees, per D13's `collapse`: one entry per method type, never
a rail and never a score. It needs:

- **the server half is in slice 1.** The checkout session's browser answer
  already carries one entry per method type, flowing as the rail the router
  ranks first and carrying that rail's payer fields, without naming it
  (`a_method_type_is_one_entry_flowing_as_the_routers_first_rail`). The page
  has to render a number field for a push rail and a redirect button for a
  redirect one, and `orange_wallet` can route to either (`orange_money` is
  `Redirect`, `notchpay` is `Push`);
- `frontends/apps/checkout` rendering method types when the intent lists
  them, and confirming with `payment_method_data[type]=<method type>`;
- `@vaam-apps/vpay-stripe-js` (embedded and popup), the Tauri plugin, and
  `examples/shop` creating intents with method types;
- the SDKs' confirm by type and the pin (`docs/sdks/parity.md`'s dated
  gap). A merchant's own server confirming `orange_wallet` has to pin today
  to know whether to send `return_url` or `msisdn`, because the payer fields
  depend on the rail; RFC-0008's open question R8.4 is the same problem seen
  from the checkout. _(Answered 2026-10-23: the confirm follows the rail the
  page showed while it is still eligible. Not built; RFC-0008 § R8.4.)_;
- D10's "at checkout render and again at confirm": the render-time ranking is
  advisory, and the confirm decides and records.

## Slice 3: the score itself (RFC-0008's revision, not built)

The components the revision proposes, once the maintainer chooses which are
wanted in v1:

- the estimated fee (D8's schedules);
- the recent success rate (RFC-0008 § 7's health);
- latency;
- amount fit;
- a merchant-level preference (RFC-0009).

Each is a component with its own source, its own coefficient and its own line
in the breakdown. The formula name moves past `provisional-1` when the first
one lands.

**Chosen 2026-10-23 (RFC-0008 § R8.1):** the estimated fee next, as a score
component; then the completion rate, as a reported measure the operator reads
to set a configured weight by hand, not a score input ("Go with (a)"). Health
is not a component (R8.3: it reports only), and
latency and amount fit were not chosen. The merchant-level preference waits on
RFC-0009 (R8.2: the operator sets weights until then). None of this is
built.

## Deferred, and why

- **`list` presentation** (D13's alternative): the maintainer's #10 asked for
  `collapse`.
- **`amount_limits`** (D2, D7): no adapter declares one, so no candidate is
  excluded by amount.
- **`requires`** (D15), and per-merchant enablement, which needs RFC-0009.
- **A pin stored on the intent** (`payment_method_options` on create). Slice
  1 takes the pin on the confirm only, so a checkout-session confirm cannot
  pin.
- **Cards**, until RFC-0010.
- **Stripe's `payment_method_details`** on a charge object, which would show
  a merchant the method type on `/v1`.
