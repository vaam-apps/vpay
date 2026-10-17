# RFC-0009: Provider accounts per merchant — pass-through on a shared deployment

- **Status:** Draft
- **Author:** vpay maintainers
- **Date:** 2026-10-16
- **Related:** [RFC-0001](0001-settlement-and-payouts.md) (pass-through),
  [RFC-0004](0004-billing-on-top-of-invoices.md) § 7 (one adapter per
  provider), [RFC-0008](0008-payment-method-routing.md) open question 5 (where
  fee schedules live), [ADR-0002](../adr/0002-provider-port.md) (the port),
  [ADR-0003](../adr/0003-yaml-configuration.md) (YAML configuration),
  [ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md) (onboarding is a
  PR), [ADR-0019](../adr/0019-credential-model.md) (credentials as rows).

Nothing in this document is built. It proposes how a merchant's own account on
each provider is named in configuration and reached on every rail call. It
starts from a gap measured in today's tree, not from a feature request.

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
prerequisite.

## Proposal

### 1. A provider account is a named block on the merchant

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
- **Onboarding stays a PR** (ADR-0010). Self-serve onboarding needs the
  dashboard writes ADR-0008 has not designed, and is out of scope here.

### 2. Two levels, merged per call, never overlapping

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

**The port does not change.** Adapters already take `&ProviderConfig` per
call, and the token caches already fingerprint the credentials they are
handed. What changes is which value the core hands them.

### 3. A charge records its account

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

### 4. Which merchant may use which rail

- A rail is offered to a merchant **only if that merchant has an account on
  it**. `payment_method_types` validation (`validated_rails`) refuses a rail
  code the merchant has no account for, with the same `400` shape as an
  unknown rail. The checkout's `build_rails` lists only those rails.
- **Backward compatibility, for one-merchant deployments.** A deployment whose
  `merchant_clients` name exactly one `merchant_id`, and whose rails carry
  their credentials at rail level, boots as today: those credentials are that
  merchant's account, under a reserved id recorded on its charges. No YAML
  change, no behaviour change.
- **Fail closed on two.** A deployment that names two or more `merchant_id`s
  and gives a rail's merchant-level credentials only at rail level refuses to
  boot, naming the rail. This is the measured gap above turned into a
  refusal. It is a breaking change for any such deployment, and is meant to
  be one.

### 5. Proving it

- A container test registers two merchants on one deployment, each with its
  own account on each rail, and asserts from the WireMock request journal that
  each merchant's `submit`, `query_status` and `refund` carried that
  merchant's credentials and nobody else's.
- The mutation that hands every call the first account's config must fail it.
- Boot refusals get `ConfigError` variants and fixtures, as every existing
  refusal does.
- `docs/status/` gains a row only when that test exists. Until then the row is
  the measured gap.

## Delivery

| Step | Contents                                                                                                                                   | Depends on            |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------ | --------------------- |
| 1    | Config shape, two-level merge, boot refusals, one-merchant compatibility                                                                   | nothing               |
| 2    | `charges.provider_account_id`; per-call resolution in `vpay-api` and the worker; the two-merchant proof                                    | 1                     |
| 3    | MTN and Orange per-account keys declared and checked, Disbursements included                                                               | 2                     |
| 4    | Stripe ([RFC-0010](0010-stripe-adapter.md)) and PayPal ([RFC-0011](0011-paypal-adapter.md)) adapters under RFC-0004 § 7, using both levels | 2; their own decision |

## Alternatives considered

- **One deployment per merchant, written down.** No code: document that a
  deployment serves one merchant and add only § 4's boot refusal. It is
  honest, and it is what the tree supports today. It costs an operator one
  stack per merchant, and it makes `merchant_id` scoping a boundary between
  tenants who can never coexist. It remains a valid way to run vpay after
  this RFC.
- **One adapter instance per account.** Rejected. The port already takes the
  configuration per call, capabilities are a property of the rail rather than
  of an account, and a map keyed by account would make "which rails exist" a
  question with as many answers as merchants.
- **Provider accounts as database rows now**, sealed at rest, reusing
  ADR-0019's `credentials` table with a merchant subject. It is where this
  ends up once onboarding is self-serve. It is deferred because it needs a
  key-custody decision for rail secrets and a write path no surface has, and
  configuration already carries every other per-merchant secret.
- **Stripe Connect or PayPal partner models that route through the operator**
  (destination charges, separate charges and transfers, delayed
  disbursement). Rejected by RFC-0001: the money passes through an account
  the operator controls.

## Open questions

1. **May several `merchant_id`s share one account?** One legal entity with two
   shops is legitimate; two legal entities on one account is aggregation. The
   configuration cannot tell them apart. An explicit, per-account list of
   merchants it serves would make the claim the operator's, in writing.
   (Maintainer, then counsel.)
2. **How long must a retired account stay configured?** At least until every
   charge on it is terminal and its refund window has closed. A boot check
   against non-terminal charges needs the database at boot; a worker-time
   `ProviderError::Config` does not. (Maintainer.)
3. **Per-account capabilities.** A Stripe account without `card_payments`, or a
   PayPal seller whose `payments_receivable` is false, is a property of the
   account, not the rail. Is that an adapter's `Config` error at call time, or
   a boot-time check against the provider? (Maintainer, per adapter.)
4. **When do provider accounts move to the database?** With dashboard writes
   (ADR-0008), or earlier if per-merchant `${VAR}`s become unmanageable.
   (Maintainer.)
5. **Does running one deployment for many merchants, each on its own accounts,
   need a status in CEMAC?** The operator holds no money, but Règlement
   n° 04/18/CEMAC/UMAC/COBAC names "partenaires techniques", and Cameroon's
   finance ministry ordered licensed providers on 5 May 2025 to end
   partnerships with unlicensed platforms (secondary source only). (Counsel.)
6. **Fee schedules** (RFC-0008 open question 5): this RFC proposes they live on
   the provider account, beside the contract they price. (Maintainer, when
   RFC-0008 is accepted.)

## Impact on existing invariants

- **Pass-through** becomes true of a deployment with more than one merchant,
  and checkable: a charge names its account, and the account names its
  merchant.
- **Rails behind the port** holds. No `ProviderAdapter` method changes, and
  `ProviderConfig` comes to mean what its own doc comment already says.
- **Branch on capability, never on a provider code** holds. Account
  resolution reads the charge and the configuration, and names no rail.
- **Callbacks are hints** holds, and gains a reason: a callback names a
  charge, the charge names its account, and the authoritative status read
  uses that account.
- **Crash safety** holds: the account id is durable before the rail call.
- **Configuration changes meaning.** `providers[].credentials` narrows to
  rail-level material. One-merchant deployments are unaffected; deployments
  with several merchants and rail-level merchant credentials stop booting
  until they declare accounts.
- **No mocks in shipping processes** holds. The two-merchant proof runs
  against WireMock hosts in configuration.
