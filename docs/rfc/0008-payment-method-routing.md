# RFC-0008: Payment-method routing — the cheapest capable rail first

- **Status:** Accepted 2026-10-17 →
  [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md),
  except § 7 (health), which this RFC defers itself. The ADR answers open
  questions 1–5; its answers are proposed and need the maintainer's
  confirmation. **Revised 2026-10-23 (below): the order becomes a score per
  checkout.** That revision is a **Draft**: it proposes amendments to
  ADR-0029 D8, D9 and D11 that the maintainer has not accepted. The parts that
  do not depend on its formula are built: method types, the eligibility
  filter, a scored and explained ranking with one provisional component, and
  the decision row
  ([plans/2026-10-23-method-type-routing.md](../plans/2026-10-23-method-type-routing.md)).
- **Author:** vpay maintainers
- **Date:** 2026-09-23; revised 2026-10-23
- **Related:** [RFC-0004](0004-billing-on-top-of-invoices.md) § 7 (one adapter
  per provider), [RFC-0006](0006-card-processing-without-a-psp.md) (the vault,
  where the BIN is known), [RFC-0007](0007-bank-transfer-reconciliation.md)
  (bank transfers), [ADR-0002](../adr/0002-provider-port.md) (the port),
  [docs/flows/money.md](../flows/money.md).

Nothing in this document was built when it was written; the 2026-10-23
revision below says what is built now.

## The 2026-10-23 revision: a score per checkout

The maintainer, on 2026-10-23, after asking that the payer choose a network
and vpay show "the correct rail" (ADR-0029's Status quotes both sentences):

> "I think routing should be though again. We should have a score of each
> payment solution for each checkout"

They confirmed `mtn_wallet` and `orange_wallet` the same day. This revision
replaces § 4's sort ("estimated fee ascending, then an explicit `priority`,
then rail code") with a score. §§ 1, 2, 5, 6 and 8 stand as written. § 3's
fee model becomes one component of the score instead of the sort key, and
§ 7's health becomes another. The numbered sections after R9 are kept as they
were written on 2026-09-23.

### R1. The model

For one checkout (one intent) and one method type:

1. **Filter.** The candidates are the rails § 2 admits (ADR-0029 D7): they
   serve the type, are enabled, settle the currency, admit the amount, and
   declare every capability the intent requires. A merchant's pin
   (`payment_method_options[<type>][rail]`) narrows them to one. The filter is
   hard: no score can bring back an excluded rail.
2. **Score.** Every candidate gets a score, an integer, that is a weighted
   sum of components (R2), and a **breakdown** that names each component, its
   input and its points.
3. **Rank.** Highest total first. Ties go to the lower rail code, so the
   ranking is total and deterministic for given inputs.
4. **When.** At checkout render and again at confirm, as § 6 and D10 say. The
   confirm's ranking decides, and it is the one recorded (R5).

The router stays **one pure function that names no rail** (D9's test). Every
input reaches it as data: a rail's declarations, configuration, and, once
they exist, measured rates handed in by the caller. A pure function cannot
read a clock or a table.

### R2. The components

| Component              | What it rewards                                                                                  | Input, and where it would come from                                                                                                                                                                         | Exists today?                                                                                                                                                          |
| ---------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Configured weight**  | the operator's (later the merchant's) preference for a rail, for one type or for all             | `routing.weights` in the deployment YAML; per merchant once RFC-0009 gives merchants provider accounts                                                                                                      | **Yes, built 2026-10-23**, as the only component of `provisional-1`                                                                                                    |
| **Estimated fee** (D8) | the cheaper rail for _this_ amount                                                               | § 3's fee schedules, `fee(amount)` in minor units, with D16's placement on the provider account                                                                                                             | No: no fee schedule is configured or parsed anywhere                                                                                                                   |
| **Rail health** (§ 7)  | a rail that is answering                                                                         | the share of a rail's recent calls that got a usable answer: `provider_requests` (`status_code`, `error_kind`, `sent_at`) over a window                                                                     | The rows exist; no rate is computed. Deferred by § 7 itself                                                                                                            |
| **Completion rate**    | a rail whose payers finish paying                                                                | the share of a rail's charges for this type that reached `succeeded` among those that reached a terminal state, over a window: `charges` (`state`, `failure_code`, `created_at`, and the new `method_type`) | The rows exist, and `method_type` from 2026-10-23; no rate is computed                                                                                                 |
| **Latency**            | a rail that answers fast, and on which payers finish fast                                        | `provider_requests.responded_at − sent_at` (the rail), and `charges` creation to terminal (the payer's prompt)                                                                                              | The timestamps exist; nothing aggregates them                                                                                                                          |
| **Amount fit**         | a rail comfortable at this amount                                                                | `amount_limits` (D2)                                                                                                                                                                                        | No adapter declares a limit. **Argued against as a soft component**: a limit is a hard filter (§ 2), and inside the limits the fee component already prices the amount |
| **Refund capability**  | a rail that can refund, for merchants who care but do not _require_ it (D15 is the hard version) | `Capabilities::supports_refunds` and `supports_partial_refunds`                                                                                                                                             | The flags exist; no merchant preference does                                                                                                                           |
| **Settlement speed**   | a rail that pays the merchant sooner                                                             | the merchant's contract: configuration, like fees                                                                                                                                                           | No                                                                                                                                                                     |

Two rules hold for every component:

- **A missing input is a stated zero, never a skip.** A rail with no fee
  schedule earns zero fee points, and its breakdown says "no input". That is
  § 3's "sorts last, and boot warns" restated as a score: silently skipping
  the component would rank the unconfigured rail as if its fee were neutral.
- **Absolute scales, not relative ones.** Each component maps its input to
  0–1000 points on a fixed scale; a fee in basis points, for instance, maps
  to `1000 − min(bps, 1000)`. It is **not** a ratio of the best and worst
  candidate. A relative scale changes every rail's score when an unrelated
  rail is enabled, and the decision row would then not explain itself.

### R3. How they combine

**A weighted sum**: `score = Σ coefficientᵢ × pointsᵢ`, integer arithmetic,
with the coefficients configured per deployment and recorded with the
breakdown.

- **Explainable.** The breakdown adds up to the total, so an operator reads
  "fee 640 × 2, health 980 × 1, weight 100 × 1" off a row.
- **Monotone.** Improving one input never lowers a rail's rank against
  another whose inputs did not change.
- **Cheap and deterministic.** It is a sum over a handful of integers.

The alternatives considered:

- **Lexicographic**, § 4 as written and D9: fee first, then priority. One
  dimension dominates absolutely, so a rail one basis point cheaper wins
  however unhealthy it is. That is what the maintainer's sentence reopened.
- **A product** of components: one zero component (a new rail with no
  history) zeroes the whole score, which is the cold-start problem made
  absolute.
- **A learned model:** it cannot be explained from a row, and a new
  deployment has no data to learn from.

### R4. Tie-breaks and determinism

- The total, descending, then the rail code, ascending. A tie is common
  while only the configured weight exists, and is meant to be.
- **Determinism is per input, not per day.** Health, completion and latency
  move with time, so the render-time and confirm-time rankings can differ.
  That is intended: the confirm decides, and records the inputs it used
  (R5), so the decision is explainable after the fact even though it is not
  reproducible from configuration alone.
- **Measured components need smoothing.** A window, a minimum sample size
  below which the component contributes its neutral value, and hysteresis.
  The point is that a rail does not flap between first and second on every
  failed call. § 7's reason for deferring health applies to these components,
  not to the score.

### R5. What is recorded

`routing_decisions` (migration `0053`, built 2026-10-23) keeps, per routed
charge, in the transaction that opens the charge:

- the method type, the chosen rail and any pinned rail;
- **every rail considered**: a candidate with its total and its breakdown,
  component by component, and an excluded rail with its exclusion reason;
- the formula's name (`provisional-1` today), so a row stays readable after
  the formula changes;
- a configuration version: a digest of the formula, the weights that bear on
  the type, and every rail's code, declaration and `enabled`.

When a measured component lands, its **input values** (the rate and the
window) go into that candidate's breakdown. A ranking that depended on
yesterday's health then still explains itself.

### R6. What the payer sees

Unchanged from D13 as the maintainer confirmed it (#10): `collapse`. One
entry per method type. The payer never sees a rail, a score or a fee (§ 5).

### R7. What this changes in ADR-0029, proposed

These are proposals for the maintainer; ADR-0029's body is not edited until
they are accepted.

- **D8.** Fees stay the merchant's contract and stay configuration. They
  become **one component** of the score, not the sort key. "Sorts last" for a
  rail with no schedule becomes "earns zero fee points, recorded as no input".
- **D9.** "Estimated fee ascending, then an explicit priority, then rail
  code" becomes "**score descending, then rail code**", with the score R3's
  weighted sum. Still one pure function in `vpay-core` that names no rail,
  and still property-tested: built that way on 2026-10-23.
- **D11.** "No automatic cascading. One charge per intent, forever" stands
  word for word. The amendment is a clarification: a score never retries an
  intent, and a decline lowers a rail's _measured_ components for **later**
  checkouts only, through the window (R4), never for the intent it happened
  on. That clarification is the one place this revision lets traffic move
  automatically, and whether it may is open question R8.3.

### R8. Open questions for the maintainer

These are the maintainer's decisions ("I think routing should be though
again"), not the implementer's.

1. **Which components are wanted in v1?** The built `provisional-1` has the
   configured weight only. The candidates are fee, health, completion rate
   and latency. Fee needs schedules to be written down; the three measured
   ones need only the rows vpay already keeps.
2. **Who sets the weights and coefficients?** The operator for the
   deployment, the merchant for their own payments (needs RFC-0009), or
   both, with the merchant's overriding?
3. **May health move traffic automatically?** With health in the score, a
   failing rail loses checkouts without anyone deciding it. The alternative
   is that health only _reports_ (an alert and a dashboard) and an operator
   changes a weight.
4. **When render and confirm disagree,** and the two rails have different
   flows (a push rail and a redirect rail both serve `orange_wallet`), does
   the confirm follow the render's rail if it is still a candidate (stable
   for the payer) or the confirm's best (the current one)?
5. **Exploration.** Should a small share of checkouts go to a rail that is
   not first, so that its measured components stay fresh? It keeps the data
   honest, at the cost of some checkouts on a worse rail.

### R9. Delivery of this revision

| Step | Contents                                                                                                                                                                                                                                                       | Depends on       |
| ---- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------- |
| R-a  | Method types, `Capabilities::method_types`, the hard filter, the scored ranking with `provisional-1` (configured weight), `routing_decisions` with breakdowns, `charges.method_type`, the confirm pin, Notch Pay's channel from the type. **Built 2026-10-23** | nothing          |
| R-b  | The checkout renders method types (D13 `collapse`)                                                                                                                                                                                                             | R-a              |
| R-c  | The components the maintainer chooses (R8.1), each with its source, coefficient and breakdown entry, and the formula renamed past `provisional-1`                                                                                                              | R8 answered      |
| R-d  | Per-merchant weights                                                                                                                                                                                                                                           | RFC-0009 phase B |

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

_(Superseded by the 2026-10-23 revision's R1–R4, a score per checkout. Kept as
written.)_

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
   they do. (Maintainer.) _(2026-10-16: [RFC-0009](0009-provider-accounts-per-merchant.md)
   proposes that they do, and that a fee schedule lives on the provider
   account it prices.)_

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
