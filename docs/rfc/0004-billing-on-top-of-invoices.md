# RFC-0004: Billing on top of invoices — catalog, subscriptions, collection, payment methods and documents

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-09-23
- **Related:** [docs/flows/invoices.md](../flows/invoices.md),
  [RFC-0001](0001-settlement-and-payouts.md) (pass-through),
  [RFC-0002](0002-gdpr-policy-and-operator-decisions.md) (generated files,
  retention), [RFC-0003](0003-refunds-destinations-and-the-first-ledger-postings.md)
  (refunds, the ledger), [RFC-0005](0005-prepaid-customer-balances.md)
  (prepaid balances), and the usage-metering service brief
  [plans/2026-09-23-metering-service.md](../plans/2026-09-23-metering-service.md).
  The gap list this RFC answers was drawn up against Lago's public API
  (v1.53.0, <https://swagger.getlago.com/>) on 2026-09-23.

Nothing in this document is built. Every route, table, event type and
capability below is a proposal; the Status sections of the flow documents
remain the only statement of what exists. _(**Except § 6, built 2026-09-23**
— see its own dated note. The status line stays Draft: the rest of the RFC is
still a proposal.)_

## Problem

vpay can bill a customer **once**: an invoice is drafted, finalized, and paid
through the hosted checkout. Everything a merchant needs to bill a customer
_repeatedly_, or to be paid in any way other than mobile money, is missing:

1. **No catalog.** A line is a description, a quantity and a unit amount.
   There is nothing to reuse across a thousand customers who pay the same
   price.
2. **No schedule.** Nothing creates an invoice on a date. `due_date` is stored
   and read by nothing ([invoices.md](../flows/invoices.md), "What is not
   built").
3. **No way to record money that did not cross a rail.** An invoice becomes
   `paid` only inside the settlement transaction of a succeeded payment
   intent. A merchant paid in cash, by cheque or by bank transfer can only
   `void` or `mark_uncollectible` a bill that was in fact settled.
4. **Mobile money only.** Two rails, MTN MoMo (push) and Orange Money
   (redirect). No card, no bank.
5. **No document.** No PDF, no receipt. A Cameroonian invoice number is read
   by a tax authority; the document it numbers does not exist.
6. **No lookup by customer** outside `GET /v1/invoices?customer=`.
   `GET /v1/payment_intents` takes no `customer` filter.

### The constraints every proposal below is shaped by

| Constraint                                                                                                              | Where it is recorded                                                   | What it forces                                                                                                            |
| ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| No stored payment methods on mobile money; every payment is a payer approving a prompt or visiting a page               | [invoices.md](../flows/invoices.md) § Paying                           | A subscription on mobile money decides _when money is owed_, never _takes_ it. `charge_automatically` is capability-gated |
| Pass-through: each merchant brings their own account on each rail; vpay never holds merchant money                      | [RFC-0001](0001-settlement-and-payouts.md)                             | A card or bank provider is the **merchant's** contract, not vpay's                                                        |
| Rails live behind the port; branch on capability values, never on a provider code                                       | [AGENTS.md](../../AGENTS.md), [ADR-0002](../adr/0002-provider-port.md) | Cards and banks are new adapters and, where needed, new capability values                                                 |
| Every status transition is a compare-and-swap; multi-column CHECKs make broken states unstorable                        | [invoices.md](../flows/invoices.md) § The state machine                | Every new transition below is a `WHERE … AND status = '<from>'`, with a CHECK behind it                                   |
| Event types are Stripe's own spellings, and each is added to the database's vocabulary in the same commit as its writer | [webhooks.md](../flows/webhooks.md), migration `0039`                  | Each event named below must be re-checked against Stripe's list when its migration is written                             |
| The image is `scratch`, runs with a read-only root filesystem, and there is no object storage                           | `backends/Dockerfile`, `deploy/helm/vpay/templates/_helpers.tpl`       | A PDF cannot be written to disk or a bucket without a new infrastructure decision                                         |
| No staff write path exists; dashboard writes are designed and unbuilt                                                   | [ADR-0008](../adr/0008-dashboard-scope.md)                             | Every write proposed here is a merchant `/v1` write                                                                       |
| Erasing a customer who is referenced by an invoice **anonymises** them; invoices carry no customer snapshot             | migration `0041`                                                       | A re-rendered PDF of an old invoice shows `[redacted]`, and an active subscription must pin its customer                  |
| `deny.toml` allows permissive licences only                                                                             | `deny.toml`                                                            | Embedding Typst needs a scoped exception for its OFL-1.1 fonts                                                            |
| The merchant token vocabulary is two scopes, read and write                                                             | [merchant-auth.md](../flows/merchant-auth.md)                          | A client that may write invoice items may write anything that merchant can                                                |

## Proposal

Thirteen parts. The order is the dependency order within the RFC, not a delivery
order; [Delivery](#delivery) gives that.

### 1. Catalog: `product` (`prod_…`) and `price` (`price_…`)

Stripe's two objects, narrowed the way `invoice` was.

- **Product:** `name`, `description`, `active`, `metadata`. Deleted only
  while no price references it; otherwise archived with `active=false`.
- **Price:** `product`, `currency`, `unit_amount` (integer minor units),
  `type` (`one_time` | `recurring`), `recurring.interval`
  (`day|week|month|year`), `recurring.interval_count`, `active`,
  `lookup_key`, `metadata`.
- **A price is immutable once created**, as in Stripe. Changing what a plan
  costs is a new price and an archived old one, so an issued invoice can
  always be traced to the exact terms it was raised under.
- **Flat per-unit only.** No tiers, no packages, no `usage_type=metered`.
  Rating usage is the metering service's job
  ([brief](../plans/2026-09-23-metering-service.md)), and it hands vpay lines
  already priced.
- An invoice item may name a `price` instead of `unit_amount`; the line copies
  `unit_amount` and `currency` from it at write time and keeps the `price`
  reference. `amount_is_the_product` is untouched.

Routes: `/v1/products` (`POST`, `GET`), `/v1/products/{id}` (`GET`, `POST`,
`DELETE`), `/v1/prices` (`POST`, `GET` with `product`, `active`,
`lookup_keys[]`), `/v1/prices/{id}` (`GET`, `POST` — `active`, `metadata`,
`lookup_key` only).

This covers Lago's `plans` (a recurring price) and `add_ons` (a one-time
price).

### 2. Subscriptions: `subscription` (`sub_…`) and `subscription_item` (`si_…`)

A **schedule that issues invoices**. On mobile money it does not collect; on a
rail with `supports_off_session` ([§ 7](#7-cards-and-bank-one-adapter-per-provider))
it may.

**Object:** `customer` (required), `currency`, `items[]` (each a `price` and a
`quantity`; every price `recurring` with the same interval and currency),
`status`, `collection_method`, `days_until_due`, `billing_cycle_anchor`,
`current_period_start`, `current_period_end`, `cancel_at_period_end`,
`canceled_at`, `ended_at`, `latest_invoice`, `default_payment_method`
(only with `charge_automatically`), `metadata`, `created`, `livemode`.

**`collection_method`:**

- `send_invoice`, available on every rail. The invoice is finalized and the
  merchant is told; collection is [§ 4](#4-collection-payment-links-retry-payment-requests-past_due).
- `charge_automatically`, refused with a `400` naming it unless
  `default_payment_method` is a stored method whose rail declares
  `supports_off_session`. **It is never silently downgraded to
  `send_invoice`** — a merchant who asked for automatic collection and got a
  link would be told something false about when they get paid.

**Statuses:** `active`, `past_due`, `canceled`. `past_due` is written by the
due-date sweep in § 4. Stripe's `incomplete`, `trialing`, `paused` and
`unpaid` are not proposed.

**Renewal is a sweep, and each renewal is one transaction.** A singleton
`renew_subscriptions` job, modelled on `sweep_expired` →
`expire_due_sessions`, takes `now` as a parameter and claims subscriptions
with `current_period_end <= now` under `FOR UPDATE SKIP LOCKED`. For each:

1. `UPDATE subscriptions SET current_period_start = current_period_end,
current_period_end = $next … WHERE id = $1 AND status IN ('active',
'past_due') AND current_period_end = $expected`. Matching no row is the
   refusal.
2. If `cancel_at_period_end`, cancel instead and issue nothing.
3. Otherwise write a **draft** invoice carrying the subscription's items and
   every pending item for that subscription ([§ 3](#3-pending-invoice-items)),
   with `subscription_id` and `subscription_period_start` set.
4. Enqueue `finalize_subscription_invoice` at `now + draft_window`.

**The draft window** (per merchant, `merchant_clients[].subscriptions.draft_window_seconds`,
default 3600 — Stripe's own one-hour auto-finalize) is where the metering
service and the merchant add lines before the document is frozen. A window of
`0` finalizes in the renewal transaction itself.

**One invoice per period, forever.** A partial unique index on
`invoices (subscription_id, subscription_period_start)` makes a second invoice
for the same period a row Postgres refuses, whatever the sweep does. The
compare-and-swap is the first guard; the index is the one that survives a
future writer who forgets it.

**Period arithmetic** is one pure function in `vpay-core`,
`period_start(anchor, interval, interval_count, n)`, computed from the anchor
and never from the previous period's end — a 31 January anchor gives 28 (or 29) February, then 31 March, not 28 March. Integer `time` arithmetic in UTC,
property-tested over month ends and leap years.

**Missed periods after an outage** each get their own invoice, in order, with
consecutive numbers — each period was owed. A page with progress reschedules at
zero delay so the backlog drains. (Open question 2.)

**An unpaid invoice does not stop the next one** (Stripe's `send_invoice`
behaviour). What stops the pile-up is the merchant, informed by `past_due`.

**An active subscription pins its customer.** `DELETE /v1/customers/{id}`
answers `409` naming the subscription, and the twelve-month idle sweep treats a
non-`canceled` subscription as a use. Otherwise renewals would bill a customer
whose phone number is `[redacted]`.

Routes: `/v1/subscriptions` (`POST`, `GET` with `customer`, `status`,
`price`), `/v1/subscriptions/{id}` (`GET`, `POST`/`PATCH` — `metadata`,
`cancel_at_period_end`, `days_until_due`, `default_payment_method`; `DELETE`
cancels now). Items are fixed after creation, so there is no proration; a price
change is cancel-and-create. `GET /v1/invoices` gains a `subscription` filter.

**On create**, one transaction writes the subscription, its items, and the
first period's invoice, and follows the same draft-window rule.

### 3. Pending invoice items

`POST /v1/invoice_items` without `invoice`, but with `customer` (and optionally
`subscription`), writes a **pending** item. It is attached to the next invoice
raised for that subscription (at renewal), or for that customer by
`POST /v1/invoices` with `pending_invoice_items_behavior=include`.

This **reverses a sentence** in [invoices.md](../flows/invoices.md) — "there is
no pending-charge inbox, because that is a subscription feature and
subscriptions are not built" — on exactly the condition it gave. The route
keeps its spelling; the pending object is Stripe's `invoiceitem`, and the
attached one stays `line_item`. `invoice_items` gains nullable `customer_id`
and `subscription_id`, and a CHECK that a row has exactly one owner: an
invoice, or a customer.

It is also the carry-over for usage that arrives after a draft window closed
(see the metering brief): late lines go onto the next invoice, never onto a
frozen one.

### 4. Collection: payment links, retry, payment requests, `past_due`

- **The link is made at finalize.** The body of `pay_once` — `intent_for`,
  `session_for`, `attach_intent` — moves into a function that takes no HTTP
  request, so the finalize job (and the `finalize` route, when asked) can give
  every `send_invoice` invoice a `hosted_invoice_url`. This requires
  `merchant_clients[].invoices` URLs to be configured; a subscription whose
  merchant has none is refused at create, because vpay never invents a
  redirect target.
- **Retry.** Today a failed intent stays attached and blocks `pay` until the
  merchant cancels it. Proposed: `pay` cancels an attached intent that is in
  `requires_payment_method` with a `last_payment_error` and no live charge, in
  the same transaction as it mints the next. That is the merchant deciding the
  attempt is over — the condition the current rule exists for — made in one
  call instead of two. (Open question 4; Lago's `retry_payment`.)
- **Payment requests** (Lago's `payment_requests`; Stripe has none). One hosted
  link for every open invoice of one customer: one intent for the sum,
  attached to N invoices, and a settlement that marks all N `paid` in one
  transaction. This changes two invariants (see [Impact](#impact-on-existing-invariants))
  and makes refund allocation ambiguous, so it is **sequenced after RFC-0003's
  refund path is real**.
- **`past_due`.** A due-date sweep, the first reader of `due_date`, moves the
  subscription to `past_due` when an invoice of it passes `due_date` unpaid,
  and back to `active` when every such invoice is settled. It emits
  `invoice.overdue` and `customer.subscription.updated`. It never moves an
  invoice to `uncollectible`: writing a bill off is the merchant's decision.
- **vpay sends the payer nothing.** No SMS, no e-mail. Every reminder is a
  webhook to the merchant. Whether payer reminders should go through `vsms`,
  the organisation's SMS service, is open question 5.

### 5. Customer-scoped reads and the invoice preview

- A `customer` filter on `GET /v1/payment_intents`, `/v1/checkout/sessions`,
  `/v1/refunds` and `/v1/subscriptions`, checked in the same `WHERE` as
  `merchant_id`, as `checkout_sessions`' `payment_intent` filter already is.
  This is the Stripe spelling of Lago's `/customers/{id}/invoices`,
  `/payments` and `/subscriptions`. **`GET /v1/customers` stays unfiltered**,
  for the reason [customers.rs](../../backends/crates/vpay-api/src/v1/customers.rs)
  gives.
- **Invoice preview** for a subscription's next period: Stripe's
  `create_preview`, which replaced `upcoming`. The exact spelling is checked
  against the `stripe` version `sdks/stripe-compat` pins before the route is
  written. It writes nothing and takes no number.

### 6. Manual (out-of-band) payments

> **Built 2026-09-23** (step A; migration `0049_manual-payments.sql`),
> as proposed below. [docs/flows/invoices.md](../flows/invoices.md)
> § "Paid out of band" is the record of what exists — including the
> decisions this section left open, all **accepted** in ADR-0024
> (`docs/adr/0024-customer-filters-and-manual-payments.md`, D9–D19,
> confirmed by the maintainer on 2026-09-23): the method optional and
> defaulting to `other` (D11), a 500-character `reference` (D13), a
> 30-second clock allowance on `received_at` and nothing before
> `finalized_at` (D14, D11), a canceled intent staying attached (D15), a new
> `paid_names_how` CHECK (D16), and a reference refused on an erased
> customer's invoice (D17). Its Status carries the evidence: the
> integration, repository and smoke suites against a real Postgres. The one
> wire addition beyond this section is `out_of_band_payment`, the record's
> four keys on the invoice beside Stripe's `paid_out_of_band` (D9). _(This
> note said "five decisions this section left open" without their status
> until the ADR was accepted the same day.)_

The merchant records that an invoice was settled outside vpay: cash, cheque,
bank transfer received directly, or anything else.

- `POST /v1/invoices/{id}/pay` with `paid_out_of_band=true` — Stripe's own
  parameter — plus a vpay-native `out_of_band[method]`
  (`cash|cheque|bank_transfer|other`), `out_of_band[reference]` and
  `out_of_band[received_at]`.
- It is a compare-and-swap from `open` to `paid`, refused with the same `409`
  as `void` while a non-`canceled` intent is attached, so a payer cannot pay
  through the link after the merchant recorded cash.
- The detail lives in a `manual_payments` table (`mp_…`), one row per invoice
  (payment is still all-or-nothing — `paid_means_nothing_remaining` holds).
  The invoice object gains `paid_out_of_band` (Stripe's key).
- It emits `invoice.paid` — a second writer of an existing type.
- **It posts nothing to the ledger.** No money crossed `payer_clearing`, and
  under pass-through vpay has no account the money arrived in. The row is a
  record of a merchant's statement, and the API says so: nothing in vpay can
  verify it.
- A bank transfer the merchant would otherwise record here by hand can
  instead be matched automatically from the merchant's bank statement:
  [RFC-0007](0007-bank-transfer-reconciliation.md).
- Only the merchant API writes it. An operator recording a payment on a
  merchant's behalf needs ADR-0008's unbuilt dashboard writes and their audit
  log.

### 7. Cards and bank: one adapter per provider

vpay is open source, and a deployment's choice of provider is the operator's,
not the project's. So **every provider that can meet the hard rules below gets
its own adapter behind the existing port**, the operator enables the ones
their merchants have contracts with, and
[RFC-0008](0008-payment-method-routing.md) chooses between them when more than
one can serve the same payment method. No provider is "the" card provider.

There are three routes to a card or bank payment, and this RFC covers the first:

| Route                                          | Where                                             | Who holds PCI scope for card data |
| ---------------------------------------------- | ------------------------------------------------- | --------------------------------- |
| A third-party PSP's hosted page                | This section                                      | The PSP                           |
| vpay's own card vault and acquirer connectors  | [RFC-0006](0006-card-processing-without-a-psp.md) | The operator, for the vault alone |
| Bank transfers reconciled from bank statements | [RFC-0007](0007-bank-transfer-reconciliation.md)  | Nobody: no card data is involved  |

_(This section was titled "Cards and bank through third-party providers" and
framed its criteria as a filter for choosing **one** first provider until the
2026-09-23 amendment. That framing is what made refund-less providers read as
"disqualified". Under one adapter per provider they are adapters with a
capability switched off, as `orange_money` already is.)_

#### The criteria, in three kinds

**Hard rules.** A provider that cannot meet one of these gets no adapter,
because the adapter would break an invariant:

1. **vpay never holds the money.** The merchant contracts the PSP directly,
   and funds land in the **merchant's own** account with the PSP, withdrawable
   to the merchant's bank or mobile money. A PSP that credits _vpay_ is
   aggregation (RFC-0001), and it is out. (Maviance's S3P partner model is out
   on these terms, unless each merchant is its own S3P partner.)
2. **vpay never sees a card number** outside RFC-0006's separately deployed
   vault. A PSP's card is entered on its hosted page or hosted fields, and
   3-D Secure happens there. For the payer this is the existing `Redirect`
   flow, the path Orange Money already takes, so one-off PSP card payments
   need **no new flow**, and merchants stay on PCI SAQ-A. ADR-0021's refusal of
   a native card field stands. (Flutterwave v4's integrator-side card
   encryption fails this rule. Its v3 hosted page does not.)
3. **A secret-authenticated status query.** Callbacks are hints, and
   `query_status` is the only thing that moves money. A PSP whose only signal
   is a webhook, or whose status read accepts a publishable key, gets no
   adapter.
4. **A stub-able contract.** A sandbox reachable from CI, or a specification
   detailed enough to write WireMock mappings from, because every adapter joins
   the one conformance suite.

**Capabilities.** Each is declared by the adapter, off by default. A provider
without one still gets an adapter, and vpay refuses the dependent operation
the way it already does:

| Capability                                     | Existing or new                                                  |
| ---------------------------------------------- | ---------------------------------------------------------------- |
| `supports_refunds`, `supports_partial_refunds` | Existing                                                         |
| `delivers_callbacks`                           | Existing. Signed webhooks are a quality of this, not a gate      |
| `supports_off_session` + `charge_stored`       | New. Gates `charge_automatically` (§ 2)                          |
| `method_types`                                 | New (RFC-0008). Which payment methods the adapter serves         |
| `amount_limits`                                | New (RFC-0008). A provider's per-transaction minimum and maximum |
| `ProviderFlow::Instructions`                   | New flow. PSP-mediated bank transfer or virtual accounts         |

**Deployment questions.** Whether a provider onboards Cameroon merchants,
settles XAF, or is licensed in CEMAC is **not** a property of its adapter. An
open-source adapter for a provider that serves another market is a legitimate
contribution, and the operator's configuration is where the question is
answered. The
[2026-09-23 evaluation](../plans/2026-09-23-card-provider-evaluation.md)
answers it for Cameroon today, from public documentation only:

- No candidate is shown to accept Visa and Mastercard for a Cameroon merchant
  on a hosted page.
- Flutterwave v3 is closest, blocked on written confirmation.
- A local bank acquirer (through RFC-0006) is the lead.

#### Which adapters that implies

From the evaluation, and each subject to its hard rules being confirmed
against a sandbox:

| Adapter            | Method types it would serve                                        | Capabilities off                                    |
| ------------------ | ------------------------------------------------------------------ | --------------------------------------------------- |
| `flutterwave` (v3) | `card` (if hosted XAF is confirmed), MTN and Orange wallets        | none known                                          |
| `cinetpay` (v1)    | MTN, Orange and Express Union wallets; `card` if the v1 API has it | refunds                                             |
| `notch_pay`        | MTN, Orange, Express Union and Yoomee wallets                      | refunds until its OpenAPI carries them; off-session |
| `smobilpay_enkap`  | MTN, Orange and Express Union wallets                              | refunds, off-session                                |

An aggregator that also serves MTN or Orange **duplicates a direct adapter**.
That is now a feature, not a redundancy: RFC-0008 routes an MTN wallet payment
to whichever of `mtn_momo` and the aggregators is cheapest for that merchant.

#### Stored card methods

A `payment_method` (`pm_…`) holds only what the PSP (or RFC-0006's vault)
returns:

- its token, sealed at rest like a rail credential;
- `brand`, `last4`, `exp_month` and `exp_year`.

A method is saved when a hosted payment completes with Stripe's
`setup_future_usage=off_session`. vpay accepts and ignores that parameter
today; it becomes meaningful on rails that declare `supports_off_session`, and
stays ignored on the rest.

A stored method is **bound to the rail that minted it**. A PSP's token is
worthless to any other PSP, so RFC-0008 never routes a stored-method charge
away from its rail.

`GET /v1/customers/{id}/payment_methods` and
`POST /v1/payment_methods/{id}/detach` are the Stripe spellings.

#### Bank payments

- **PSP-mediated bank transfer** (a virtual account or a transfer
  reference): the payer is shown account details and a reference, pays from
  their bank, and the PSP reports it. That is `ProviderFlow::Instructions`,
  rendered as Stripe's `next_action.display_bank_transfer_instructions`.
- **A transfer into the merchant's own account, with no PSP**, is RFC-0007:
  a reference per payment, and the merchant's bank statement as the evidence.
- **A transfer the merchant simply tells vpay about** is § 6.

Each adapter joins the one conformance suite. Its stub is a WireMock host in
configuration, never a linked double.

### 8. Taxes: `tax_rate` (`txr_…`)

- Stripe's object: `display_name`, `jurisdiction`, `inclusive`, `active`,
  and a percentage **stored as an integer in basis points** (19.25 % is
  `1925`) and rendered as Stripe's decimal string. Floating point stays
  denied.
- Applied per line (`tax_rates[]`) or per invoice (`default_tax_rates[]`), and
  inherited by subscription invoices from the subscription.
- Computed **once, at finalize**, beside `amount_due`. The rounding rule (per
  line or per invoice, half-up or banker's) is an accountant's decision
  (open question 7) and is one function.
- `amounts_add_up` gains a `tax` term, and the invoice object gains `tax`,
  `total_tax_amounts[]` and `subtotal`.

### 9. Coupons and discounts: `coupon` (`co_…`)

- `percent_off` (in basis points) or `amount_off` plus `currency`;
  `duration` (`once|repeating|forever`) and `duration_in_months`;
  `max_redemptions`, `redeem_by`.
- Applied to a subscription (`discounts[]`) or an invoice. The discount
  amount is computed at finalize, **before tax**, and appears as
  `total_discount_amounts[]`.
- Redemption counting is `times_redeemed = times_redeemed + 1 WHERE
times_redeemed < max_redemptions`, an expression over the row, so two
  concurrent redemptions cannot both take the last one.
- Promotion codes are not proposed.

### 10. Documents: invoice PDFs through a renderer port

`GET /v1/invoices/{id}/pdf` (merchant-authenticated) and an `invoice_pdf` key
on the invoice object, pointing at it. A draft has no PDF.

**A `DocumentRenderer` port with two real implementations,** chosen per
merchant in YAML (`merchant_clients[].documents.renderer`):

1. **`typst`** — the [Typst](https://github.com/typst/typst) compiler
   embedded as a library (Apache-2.0). It compiles a template with the frozen
   invoice passed as JSON input.
   - A default template ships in the binary. A merchant template arrives the
     way everything else about a merchant does: a reviewed pull request,
     mounted read-only from configuration.
   - **Fonts are embedded** (`include_bytes!`), because the image is `scratch`
     with no font directory. That needs a `deny.toml` licence exception scoped
     to the font crates (OFL-1.1). It is the first copyleft-adjacent exception
     and is called out as such.
   - **Deterministic by construction:** the Typst version is pinned, the PDF
     creation date is set to `finalized_at`, and fonts are embedded. The same
     invoice and template produce the same bytes.
2. **`http`** — delegate to an external renderer. vpay `POST`s the same JSON
   to a YAML-configured URL and receives `application/pdf`.
   - It reuses what webhooks already built: the SSRF guard
     (`vpay-worker/src/ssrf.rs`), the signature scheme
     (`vpay-worker/src/signing.rs`), a timeout and a response-size bound.
   - The renderer is real infrastructure the operator runs, not a stub. A
     merchant who wants documents entirely their own way already can: render
     from the `invoice.finalized` webhook, which is the third option, `none`,
     and today's state.

**No PDF is stored.** There is no object storage, and RFC-0002 D11 recommends
streaming generated files rather than persisting another copy. So a PDF is
rendered on request, and immutability comes from a fingerprint instead:

- At first render, the invoice records `document_sha256`, `document_renderer`
  and `document_template_version`.
- Every later render is compared against those. A mismatch is a `500` and an
  alert — **never a silently different legal document**.

**The conflict this exposes:** after a customer is anonymised, re-rendering
their old invoice shows `[redacted]` and the fingerprint no longer matches.
Tax retention (OHADA accounting rules are commonly cited as ten years — to be
confirmed by counsel) and erasure pull in opposite directions. The proposal is
a **customer snapshot taken at finalize** — name, phone, address, as printed
— with its own retention class under ADR-0020 §4. That is a decision for
RFC-0002's owner (open question 8), and until it is taken, PDFs of invoices
whose customer is anonymised answer `409`, not a redacted document.

**Where rendering runs.** Typst compilation is CPU-bound. It runs on a bounded
blocking pool with a page and line ceiling. Whether it needs its own surface
under ADR-0022 is measured, not assumed (open question 9).

**Receipts** (Lago's `payment_receipts`) are the same port with a second
template, rendered for a `paid` invoice. They get no separate number sequence
until an accountant says a receipt needs one.

### 11. Events, SDKs, parity

| Type                                                                        | Writer                                           |
| --------------------------------------------------------------------------- | ------------------------------------------------ |
| `product.created` / `.updated` / `.deleted`                                 | product routes                                   |
| `price.created` / `.updated`                                                | price routes                                     |
| `customer.subscription.created` / `.updated` / `.deleted`                   | subscription routes, renewal and due-date sweeps |
| `invoice.created`, `invoice.finalized`                                      | **second writer:** renewal and finalize jobs     |
| `invoice.paid`                                                              | **second writer:** `paid_out_of_band`            |
| `invoice.overdue`                                                           | due-date sweep                                   |
| `invoiceitem.created`                                                       | pending item writes                              |
| `tax_rate.created` / `.updated`, `coupon.created` / `.updated` / `.deleted` | their routes                                     |
| `payment_method.attached` / `.detached`                                     | stored card methods                              |

Each type is added to `type_is_a_documented_event` in the migration that ships
its writer, never before. Every resource lands in `sdks/rust` and
`sdks/nodejs` in the same pull request (ADR-0015), with a live suite. The
Rust SDK test that uses `customer.subscription.created` as its "unknown type"
example (`sdks/rust/tests/resources.rs`) needs a new one.

### 12. Deliberately not proposed

| Lago                                                                   | Why not                                                                                                                                |
| ---------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `credit_notes`                                                         | Decided against on 2026-09-10 (issue #91, D5): a refund leaves the invoice `paid` with `amount_refunded`. Revisit only if tax requires |
| `customers/{id}/portal_url`                                            | A third payer surface to keep correct, next to checkout and the mobile SDKs                                                            |
| `webhook_endpoints` CRUD                                               | Endpoints are YAML, changed by a reviewed PR (ADR-0003, ADR-0010, `vpay-config/src/oauth.rs`)                                          |
| `organizations`, `billing_entities`                                    | A merchant is configuration, not a resource                                                                                            |
| `entitlements`, `features`                                             | Product gating, not payments                                                                                                           |
| `quotes`, `order_forms`, `orders`                                      | Sales tooling                                                                                                                          |
| `analytics` (MRR, overdue balance)                                     | Dashboard work, after subscriptions exist                                                                                              |
| Usage metering (`billable_metrics`, usage `events`, `fees`, `*_usage`) | A separate service: [the brief](../plans/2026-09-23-metering-service.md)                                                               |
| `wallets`                                                              | [RFC-0005](0005-prepaid-customer-balances.md)                                                                                          |

### 13. Documentation and skills

Each part ships with its `docs/flows/` page (`catalog.md`, `subscriptions.md`,
`documents.md`; manual payments and taxes extend `invoices.md`), its status
rows, its `docs/api/README.md` rows, and its `vpay-skills` companion PR
(`vpay-merchant-api`, and `vpay-sdks` beside the parity rows). The
`docs/flows/README.md` index gains the missing `invoices.md` row with the
first of them.

## Delivery

Independent tracks, each shippable alone:

| Step | Contents                                                                      | Depends on                                  |
| ---- | ----------------------------------------------------------------------------- | ------------------------------------------- |
| A    | `customer` filters (§ 5), manual payments (§ 6)                               | nothing                                     |
| B    | Catalog (§ 1), pending items (§ 3)                                            | nothing                                     |
| C    | Subscriptions with `send_invoice`, draft window, link at finalize, `past_due` | B                                           |
| D    | Renderer port, Typst and HTTP (§ 10)                                          | open question 8 for anonymised customers    |
| E    | Taxes (§ 8), coupons (§ 9)                                                    | accountant's rounding rule; D for printing  |
| F    | PSP adapters, one per provider (§ 7), and routing (RFC-0008)                  | Providers' written answers; sandbox access  |
| G    | Stored methods and `charge_automatically`                                     | C, F, and a PSP with reusable tokens        |
| H    | Retry and payment requests (§ 4)                                              | RFC-0003's refund path for payment requests |

## Alternatives considered

- **Run Lago itself, with vpay as its payment provider.** Lago is a mature
  billing engine and would supply §§ 1–3, 8–10 at once. Rejected for now:
  - It is AGPLv3. That is acceptable for a separate service, but it is a
    licence decision.
  - Its collection model assumes a provider that can charge a stored method,
    which mobile money cannot.
  - Whether it accepts a custom payment provider has **not** been checked.
  - Merchants' Stripe-shaped clients would face two APIs.
- **A Lago-shaped surface** (`plans`, `charges`, `external_customer_id`).
  Rejected: every `/v1` route is Stripe-shaped, and the SDKs and the
  Stripe-compat suite are what make that worth something.
- **Subscriptions with inline items and no catalog.** Smaller, but every
  subscriber would carry their own copy of a price, and a price change would
  be a data migration.
- **`charge_automatically` on mobile money as a push prompt each period.**
  Rejected: nothing about it is automatic, and an unrequested money prompt on
  a payer's handset is a consent question before it is a feature.
- **Object storage for PDFs.** Deferred rather than rejected. It is the answer
  if a tax authority requires archived bytes, and it needs RFC-0002 D13's
  owner and retention.
- **A PDF library other than Typst** (`printpdf`, headless Chromium). Chromium
  cannot live in a `scratch` image. A low-level PDF library makes every
  template a Rust change. Typst keeps templates as reviewable text.

## Open questions

1. **Catalog first?** This RFC assumes subscriptions reference prices. Inline
   items would ship C sooner, at the cost described above. (Maintainer.)
2. **Missed periods:** one invoice per missed period (proposed), or only the
   current one? A payer could receive three bills at once after an outage.
   (Maintainer.)
3. **Draft window default:** 3600 s as in Stripe, or `0` until the metering
   service exists? (Maintainer.)
4. **Retry:** may `pay` cancel a failed attached intent itself, or must the
   merchant keep cancelling explicitly? (Maintainer.)
5. **Payer reminders:** should vpay ever message a payer (for instance through
   `vsms`), or does every reminder stay a merchant webhook? (Maintainer,
   product.)
6. **Which adapters first.** This is sequencing, not selection: every provider
   that meets § 7's hard rules can have an adapter, and RFC-0008 routes between
   them. The order is set by the written answers the 2026-09-23 evaluation
   lists, and by which merchants have contracts. _(This question read "Which
   PSP first" until the 2026-09-23 amendment made providers additive.)_
   (Maintainer.)
7. **Tax rounding and the VAT rate** actually applicable to each merchant.
   (Accountant.)
8. **Customer snapshot at finalize** and its retention class, versus erasure.
   (RFC-0002 owner, counsel.)
9. **Rendering placement:** in `serve`, in `worker`, or a surface of its own
   under ADR-0022. (Measure first.)
10. **Resource-scoped tokens.** The metering service and any renderer
    integration need narrower write than "everything this merchant can do".
    Does ADR-0010's scope vocabulary grow? (Maintainer.)

## Impact on existing invariants

Kept:

- **One charge per intent, and a retry is a new intent.** Retry (§ 4) mints a
  new one.
- **Callbacks are hints.** A PSP without a status read is disqualified (§ 7).
- **Integer minor units.** Tax and discount percentages are basis points.
- **Every transition is a compare-and-swap** with a CHECK behind it.
- **Rails behind the port.** Cards and banks are adapters, and new behaviour
  is capability-gated.
- **Pass-through.** A PSP must settle to the merchant.
- **No mocks in the shipping binary.** Both renderers are real, and PSP stubs
  are WireMock hosts in configuration.

Changed, each deliberately:

- **The invoice wire object grows** from nineteen keys: `subscription`,
  `paid_out_of_band`, `invoice_pdf`, `tax`, `subtotal`,
  `total_tax_amounts`, `total_discount_amounts`. The key-count pin and both
  SDK decoders move with it.
- **`amounts_add_up`** gains tax and discount terms.
- **`invoice_items` stops being draft-only.** A pending item has no invoice.
  The exactly-one-owner CHECK replaces "a line always has a parent".
- **`paid` gains a second writer** (`paid_out_of_band`), outside the
  settlement transaction. It is still a compare-and-swap from `open`.
- **Payment requests break "one invoice per intent".**
  `only_a_live_invoice_has_an_intent` and `flip_invoice`'s "flips only the
  invoice its own intent is bound to" become "flips every invoice bound to
  it". Refund allocation across those invoices must be decided before step H.
- **`due_date` gets its first reader** (the `past_due` sweep).
- **Customers:** an active subscription blocks both erasure paths.
- **The port grows** `supports_off_session`, `charge_stored`, and possibly
  `ProviderFlow::Instructions`. Each defaults to off or `Unsupported`, so
  existing adapters are unchanged.
- **Job kinds:** `renew_subscriptions`, `finalize_subscription_invoice` and
  the due-date sweep widen `kind_is_known`.
- **`deny.toml`** gains its first font-licence exception.
