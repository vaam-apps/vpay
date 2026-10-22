# RFC-0009: Provider accounts per merchant — pass-through on a shared deployment

- **Status:** Draft. Revised 2026-10-22 on the maintainer's decisions (below):
  phase A first, phase B designed and not scheduled.
- **Author:** vpay maintainers
- **Date:** 2026-10-16; revised 2026-10-22
- **Related:** [RFC-0001](0001-settlement-and-payouts.md) (pass-through),
  [RFC-0004](0004-billing-on-top-of-invoices.md) § 7 (one adapter per
  provider), [RFC-0008](0008-payment-method-routing.md) open question 5 (where
  fee schedules live), [RFC-0010](0010-stripe-adapter.md),
  [RFC-0011](0011-paypal-adapter.md) and [RFC-0012](0012-notchpay-adapter.md)
  (each rail's model B), [RFC-0013](0013-vpay-registers-its-own-callbacks.md)
  and [RFC-0014](0014-verifying-rail-callbacks.md) (callbacks per account),
  [ADR-0002](../adr/0002-provider-port.md) (the port),
  [ADR-0003](../adr/0003-yaml-configuration.md) (YAML configuration),
  [ADR-0008](../adr/0008-dashboard-scope.md) (the dashboard creates no
  merchant), [ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)
  (onboarding is a PR), [ADR-0019](../adr/0019-credential-model.md)
  (credentials as rows), [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
  D1 rule 1 and D5.

Nothing in this document is built. It proposes how a merchant's own account on
each provider is named in configuration and reached on every rail call. It
starts from a gap measured in today's tree, not from a feature request.

## The 2026-10-22 revision

The maintainer answered the question this RFC had left open: whose money a
deployment collects. In the running session's chat:

- Asked whether a licence is needed "only if I don't know the merchants", and
  whether the company's own projects need their payments split, the
  maintainer answered that the merchants are **"Same company for now"**.
- On the shape for later: **"I prefer the way Stripes does the connect
  already"**, then, choosing among proposed options: both layers, phased;
  direct charges plus an application fee; accounts in the database, secrets
  too; the operator as the only platform.
- **"Shouldn't webhook be configured by account ?"**: yes (§ 10).
- **"Hosted page for onboarding should be a separate application again, like
  the checkout"**: § 8.

So this RFC now has two phases:

| Phase | Who the merchants are                                                     | Sections                 | When                                                              |
| ----- | ------------------------------------------------------------------------- | ------------------------ | ----------------------------------------------------------------- |
| **A** | Projects of **one company**: every `merchant_id` is the same legal entity | §§ 3, 4, 5, 6            | Now                                                               |
| **B** | **Other companies**, subsidiaries included, on Stripe Connect's shape     | §§ 1, 2, 7, 8, 9, 10, 11 | When the first merchant that is another legal entity is onboarded |

Phase A is small, because one company collecting its own projects' revenue
into its own account on each rail is that company being a merchant. Nothing
needs splitting. Phase B is the original proposal, grown into Stripe Connect's
shape. § 6's boot rule is what keeps phase A honest: a merchant naming another
legal entity cannot boot on phase A's configuration, so phase B cannot be
skipped by accident.

Sections 1 to 5 keep their numbers: RFC-0010 and RFC-0011 cite § 1, and
RFC-0013 cites §§ 3 and 4.

## Problem

### What the repository says

RFC-0001: "v1 is **pass-through**: each merchant brings their own merchant
account on each rail, and funds move payer → merchant's own wallet. vpay never
holds merchant money." RFC-0004 § 7's first hard rule, RFC-0005 and RFC-0007
all build on that sentence.

The port agrees. `vpay_provider::ProviderConfig` is documented as "Opaque
per-merchant, per-rail configuration handed to the adapter", and every
`ProviderAdapter` method takes `&ProviderConfig` per call. The MTN adapter's
token cache is keyed by a fingerprint of the credentials precisely so that "a
second merchant's configuration passed to the same adapter therefore mints its
own token" ([adapter-mtn-momo.md](../flows/adapter-mtn-momo.md) § "What the
token cache does").

### What the tree does

Measured on `master` at `44c5bf5`, 2026-10-16:

- **Provider credentials exist once per deployment.** They are
  `providers[].credentials` in `config/application.yml`, one block per rail
  code.
- **A merchant registration names no provider account.**
  `vpay_config::oauth::MerchantClient` carries a `merchant_id`, its JWKS,
  scopes, publishable keys, a display name and webhook endpoints. It has
  nothing for a rail.
- **One `ProviderConfig` per rail, built at boot.**
  `vpay_api::v1::ResourceConfig::from_config` projects each `providers[]`
  entry into one `RailConfig`, keyed by code. Every merchant's call on that
  rail is handed the same value. The worker builds the same projection.
- **A charge does not record which account it went to.** `charges` carries
  `provider_code` (migration `0004`) and nothing narrower.

_Re-measured on `master` at `3eee006`, 2026-10-22: unchanged. No source file
names a provider account or a legal entity, and migrations `0001` to `0050`
add no column for either._

Orange shows the gap most plainly. The comment on `credentials.merchant_key` in
`config/application.yml` calls it "per-merchant material". It is configured
once, so every Orange payment on a deployment lands in that one merchant
account, whichever `merchant_id` created the intent.

### What follows

**Pass-through holds today only on a deployment that serves one merchant.** On
a deployment that registers two `merchant_id`s, both merchants' payments on
MTN land in the one MTN account the deployment's credentials belong to, and the
same is true of Orange. Whoever holds that account holds the second merchant's
money. That is the aggregation RFC-0001 says not to start before a licence
conversation, reached by configuration rather than by code.

No test asserts which account a merchant's money reached, because no code path
can tell two merchants' accounts apart. The ledger's per-merchant
`merchant_payable` (migration `0045`) records whom vpay believes is owed; it
says nothing about where the rail put the money.

### Whose money it is (2026-10-22)

The paragraph above assumes that two `merchant_id`s are two owners. A
`merchant_id` is a tenancy boundary inside vpay
(`vpay_config::oauth::MerchantClient::merchant_id`), and nothing says it is a
legal party. **What decides aggregation is whose money it is, not how many
`merchant_id`s there are, and not whether the operator knows the
merchants.** Knowing them is the identification a licensed provider must do;
it does not stand in for the licence.

- **One company, several projects.** Every payment is the company's revenue.
  One account per rail, held by that company, is the company being a merchant
  on that rail. Each project keeps its own `merchant_id`, credentials,
  objects, webhooks and ledger balance; those are internal accounting, not
  separate owners. This is open question 1's "one legal entity with two
  shops", and it is the maintainer's case on 2026-10-22.
- **Two companies.** The second company's money landing in the first one's
  account is aggregation, whether or not the two are related. A subsidiary is
  another company (open question 7).

So the measured gap is a defect only when a deployment serves more than one
legal entity, and nothing in the tree can tell which case it is in. Phase A
makes the configuration say so, and refuses to boot when it says two.

### Why it matters now

Every card provider RFC-0004 § 7 admits is the merchant's contract. The
2026-10-16 desk research on Stripe and PayPal (public documentation only, no
account opened) shows both providers can be pass-through, and that each
requires a per-merchant account identity on every call:

- **Stripe, direct charges.** "The payment appears in the connected account's
  balance, not in your platform's balance." Destination charges and separate
  charges and transfers put it in the platform's balance, and therefore fail
  RFC-0004 § 7's first rule
  ([docs.stripe.com/connect/charges](https://docs.stripe.com/connect/charges)).
  The account is named per request with the `Stripe-Account` header.
- **Stripe, a merchant's own restricted key.** Stripe's documented route for
  "Your users run your software on-premise" is a Stripe App that issues the
  merchant a restricted key for the software to hold
  ([docs.stripe.com/stripe-apps/api-authentication/rak](https://docs.stripe.com/stripe-apps/api-authentication/rak)).
  That is a per-merchant secret, and the operator needs no Stripe account.
- **PayPal, third-party integration.** The platform calls Orders v2 on behalf
  of the merchant with a `PayPal-Auth-Assertion` naming the merchant's
  `payer_id`, and the payee is "the merchant who receives the funds"
  ([developer.paypal.com/api/rest/requests](https://developer.paypal.com/api/rest/requests/#paypal-auth-assertion)).

So a Stripe or PayPal adapter written against today's configuration would
either serve one merchant per deployment or pool every merchant into one
account, and the second fails RFC-0004 § 7 at the first rule. This RFC is the
prerequisite. _(2026-10-22: for merchants that are other companies. For one
company's projects, one account per rail is phase A, § 6.)_

## Proposal

### 1. A provider account is a named block on the merchant

_Phase B._

```yaml
# Illustrative. Names and keys are proposals, not a schema.
providers:
  - code: orange_money
    host: { url: https://api.orange.example/orange-money-webpay/cm/v1 }
    settings: { lang: fr }
    credentials: {} # rail-level only; see § 2
merchant_clients:
  - client_id: acme-cameroon
    merchant_id: acme-cameroon
    provider_accounts:
      - id: acme-orange-1 # stored on every charge; see § 3
        provider: orange_money
        settings: {}
        credentials:
          merchant_key: ${ACME_ORANGE_MERCHANT_KEY}
          client_id: ${ACME_ORANGE_CLIENT_ID}
          client_secret: ${ACME_ORANGE_CLIENT_SECRET}
```

The shape copies one this repository already has. `merchant_clients[].webhooks`
is a per-merchant list whose entries carry an explicit `id` that is "stored
verbatim on every `webhook_deliveries` row", and secrets that must be `${VAR}`
under `livemode: true`. A provider account follows the same rules:

- **The `id` is explicit and never derived**, for the reason publishable keys
  give (D1): a derived identifier cannot be retired, and a merchant who moves
  to a new account on the same rail needs both ids live while old charges
  settle.
- **Secrets are `${VAR}` under livemode**, as `validate_secret` already
  requires of `providers[].credentials`.
- ~~**Onboarding stays a PR** (ADR-0010). Self-serve onboarding needs the
  dashboard writes ADR-0008 has not designed, and is out of scope here.~~
  _Superseded 2026-10-22._ The maintainer chose database rows with sealed
  secrets (§ 11), created through an API (§ 7) and an onboarding application
  (§ 8). The YAML block above stays as the way a deployment declares
  **config-managed** accounts, which boot imports and the API can read but
  not change.

### 2. Two levels, merged per call, never overlapping

_Phase B._

Some providers need a credential that belongs to the deployment and an
identity that belongs to the merchant: a Stripe platform key and an
`acct_…`; a PayPal partner app and a `payer_id`. Others need only the
merchant's own credentials. So:

- `providers[].credentials` and `providers[].settings` become **rail-level**:
  what every account on that rail shares. Empty is allowed.
- `provider_accounts[].credentials` and `.settings` are **account-level**.
- The `ProviderConfig` an adapter receives is the union of the two. **A key
  present at both levels is a refusal to boot**, not an override: a silent
  precedence rule is how a deployment ends up calling one merchant's account
  with another's secret.
- Boot checks every account's keys where ADR-0012 already checks a rail's:
  `vpay_config::config::REQUIRED_RAIL_KEYS`, keyed by rail code, until the port
  grows the hook ADR-0012 anticipates. The table learns which keys are
  rail-level and which are account-level; it stays the one sanctioned place
  outside an adapter that matches on a provider code.

**The port does not change** for this section. Adapters already take
`&ProviderConfig` per call, and the token caches already fingerprint the
credentials they are handed. What changes is which value the core hands them.
_(2026-10-22: § 9's application fee does change the port, and only § 9.)_

### 3. A charge records its account

_Phase A and phase B._

- `charges` gains `provider_account_id`, written in the same transaction that
  writes the charge, before any rail call. The crash-safety ordering rule
  ([crash-safety.md](../flows/crash-safety.md)) applies unchanged: the account
  is durable before the rail can act.
- `query_status`, `parse_callback` follow-up, `refund` and every reconciler
  pass resolve the account from the **charge**, never from the merchant's
  current configuration. A merchant who changes accounts must not have an old
  charge polled, or refunded, against the new one.
- A charge whose `provider_account_id` is no longer configured is a
  `ProviderError::Config` naming the id, never a fallback to another account.
- `schemas/privacy-inventory.yaml` classifies the column. It is an operator
  identifier, not personal data.

In phase A every charge records the reserved id of § 4. That costs one column
now, and it means phase B never has to guess which account an old charge used.

### 4. Which merchant may use which rail

- _Phase B._ A rail is offered to a merchant **only if that merchant has an
  account on it**. `payment_method_types` validation (`validated_rails`)
  refuses a rail code the merchant has no account for, with the same `400`
  shape as an unknown rail. The checkout's `build_rails` lists only those
  rails.
- _Phase A._ **Backward compatibility, for one-merchant deployments.** A
  deployment whose `merchant_clients` name exactly one `merchant_id`, and
  whose rails carry their credentials at rail level, boots as today: those
  credentials are that merchant's account, under a reserved id recorded on its
  charges. No YAML change, no behaviour change. The reserved id is `default`,
  and `(provider_code, provider_account_id)` names an account. Phase B refuses
  an explicit account id of `default`.
- ~~**Fail closed on two.** A deployment that names two or more `merchant_id`s
  and gives a rail's merchant-level credentials only at rail level refuses to
  boot, naming the rail. This is the measured gap above turned into a
  refusal. It is a breaking change for any such deployment, and is meant to
  be one.~~ _Replaced 2026-10-22 by § 6's rule._ Two or more `merchant_id`s on
  rail-level credentials boot when every one of them names the deployment's
  legal entity, and refuse to boot otherwise, naming the merchant. A
  deployment with several projects of one company keeps booting; one with a
  second company stops, which is the measured gap turned into a refusal, as
  before.

### 5. Proving it

_Phase A:_

- A container test registers two `merchant_id`s that name one legal entity,
  takes a payment for each, and asserts that both charges carry
  `provider_account_id = 'default'` and each merchant's own `merchant_id`.
- Boot fixtures: two merchants without `deployment.legal_entity`, and a
  merchant naming another legal entity, each refused with its own
  `ConfigError` variant naming the merchant.
- The mutation that drops the legal-entity check must fail the second fixture.

_Phase B:_

- A container test registers two merchants on one deployment, each with its
  own account on each rail, and asserts from the WireMock request journal that
  each merchant's `submit`, `query_status` and `refund` carried that
  merchant's credentials and nobody else's.
- The mutation that hands every call the first account's config must fail it.
- Boot refusals get `ConfigError` variants and fixtures, as every existing
  refusal does.
- `docs/status/` gains a row only when that test exists. Until then the row is
  the measured gap.

### 6. One company, many projects

_Phase A. Added 2026-10-22._

```yaml
# Illustrative. Names and keys are proposals, not a schema.
deployment:
  livemode: true
  legal_entity: # required once merchant_clients name two or more merchant_ids
    id: acme-sarl
    name: ACME SARL
providers:
  - code: notchpay
    credentials: # the company's own account on this rail: provider account `default`
      public_key: ${NOTCHPAY_PUBLIC_KEY}
      private_key: ${NOTCHPAY_PRIVATE_KEY}
merchant_clients:
  - client_id: shop-web
    merchant_id: shop
    legal_entity: acme-sarl # the written claim: this project is that company's
  - client_id: events-web
    merchant_id: events
    legal_entity: acme-sarl
```

**Boot rules**, each a `ConfigError` variant naming the merchant:

1. Two or more `merchant_id`s, and no `deployment.legal_entity`: refused. The
   configuration has not said whose money it collects.
2. A merchant whose `legal_entity` is not the deployment's: refused, with a
   message that this merchant is another company and that its money cannot
   land in this deployment's accounts until phase B gives it its own.
3. One `merchant_id`: both fields optional, and today's configuration boots
   unchanged (§ 4).

The per-merchant `legal_entity` repeats the deployment's on purpose. It is the
claim open question 1 asked for, made in writing for each project. It is also
what phase B reads to decide whether a merchant may use the company's
accounts or needs its own.

**What each project already has, and keeps.** Its OAuth client and keys
(ADR-0010), objects scoped by `merchant_id`, webhook endpoints
(`merchant_clients[].webhooks`, § 10), publishable keys, checkout origins and
invoice defaults, and a `merchant_payable` ledger balance (migration `0045`).
That balance is the company's internal allocation of its own money to a
project. It is not a claim that the money sits anywhere separate. The rail
pays the company's one account, and the company's withdrawals from it are
the company's.

**What phase A does not need.** No split payment, no application fee, no
provider account per project, no account in the database, no onboarding
application and no platform client. Provider callbacks stay one route per
rail, `/provider/{code}/callback`, because there is one account per rail.

**Two dependencies outside vpay.**

- A company must be able to hold the account. On Notch Pay that is RFC-0012
  § 10 item 10: the merchant agreement § 3 reserves the service to natural
  persons, and Notch Pay has not answered.
- The maintainer's reading that a company collecting its own projects' revenue
  needs no status of its own is not counsel's. It narrows open question 5 for
  phase A; it does not answer it.

### 7. Connected accounts, on Stripe Connect's shape

_Phase B. Added 2026-10-22._

**Accounts.** Each merchant that is its own company is an account, `acct_…`,
a database row. It holds a display name, its legal entity, a status, per-rail
capabilities (`active`, `pending`, `inactive`), `requirements` (what is still
missing, for example an MTN API key), and whether it can take payments.
Phase A's `merchant_clients` are imported at boot as config-managed accounts
of the deployment's own legal entity.

**One platform, the operator.** One OAuth client, `private_key_jwt` as every
client is (ADR-0010), carries an `accounts` scope. It can:

- create, read, update and list accounts at `/v1/accounts`, and deactivate
  one. An account with charges is never deleted;
- create an onboarding link, `POST /v1/account_links` (§ 8);
- act as any account by sending `Stripe-Account: acct_…`. Today vpay accepts
  that header from every client and ignores it
  ([stripe-sdk-compat.md](../flows/stripe-sdk-compat.md)). For the platform
  client it would scope every `/v1` request to that account; for a merchant's
  own client it stays ignored, because a merchant acts only as itself.

A merchant cannot be a platform. Every event carries the `account` it belongs
to.

**Each rail account works one of two ways.**

| Rail        | Connected: platform keys at rail level, the merchant's id at account level | Own keys: the merchant's secrets at account level |
| ----------- | -------------------------------------------------------------------------- | ------------------------------------------------- |
| Stripe      | `Stripe-Account: acct_…`, direct charges (RFC-0010 model B)                | a restricted key (RFC-0010 model A)               |
| PayPal      | `PayPal-Auth-Assertion` naming `payer_id` (RFC-0011 model B)               | the merchant's REST app (RFC-0011 model A)        |
| Notch Pay   | `X-Sync` naming the Sync account (RFC-0012 model B; item 9 first)          | the merchant's own keys (RFC-0012 model A)        |
| MTN, Orange | none: there is no platform model                                           | the merchant's own API user, key, merchant key    |

On MTN and Orange, one shared account would be aggregation, so each company
brings its own. § 2's two levels are exactly this table.

**Direct charges only.** The payment lands in the connected account's
balance. Destination charges and separate charges and transfers stay out
(RFC-0001; ADR-0029 D1 rule 1), and so do `transfer_data` and
`on_behalf_of`, which `/v1` refuses today with a `400` naming the field.

### 8. Onboarding is a separate application

_Phase B. Added 2026-10-22, on the maintainer's "a separate application
again, like the checkout"._

- **`frontends/apps/onboarding`**, built and deployed as `frontends/apps/checkout`
  is: its own image, its own Helm component behind its own `enabled` flag, its
  own origin.
- **Its own API surface**, `/onboard/v1`, authenticated only by the link's
  token. No merchant credential and no platform credential is accepted there,
  and the token reaches nothing outside that one account's onboarding.
- **The link.** `POST /v1/account_links` (platform client) returns a URL that
  expires within minutes and works once, as Stripe's account links do.
- **Own-key rails.** The merchant enters their MTN, Orange or Notch Pay keys
  on the page. They are sealed on receipt (§ 11), and never shown again. The
  page shows only that a value is set, and when.
- **Connected rails.** The page sends the merchant to the provider's own
  onboarding (Stripe's account links; Notch Pay Sync's, once item 9 is
  answered) and records the connected id it comes back with.
- **`requirements` and capabilities** move as each rail is completed, and an
  account takes payments on a rail only once that rail is `active`.

Keeping it apart from the checkout and the dashboard means the one page where
people type rail secrets shares no origin, session or content-security policy
with anything else.

### 9. Application fees

_Phase B. Added 2026-10-22. Blocked on an ADR and on counsel._

- `application_fee_amount` on a payment intent is accepted **only** from the
  platform client acting for an account, and **only** when the chosen rail's
  adapter declares that it can split a payment at the provider. That is a
  capability, so the core branches on it, never on a provider code.
- Which rails can: Stripe's direct charges take an application fee; PayPal
  has `payment_instruction.platform_fees` (RFC-0011, not sent in its v1);
  Notch Pay Sync is unknown (open question 9); MTN and Orange cannot.
- This is RFC-0010's open question 5 and RFC-0011's `platform_fees`. The
  maintainer chose to have it on 2026-10-22, subject to the ADR and counsel
  below.
- Anywhere else the request is refused with a `400` naming
  `application_fee_amount`. It is never collected another way, for example on
  an invoice the merchant did not see coming.
- **The port changes**: the submit request carries the fee, and adapters
  declare the capability. The ledger records the fee, and a refund says
  whether the fee is refunded too.
- **It conflicts with ADR-0029 D1 rule 1 as written**: "a provider that
  credits **vpay** is aggregation and is out". A fee is the provider crediting
  the operator. An amending ADR is needed, as ADR-0031 amended rule 3, with
  this wording proposed: the provider may pay the operator a fee the merchant
  agreed to, taken from a direct charge at charge time; the merchant's share
  never passes through an account the operator controls. Counsel decides
  whether taking that fee needs a status (open question 5).
- In phase A it means nothing: the company's money is already the company's.

### 10. Webhooks and callbacks, per account

_Added 2026-10-22, on the maintainer's "Shouldn't webhook be configured by
account ?"._

**vpay to the merchant: per account, as today.** `merchant_clients[].webhooks`
is already a per-merchant list with its own signing secrets, and every event
already belongs to one `merchant_id`. Phase A changes nothing. In phase B an
account's endpoints are rows, managed by the platform client acting for it.
One extra endpoint is optional: a platform endpoint that receives every
account's events, each naming its `account`, as Stripe Connect's webhooks do.

**The rail to vpay: per provider account.**

- _Phase A_: one account per rail, so one callback route per rail,
  `/provider/{code}/callback`, as today.
- _Phase B, own-key rails_: each provider account registers its own callback
  (RFC-0013 § 7, one pass per provider account), at
  `/provider/{code}/{provider_account_id}/callback`. The id is in the path so
  that RFC-0014 can pick that account's signing secret **before** it trusts
  the body. The `default` account keeps answering the old route.
- _Phase B, connected rails_: the platform registers one endpoint per rail at
  the provider, and each event names its connected account.

A callback stays a hint either way. The charge it names says which account to
read, and only that read moves money (§ 3).

### 11. Secrets at rest

_Phase B. Added 2026-10-22, on the maintainer's choice of "Database, secrets
too". Blocked on an ADR and a threat model._

- Provider account credentials are database rows, **sealed**: a data key per
  row (AES-256-GCM, the cipher `vpay_api::staff_auth::secret_box` already uses
  to seal staff TOTP seeds, ADR-0017), itself wrapped by a key-encryption key that
  carries a key id, so the key-encryption key can rotate without re-entering
  any secret.
- The associated data binds each ciphertext to its account, its rail and its
  field. A ciphertext copied into another account's row fails to open. That is
  § 2's "a key present at both levels is a refusal" carried into the
  database: no row can make one merchant's call with another's secret.
- Write-only through every surface. The API, the dashboard and the onboarding
  page show that a value is set, and when, never the value. Every write is an
  `audit_log` row, as ADR-0008 requires of every dashboard write.
- **Who holds the key-encryption key** (a `${VAR}` under livemode, a cloud
  KMS, or both) is open question 8, and an ADR.
- This supersedes, for accounts, ADR-0010's "onboarding is a PR" and
  ADR-0008's "cannot create merchants, edit provider configuration". Each
  needs an ADR before phase B is built.

## Delivery

_Rewritten 2026-10-22._ The table below said, until then: 1, config shape,
two-level merge, boot refusals, one-merchant compatibility; 2,
`charges.provider_account_id`, per-call resolution, the two-merchant proof;
3, MTN and Orange per-account keys, Disbursements included; 4, Stripe and
PayPal adapters using both levels.

| Step | Contents                                                                                                                                                                                                         | Depends on                          |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| A1   | `deployment.legal_entity` and `merchant_clients[].legal_entity`; § 6's three boot rules with their `ConfigError` variants and fixtures; one-merchant compatibility (§ 4)                                         | nothing                             |
| A2   | `charges.provider_account_id`, `default` on every charge, resolved from the charge (§ 3); the privacy-inventory row; § 5's phase A test; `docs/status/backend.md`'s row                                          | A1                                  |
| B0   | Three ADRs: connected accounts (superseding ADR-0010's onboarding rule and amending ADR-0008 for accounts); key custody for sealed rail secrets; ADR-0029 D1 rule 1 amended for application fees. A threat model | the maintainer; counsel for the fee |
| B1   | Accounts and provider accounts as rows, sealed secrets (§ 11), config-managed accounts imported, the two-level merge (§ 2), per-call resolution, § 5's phase B test, MTN Disbursements per account               | A2; B0 but the fee ADR              |
| B2   | The platform client, `/v1/accounts`, `Stripe-Account` for that client, `account` on events, per-account webhook endpoints and the optional platform endpoint (§ 10), both SDKs and `docs/sdks/parity.md`         | B1                                  |
| B3   | `frontends/apps/onboarding`, `/onboard/v1`, `/v1/account_links`, `requirements` and capabilities (§ 8)                                                                                                           | B2                                  |
| B4   | Per-account provider callbacks at `/provider/{code}/{provider_account_id}/callback` (§ 10; RFC-0013 step 7; RFC-0014)                                                                                            | B1                                  |
| B5   | Application fees (§ 9): the capability, the port change, ledger entries, fee refunds                                                                                                                             | B2; the fee ADR and counsel         |
| B6   | Connected rails: Stripe (RFC-0010 step 6), Notch Pay Sync (RFC-0012 step 7), PayPal (RFC-0011 step 7)                                                                                                            | B2; each RFC's own conditions       |
| B7   | Account writes in the dashboard                                                                                                                                                                                  | B3; ADR-0008 amended                |

ADR-0029 D5 says this RFC "lands before any adapter is offered on a
deployment with more than one merchant". After A2, that holds for a
deployment whose merchants are one company. A deployment serving a second
company needs B1, and A1 refuses to boot it until then.

## Alternatives considered

- **One deployment per merchant, written down.** No code: document that a
  deployment serves one merchant and add only § 4's boot refusal. It is
  honest, and it is what the tree supports today. It costs an operator one
  stack per merchant, and it makes `merchant_id` scoping a boundary between
  tenants who can never coexist. It remains a valid way to run vpay after
  this RFC. _(2026-10-22: for one company with several projects, phase A
  makes one deployment enough.)_
- **One adapter instance per account.** Rejected. The port already takes the
  configuration per call, capabilities are a property of the rail rather than
  of an account, and a map keyed by account would make "which rails exist" a
  question with as many answers as merchants.
- **Provider accounts as database rows now**, sealed at rest, reusing
  ADR-0019's `credentials` table with a merchant subject. It is where this
  ends up once onboarding is self-serve. It is deferred because it needs a
  key-custody decision for rail secrets and a write path no surface has, and
  configuration already carries every other per-merchant secret.
  _(2026-10-22: chosen for phase B (§ 11). Still not first, because phase A
  needs none of it.)_
- **Stripe Connect or PayPal partner models that route through the operator**
  (destination charges, separate charges and transfers, delayed
  disbursement). Rejected by RFC-0001: the money passes through an account
  the operator controls. _(2026-10-22: still rejected. Phase B takes Stripe
  Connect's shape with direct charges only, § 7.)_
- **Phase B first** (added 2026-10-22). Rejected for now: every merchant
  today is one company, and phase B builds sealed secrets, an onboarding
  application and a platform client that nobody would use yet.
- **A provider account per project inside one company** (added 2026-10-22).
  Not needed: no rule requires it, and it would split the company's own money
  across accounts for nothing. Phase B's shape allows it later if a company
  wants it.

## Open questions

1. **May several `merchant_id`s share one account?** One legal entity with two
   shops is legitimate; two legal entities on one account is aggregation. The
   configuration cannot tell them apart. An explicit, per-account list of
   merchants it serves would make the claim the operator's, in writing.
   (Maintainer, then counsel.) _(Answered 2026-10-22 for one company: yes,
   declared per merchant as `legal_entity` (§ 6). Two companies: no, phase B.)_
2. **How long must a retired account stay configured?** At least until every
   charge on it is terminal and its refund window has closed. A boot check
   against non-terminal charges needs the database at boot; a worker-time
   `ProviderError::Config` does not. (Maintainer.)
3. **Per-account capabilities.** A Stripe account without `card_payments`, or a
   PayPal seller whose `payments_receivable` is false, is a property of the
   account, not the rail. Is that an adapter's `Config` error at call time, or
   a boot-time check against the provider? (Maintainer, per adapter.)
   _(2026-10-22: § 8's per-rail capabilities and `requirements` are the place
   for it in phase B.)_
4. **When do provider accounts move to the database?** With dashboard writes
   (ADR-0008), or earlier if per-merchant `${VAR}`s become unmanageable.
   (Maintainer.) _(Answered 2026-10-22: in phase B, secrets included (§ 11).)_
5. **Does running one deployment for many merchants, each on its own accounts,
   need a status in CEMAC?** The operator holds no money, but Règlement
   n° 04/18/CEMAC/UMAC/COBAC names "partenaires techniques", and Cameroon's
   finance ministry ordered licensed providers on 5 May 2025 to end
   partnerships with unlicensed platforms (secondary source only). (Counsel.)
   _(2026-10-22: phase A is one company collecting its own revenue; the
   maintainer reads that as needing no status, and counsel has not been
   asked. Phase B adds the application fee (§ 9).)_
6. **Fee schedules** (RFC-0008 open question 5): this RFC proposes they live on
   the provider account, beside the contract they price. (Maintainer, when
   RFC-0008 is accepted.)
7. **Is a subsidiary the same company?** This RFC says no: it is another legal
   entity, so its money is someone else's, and it needs phase B. Whether any
   CEMAC rule treats a group as one is for counsel. (Added 2026-10-22.)
8. **Who holds the key-encryption key?** A `${VAR}` under livemode, a cloud
   KMS, or both, and how often it rotates. (Maintainer, in B0's key-custody
   ADR. Added 2026-10-22.)
9. **Can Notch Pay Sync take an application fee from a direct charge?**
   Belongs in RFC-0012 § 10 when B5 starts. (Notch Pay, in writing. Added
   2026-10-22.)

## Impact on existing invariants

- **Pass-through** becomes true of a deployment with more than one merchant,
  and checkable: a charge names its account, and the account names its
  merchant. _(2026-10-22: in phase A, the merchants are one company, so every
  account is that company's and pass-through holds trivially; A1 refuses the
  configuration in which it would not.)_
- **Rails behind the port** holds. No `ProviderAdapter` method changes, and
  `ProviderConfig` comes to mean what its own doc comment already says.
  _(2026-10-22: except § 9, whose submit request carries a fee.)_
- **Branch on capability, never on a provider code** holds. Account
  resolution reads the charge and the configuration, and names no rail. § 9's
  fee is a capability too.
- **Callbacks are hints** holds, and gains a reason: a callback names a
  charge, the charge names its account, and the authoritative status read
  uses that account.
- **Crash safety** holds: the account id is durable before the rail call.
- **Configuration changes meaning.** `providers[].credentials` narrows to
  rail-level material. One-merchant deployments are unaffected; deployments
  with several merchants and rail-level merchant credentials stop booting
  until they declare accounts. _(2026-10-22: in phase A they keep booting once
  every merchant names the deployment's legal entity.)_
- **No mocks in shipping processes** holds. The two-merchant proof runs
  against WireMock hosts in configuration.
- **The Stripe-compatible surface** (added 2026-10-22). `Stripe-Account`
  stops being ignored for the platform client in phase B (§ 7), and
  `application_fee_amount` stops being refused where § 9 allows it. Both are
  divergences `docs/flows/stripe-sdk-compat.md` and `docs/api/README.md` must
  say when B2 and B5 land, not before.
