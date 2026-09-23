# RFC-0008: Payment-method routing — the cheapest capable rail first

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-09-23
- **Related:** [RFC-0004](0004-billing-on-top-of-invoices.md) § 7 (one adapter
  per provider), [RFC-0006](0006-card-processing-without-a-psp.md) (the vault,
  where the BIN is known), [RFC-0007](0007-bank-transfer-reconciliation.md)
  (bank transfers), [ADR-0002](../adr/0002-provider-port.md) (the port),
  [docs/flows/money.md](../flows/money.md).

Nothing in this document is built.

## Problem

Once RFC-0004 § 7 gives every qualifying provider its own adapter, several
adapters can serve the same payment method:

- two PSPs that both accept Visa and Mastercard;
- RFC-0006's vault with two acquirer connectors;
- MTN MoMo reachable directly (`mtn_momo`) and through an aggregator.

Today nothing chooses between them, for a structural reason:
**`payment_method_types` is a list of rail codes.** A merchant who wants "a
card" must name a provider, and the checkout lists rails in exactly the order
the merchant gave (`build_rails` in `vpay-api/src/browser/checkout_sessions.rs`).

The requirement: **when several Visa/Mastercard providers are configured, the
cheapest appears first**, and the same for any method several rails serve.

## Proposal

### 1. Separate the method from the rail

- A **method type** is what the payer uses: `card`, `mtn_wallet`,
  `orange_wallet`, `bank_transfer`, and so on. The spelling is open
  question 1.
- A **rail** is the adapter that processes it: `mtn_momo`, `flutterwave`,
  `vpay_vault`.
- Each adapter declares the method types it serves, through a new capability,
  `method_types`. Direct adapters serve one (`mtn_momo` serves `mtn_wallet`);
  an aggregator serves several.
- `payment_method_types` on an intent accepts **method types**. A merchant who
  must pin a rail says so explicitly:
  `payment_method_options[card][rail]=flutterwave`. That is a vpay-native
  option, because Stripe has no such concept.
- **Backward compatible.** A rail code in `payment_method_types`, which is what
  every intent carries today, keeps meaning exactly what it means now: that
  method, pinned to that rail. Existing merchants and both SDKs see no change
  until they send a method type.

### 2. The candidate set

For one intent and one method type, the candidates are the rails that:

1. serve the method type (`method_types`);
2. are enabled for the deployment and for the merchant;
3. admit the intent's currency;
4. admit its amount (`amount_limits`, a new capability, because some providers
   cap a transaction; CinetPay's SDKs state 100 to 2,500,000);
5. declare every capability the intent **requires**:
   - `setup_future_usage=off_session` requires `supports_off_session`;
   - a merchant may set `requires: [refunds]` per method type, so that a rail
     that cannot refund is never chosen for them.

A charge on a **stored payment method** has exactly one candidate, the rail
that minted it, because a token is worthless to any other (RFC-0004 § 7).

### 3. The cost model

Fees are **the merchant's contract** (pass-through), so they are
configuration, not code:

```yaml
# Illustrative figures, not a quotation from any provider.
providers:
  - code: flutterwave
    fees:                       # the deployment's default schedule
      card:
        - scope: default        # used when the card's origin is unknown
          percent_bps: 480      # 4.80 %
          fixed: 0
        - scope: international
          percent_bps: 480
          fixed: 0
merchant_clients:
  - client_id: acme-cameroon
    fees:                       # overrides, for merchants with their own terms
      flutterwave:
        card:
          - scope: default
            percent_bps: 390
```

- `fee(amount) = fixed + ceil(amount × percent_bps / 10 000)`, then clamped by
  optional `min` and `max`. Integer arithmetic only, as everything touching
  money is ([money.md](../flows/money.md)). It **rounds up**, so that an
  estimate never flatters a rail.
- **Scope** is `default`, `domestic` or `international`, optionally narrowed
  by `brand`.
- **The card's origin is known only in one case.** On a PSP's hosted page, the
  card is typed _after_ the rail is chosen, so the router can only use
  `default`. With RFC-0006's vault, the card is typed first and the BIN is
  known before any acquirer is asked. So the vault's own connector choice is
  genuinely brand- and origin-aware, the **least-cost routing** a PSP cannot
  offer.
- **A rail with no fee schedule for the method is not cheapest by default.**
  It sorts last, and boot warns about it, because silently treating a missing
  schedule as zero would route every payment to the one rail somebody forgot
  to configure.

### 4. The order

Within one method type, candidates sort by **estimated fee ascending**, then
by an explicit `priority` from configuration, then by rail code. The sort is
total and deterministic. It is one pure function in `vpay-core`, and
property-tested: the same config and intent always give the same order.

**Across** method types, the merchant's own order is kept. The router decides
_which card rail comes first_. It does not decide that cards come before
mobile money, which is the merchant's product choice.

### 5. What the payer sees

Two presentations, per merchant (`routing.presentation`):

- **`list`** — every candidate is shown, cheapest first, and the payer picks.
  This is the requirement as stated.
- **`collapse`** — one entry per method type ("Card"), and the router's first
  candidate is used without asking the payer. It is simpler for the payer, and
  it is what a payer expects when they do not know or care who processes their
  card.

**The payer never sees a fee.** The fee is the merchant's cost, and card
network rules restrict surcharging a payer in many markets, so nothing here
may put the fee on the payer. `RailSpec` gains nothing but its order.

### 6. Where it runs, and what it records

- **At checkout render**, to order (`list`) or to collapse.
- **At confirm, again,** because configuration, amount or health may have
  changed between render and confirm. In `list` mode it checks that the
  payer's choice is still a candidate. In `collapse` mode it picks.
- **A routing decision is recorded in the confirm transaction**: the method
  type, every candidate with its estimated fee and the reason any was
  excluded, the chosen rail, and the configuration version. An operator asked
  "why did this card go through the expensive provider" answers from a row,
  not from a log. It lives in a `routing_decisions` table keyed by charge, not
  on the `/v1` object.

### 7. Health (a later step)

A rail that is failing costs more than its fee. Later, a rail whose recent
transport failures open a circuit is excluded from the candidate set for a
cooling-off period. The inputs are recorded in the decision, so an exclusion
is explainable. This is later because a health signal that flaps makes
routing non-deterministic in a way the first version should not be.

### 8. No automatic cascading

**One charge per intent, forever.** If the chosen rail declines, the router
does **not** retry the same intent on the next rail. A retry is a new
PaymentIntent, as it always has been. Whether the checkout should offer the
payer "try another way" by creating that new intent itself is open question 3.
The invariant is not up for negotiation here.

## Alternatives considered

- **Keep rail codes only, and let merchants order them.** That is today's
  behaviour, and it pushes a pricing decision onto every merchant, per intent.
  It stays available: a rail code still pins.
- **Route by health or approval rate first, cost second.** Approval-rate
  routing needs volume a new deployment does not have, and it is
  non-deterministic by nature. Cost-first is explainable from configuration
  alone, and health is a later filter (§ 7), not the key.
- **Delegate routing to an external switch** (Hyperswitch). That is possible
  as one adapter under RFC-0004 § 7, with its own routing inside. But vpay
  would still have to choose between that adapter and its others, so the
  router is needed regardless.

## Open questions

1. **Method-type spellings.** Stripe's `card` is certain. For wallets,
   Stripe-style names (`mobile_money` plus a network option) or one type per
   network (`mtn_wallet`)? The second is simpler to route. (Maintainer.)
2. **Default presentation:** `list` (the stated requirement) or `collapse`?
   (Maintainer, product.)
3. **"Try another way":** after a decline, should the checkout mint a new
   intent on the next candidate rail itself, which keeps the invariant, or
   leave that to the payer and the merchant? (Maintainer.)
4. **Merchant `requires`:** is refund capability something a merchant should
   require per method, or a deployment policy? (Maintainer.)
5. **Where fee schedules live** once rail credentials become per merchant, if
   they do. (Maintainer.)

## Impact on existing invariants

- **`payment_method_types` changes meaning** for values that are method types.
  Rail codes keep their meaning, so no existing intent, SDK call or
  Stripe-compat case changes. Both SDKs and `docs/sdks/parity.md` gain the new
  values and `payment_method_options[…][rail]`.
- **Checkout order is no longer purely the merchant's** within a method type
  (§ 4).
- **"Branch on capability values, never on a provider code"** is the router's
  whole design. It reads `method_types`, `amount_limits`, capabilities and fee
  configuration, and names no rail. A test asserts that the router module
  contains no rail-code literal.
- **The port gains** `method_types` and `amount_limits`.
- **One charge per intent** is untouched (§ 8).
- **Integer money** holds: fees are basis points and minor units, rounded up.
