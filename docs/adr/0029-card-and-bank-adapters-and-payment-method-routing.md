# ADR-0029: Card and bank providers as adapters, and payment-method routing

- **Status:** Accepted in part.
  - **D1–D11** are [RFC-0004](../rfc/0004-billing-on-top-of-invoices.md) § 7
    and [RFC-0008](../rfc/0008-payment-method-routing.md) as merged on
    2026-09-23 (#244) and amended the same day (one adapter per provider; the
    two criterion corrections from the
    [2026-09-23 evaluation](../plans/2026-09-23-card-provider-evaluation.md)).
    The maintainer wrote on 2026-10-16 "RFC-0004 and -0008 should be
    acceptable right? It makes sense to me", and on 2026-10-17 directed this
    ADR be written ("now write the ADR accepting RFC-0004 § 7 and RFC-0008").
    D3's decoupling of stored methods from subscriptions and D5's sequencing
    were put to the maintainer in the same 2026-10-16 exchange.
  - **D12–D17** answer the two RFCs' open questions. They were proposed by
    the agent drafting this ADR on 2026-10-16 and **need the maintainer's
    confirmation**. D12 in particular refuses a spelling the maintainer
    proposed that day, for reasons recorded there; it is the first thing to
    read if this ADR is to be reversed anywhere.
  - **D1's third hard rule amended 2026-10-20 by
    [ADR-0031](0031-a-status-read-authenticated-by-a-server-side-key.md):**
    a status read authenticated by a key the provider calls public is
    admissible when that key is held server-side, every secret the provider
    has is sent on every call, boot refuses a configuration without that
    secret, and the exposure is documented and logged at boot. Rules 1, 2
    and 4, and D2–D17, are unchanged. The rule as written below is the text
    ADR-0031 replaced.
  - **D12 and D13 confirmed by the maintainer on 2026-10-23**, in two
    sentences on the Notch Pay rail's gaps:
    - "we need a way to differentiate mtn x notchpay and mtn x direct; same
      with orange and the future integrations";
    - "A user should be proposed all methods, and he himself should choose
      orange or mtn or whatever, then we'll show him the correct rail.
      Simple".

    That confirms D12's one method type per wallet network, and D13's
    `collapse`: the payer picks the method and the router picks the rail.
    The same day the maintainer confirmed the spellings `mtn_wallet` and
    `orange_wallet`. D14–D16 are still unconfirmed.

  - **D8 and D9 reopened by the maintainer on 2026-10-23:** "I think routing
    should be though again. We should have a score of each payment solution
    for each checkout". [RFC-0008](../rfc/0008-payment-method-routing.md)'s
    2026-10-23 revision proposes a score per checkout. Its amendments to D8,
    D9 and D11 are proposals until the maintainer accepts them; the body
    below is unchanged.

    **What is built meanwhile** (the change that carries this note):
    - D6;
    - D7's capability-free filters;
    - D10's decision row;
    - a router that scores with one provisional component, a configured
      weight.

    [plans/2026-10-23-method-type-routing.md](../plans/2026-10-23-method-type-routing.md)
    has the slices.

  - **RFC-0008's R8 questions answered by the maintainer on 2026-10-23**
    ("go with your defaults"; on the score, "first go with your defaults.
    Just document everything"). The answers: the estimated fee is the next
    component and the completion rate the one after; the operator alone sets
    weights until RFC-0009; health reports and never moves traffic; the
    confirm follows the rail the page showed while it is eligible; no
    exploration; ties go to the lower rail code. What that settles here:
    - **D9:** "score descending, then rail code" is the order, as built.
    - **D8:** fees become one component of the score, not the sort key. No
      fee schedule exists yet.
    - **D11:** health moves no traffic, and neither does the completion
      rate: it is reported, and the operator sets a configured weight from
      it by hand ("Go with (a)", 2026-10-23;
      [RFC-0008](../rfc/0008-payment-method-routing.md) § R8.1).
    - D14–D16 are still unconfirmed. The body below is unchanged; RFC-0008
      § R7 and § R8 carry the detail.
- **Implementation:** none. This ADR records decisions, not capability.
  Nothing it decides exists in code on the day it is written: the port has no
  `method_types`, no `amount_limits`, no `supports_off_session`, no
  `charge_stored`, no `ProviderFlow::Instructions`; there is no `pm_…`
  object, no `routing_decisions` table, and `payment_method_types` holds rail
  codes. The first adapters are designed in [RFC-0010](../rfc/0010-stripe-adapter.md)
  and [RFC-0011](../rfc/0011-paypal-adapter.md), and
  [RFC-0009](../rfc/0009-provider-accounts-per-merchant.md) is their
  prerequisite on any deployment serving more than one merchant. The
  `docs/status/` rows, flow pages and verification pages of the pull
  requests that build these are the statement of what is built.
- **Date:** 2026-10-17
- **Deciders:** the vpay maintainer, for D1–D11 by accepting the two RFCs;
  D12–D17 by confirming them as proposed, once they do.
- **Implements:** RFC-0004 § 7 only. §§ 1–4 and 8–13 of RFC-0004 stay
  **Draft**; their open questions need an accountant and counsel, not this
  ADR. RFC-0008 in whole, except its § 7 (health), which it defers itself.
  [RFC-0006](../rfc/0006-card-processing-without-a-psp.md) and
  [RFC-0007](../rfc/0007-bank-transfer-reconciliation.md) are **not**
  accepted here.
- **Number checked at branch time, not assumed:** `git ls-tree -r
origin/master docs/adr` at `44c5bf5` returns `0001`…`0028`, 28 files for
  28 numbers. The open pull requests on 2026-10-17 are #274 (a dashboard
  nav fix) and #275 (this branch, `docs/rfc` and `docs/status` only); neither
  touches `docs/adr/`.

## Context

vpay carries two mobile-money rails and no card rail, no wallet beyond those
two, and no bank. RFC-0004 § 7 says how every further provider joins: as one
more adapter behind the port, admitted by four hard rules and described by
capabilities, so that the operator — not the project — chooses which their
merchants have contracts with. RFC-0008 says what happens once two adapters
serve the same payment method: the cheapest capable one first, decided from
configuration, recorded per charge, and never retried across rails.

Both were merged as Drafts on 2026-09-23. Since then the provider research of
2026-10-16 found that the rails most worth building first are the ones with
public specifications — Stripe and PayPal, neither of which serves a Cameroon
merchant — and that a Cameroon card rail waits on providers' written answers
rather than on engineering. RFC-0010 and RFC-0011 are written against § 7 and
RFC-0008 and cannot be built against a Draft. This ADR is what turns them
into decisions, and it decides the open questions both RFCs left, so that the
first adapter does not decide them by default.

## Decision

### RFC-0004 § 7 — one adapter per provider

**D1. Every provider that meets the four hard rules gets its own adapter, and
no provider is "the" card provider.** The operator enables the adapters their
merchants have contracts with. The rules, as corrected on 2026-09-23:

1. **vpay never holds the money.** Funds land in the **merchant's own**
   account with the provider, withdrawable to the merchant's bank or mobile
   money. A balance the merchant holds at the provider does not cross
   RFC-0001's line; a provider that credits **vpay** is aggregation and is
   out.
2. **No card number reaches `vpay-server`.** A card is entered on the
   provider's hosted page or hosted fields, where 3-D Secure also happens.
   For the payer this is the existing `Redirect` flow, and merchants stay on
   PCI SAQ A. [ADR-0021](0021-flutter-checkout-plugin.md)'s refusal of a
   native card field stands. RFC-0006's separately deployed vault is the one
   place this rule could ever bend, and RFC-0006 is not accepted here.
3. **A secret-authenticated status query.** Callbacks are hints, and the
   status read is the only thing that moves money. A provider whose only
   signal is a webhook, or whose status read accepts a publishable key, gets
   no adapter.
4. **A stub-able contract.** A sandbox reachable from CI, or a specification
   detailed enough to write WireMock mappings from, because every adapter
   joins the one conformance suite.

Whether a provider onboards a given country, settles a given currency or
holds a given licence is a property of the operator's configuration, not of
the adapter. An adapter for a provider that serves another market is a
legitimate contribution.

**D2. The port grows these capabilities, each off or `Unsupported` by
default, and each lands with its first consumer, never ahead of one:**

| Capability                               | What it gates                                                          | First consumer           |
| ---------------------------------------- | ---------------------------------------------------------------------- | ------------------------ |
| `method_types`                           | RFC-0008's candidate set (D6)                                          | RFC-0010                 |
| `amount_limits`                          | a provider's per-transaction minimum and maximum (D7)                  | RFC-0010 (Stripe's 0.50) |
| `supports_off_session` + `charge_stored` | `charge_automatically` and every merchant-initiated charge (D3)        | RFC-0010 step 7          |
| `ProviderFlow::Instructions`             | PSP-mediated bank transfer and virtual accounts (D4)                   | the first such adapter   |
| `RefundDestination::Origin`, driven      | a refund to the instrument that paid; the two debts RFC-0010 § 7 names | RFC-0010                 |

A capability that exists in the port with no adapter declaring it is a claim
with no test behind it, which is why none is added by this ADR.

**D3. Stored payment methods (`pm_…`) hold only what the provider returns,
and are bound to the rail that minted them.**

- the provider's token, sealed at rest as a rail credential is;
- `brand`, `last4`, `exp_month`, `exp_year`, and nothing else a payer typed.
- A method is saved when a hosted payment completes with Stripe's
  `setup_future_usage=off_session`, on a rail that declares
  `supports_off_session`. vpay accepts and ignores that parameter today; on
  every other rail it stays ignored.
- A stored method is charged only through the rail that minted it. A
  provider's token is worthless to any other, so the router (D7) never
  routes such a charge away.
- `GET /v1/customers/{id}/payment_methods` and
  `POST /v1/payment_methods/{id}/detach` are the Stripe spellings.
- **Decoupled from subscriptions.** RFC-0004's delivery table made stored
  methods (step G) depend on subscriptions (step C). A saved card is what
  one-click repeat payment needs, and one-click repeat payment needs no
  schedule. Step G depends on step F and on a rail that issues tokens, and
  on nothing else.

**D4. Bank payments.** A PSP-mediated transfer — a virtual account or a
transfer reference the payer pays from their bank, which the provider then
reports — is `ProviderFlow::Instructions`, rendered as Stripe's
`next_action.display_bank_transfer_instructions`. A transfer into the
merchant's own account with no provider is RFC-0007's, not accepted here. A
transfer the merchant simply tells vpay about is [ADR-0024](0024-customer-filters-and-manual-payments.md)'s,
built. The three are three things, and the name `bank_transfer` is reserved
for the first two by D12.

**D5. Sequencing, answering RFC-0004's open question 6.** Adapters are
additive, so the question is order and not selection. The first two are the
providers with public specifications, Stripe (RFC-0010) and PayPal
(RFC-0011), in that order; neither serves a Cameroon merchant, and both are
said to be what they are. A Cameroon card rail — Flutterwave Cameroon or
Tranzak on the 2026-10-16 research, a bank's hosted page if one appears —
waits on the written answers the evaluation page lists, not on engineering.
RFC-0009 lands before any adapter is offered on a deployment with more than
one merchant.

### RFC-0008 — the cheapest capable rail first

**D6. A method type is separate from a rail, and `payment_method_types`
accepts both.** A method type is what the payer uses; a rail is the adapter
that processes it; each adapter declares the method types it serves. A
merchant who must pin a rail says so with
`payment_method_options[<type>][rail]=<code>`, a vpay-native option. **A
rail code in `payment_method_types` keeps meaning exactly what it means
today** — that method, pinned to that rail — so no existing intent, SDK call
or Stripe-compat case changes until a merchant sends a method type.

**D7. The candidate set** for one intent and one method type is the rails
that serve the type, are enabled for the deployment and the merchant, admit
the intent's currency, admit its amount (`amount_limits`), and declare every
capability the intent requires: `setup_future_usage=off_session` requires
`supports_off_session`; a merchant's `requires` (D15) may add more. A charge
on a stored method has exactly one candidate (D3).

**D8. Fees are the merchant's contract, so they are configuration.**
`fee(amount) = fixed + ceil(amount × percent_bps / 10 000)`, clamped by
optional `min` and `max`, integer arithmetic only, rounding **up** so an
estimate never flatters a rail. Scope is `default`, `domestic` or
`international`, optionally narrowed by brand; on a hosted page only
`default` is ever known, because the card is typed after the rail is chosen.
A rail with no schedule for a method sorts **last** and boot warns, because
treating a missing schedule as zero would route every payment to the one
rail somebody forgot to configure. **The payer never sees a fee.**

**D9. The order is total and deterministic:** estimated fee ascending, then
an explicit `priority`, then rail code. It is one pure function in
`vpay-core`, property-tested, and **it names no rail**: a test asserts the
router module contains no rail-code literal. Across method types the
merchant's own order is kept; the router decides which card rail comes first,
never that cards come before mobile money.

**D10. The router runs at checkout render and again at confirm, and the
confirm's decision is recorded in the confirm transaction** — the method
type, every candidate with its estimated fee and the reason any was excluded,
the chosen rail, and the configuration version — in a `routing_decisions`
table keyed by charge, never on the `/v1` object. An operator asked "why did
this card go through the expensive provider" answers from a row.

**D11. No automatic cascading.** One charge per intent, forever. If the chosen
rail declines, the router does not retry the intent on the next rail; a retry
is a new PaymentIntent. RFC-0008 § 7 (excluding a failing rail for a
cooling-off period) is deferred by the RFC itself and is not accepted here.

### The open questions, answered — proposed, and needing confirmation

**D12. Method-type spellings (RFC-0008 question 1): `card`, `mtn_wallet`,
`orange_wallet`, `paypal`, and `bank_transfer` reserved for D4's two bank
shapes.** One type per wallet network, because each network is a different
rail and that is simpler to route. **The card's brand is an attribute, not a
type:** `card.brand` (`visa`, `mastercard`, `gimac`, …, `unknown`) and
`card.funding` (`credit`, `debit`, `prepaid`, `unknown`), Stripe's own shape.
A rail declares the brands it accepts, and the checkout shows their logos on
one "Card" entry; a button per brand is presentation, never a method type.

The maintainer proposed on 2026-10-16 that each card option be its own type —
`visa_card`, `master_card`, `mtn_card`, `orange_card`. It is refused, for
three reasons the 2026-10-16 research established:

1. **MTN and Orange issue cards; they do not accept them.** MTN's virtual
   MoMo card (19 Dec 2025) and Orange's OM Mastercard (28 Jan 2026) are
   prepaid Mastercards issued with Access Bank Cameroon; neither MTN's MoMo
   API nor Orange's Web Payment API takes a card. At a checkout they are
   `brand=mastercard, funding=prepaid`, and `mtn_card` names no rail a
   payment could be sent to.
2. **On a hosted page vpay cannot enforce a brand.** A payer who clicked
   "Visa" can type a Mastercard, so `visa_card` would promise something vpay
   never checks. Only RFC-0006's vault sees the number before routing.
3. **Stripe's SDKs send `card`.** A per-brand type would cost every
   stripe-node and stripe-rust user the compatibility the `/v1` surface
   exists for, and Stripe restricts brands only through
   `payment_method_options[card][restrictions][brands_blocked]`.

**D13. Default presentation (question 2): `collapse`.** One entry per method
type, and the router's first candidate is used without asking the payer, who
neither knows nor cares who processes their card. `list` — every candidate
shown, cheapest first, the payer picks — stays available per merchant
(`routing.presentation`).

**D14. "Try another way" (question 3): not now.** After a decline the checkout
does not mint a new intent on the next candidate itself; the merchant may,
through the API it already has. The invariant in D11 is untouched either
way, and the product question is left.

**D15. Merchant `requires` (question 4): a deployment default, overridable
per merchant.** `routing.requires` on the deployment (for instance
`[refunds]`) applies to every merchant that does not set its own.

**D16. Fee schedules (question 5) live on the provider account** that
RFC-0009 gives each merchant, beside the contract they price, with deployment
defaults on `providers[]` for merchants with no schedule of their own.

**D17. Sequencing within RFC-0008.** D6 lands with the first adapter that
serves `card` (RFC-0010), because Stripe's SDKs send `card` on day one. D8–D10
land when a second rail serves one method type; until then there is nothing
to sort and `routing_decisions` would record a choice between one.

## Alternatives considered

- **Keep rail codes only, and let merchants order them.** Today's behaviour.
  It pushes a pricing decision onto every merchant per intent, and it has no
  spelling for "a card". It stays available: a rail code still pins.
- **Route by approval rate or health first.** Needs volume a new deployment
  does not have, and is non-deterministic by nature. Cost-first is
  explainable from configuration alone; health is the later filter RFC-0008
  § 7 describes.
- **An external switch** (Hyperswitch) as the router. Possible as one adapter
  with its own routing inside, and vpay would still have to choose between
  that adapter and its others. The router is needed regardless, and
  Hyperswitch's `mpgs` connector is a stub as of 2026-10-16.
- **Per-brand method types.** D12.
- **Accept RFC-0004 whole.** §§ 1–4 and 8–13 (catalog, subscriptions, taxes,
  coupons, documents) have open questions only an accountant or counsel can
  answer, and none of them is about rails. They are accepted section by
  section as each is built, as ADR-0024 did for §§ 5–6.
- **Accept RFC-0006 alongside.** The vault needs an acquirer contract, a
  specification this repository does not hold, and a PCI decision per
  deployment. Nothing in D1–D17 depends on it.

## Consequences

- **The port grows, per consumer.** Each capability in D2 arrives in the
  change that first declares it, with the conformance case that proves it.
  `docs/flows/provider-port.md`'s capability table and the
  `vpay-provider-adapters` skill move with the code, never ahead of it.
- **`payment_method_types` changes meaning for values that are method
  types.** `docs/api/`, both SDKs and their parity rows (ADR-0015), and
  `docs/flows/stripe-sdk-compat.md` gain the new values and
  `payment_method_options[…][rail]`. Rail codes keep their meaning, so no
  existing integration moves.
- **Two migrations, when their consumers land:** `payment_methods` and
  `routing_decisions`, both modelled in `schemas/vpay.cstack` and classified
  in `schemas/privacy-inventory.yaml`. A stored payment method is personal
  data about a payer; what customer erasure does to it is decided by the
  change that creates the table, with RFC-0002's owner, and recorded in
  `docs/flows/customers.md`'s erasure page.
- **Checkout order is no longer purely the merchant's** within a method type
  (D9), and the checkout page gains one "Card" entry with brand logos (D12).
- **Nothing in this ADR is built, and `docs/status.md` does not move.** No
  `NotImplemented` token is created, because no function exists to carry
  one. The skills must say "accepted, not built" from the day this merges,
  which is the opposite of the "Draft, not built" they say today and the same
  distance from "working".
- RFC-0004's status line records § 7 as accepted through this ADR; RFC-0008's
  records the whole RFC, less its § 7.

## Left to the maintainer

1. **Confirm or reverse D12–D17.** D12 reverses a spelling the maintainer
   proposed; if per-brand types are wanted after all, D12's three reasons are
   what a reversal has to answer, and RFC-0010 § 4 and RFC-0011 § 4 change
   with it.
2. **`gimac` as a brand value**, and whether a GIMAC card is ever
   distinguishable from its co-badge on a hosted page. (With a local
   acquirer, when one exists.)
3. **Whether a rail's accepted brands are a capability or configuration.**
   D12 says "a rail declares"; a PSP's brand list is a fact about the
   merchant's contract as much as about the rail.
4. **RFC-0008 § 7, health**, when there is measured traffic to decide a
   threshold with — the same condition the `VpayProviderErrorRateHigh` alert
   is waiting on.
