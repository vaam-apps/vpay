# ADR-0031: A status read authenticated by a key the deployment keeps server-side

- **Status:** Accepted
- **Date:** 2026-10-20
- **Deciders:** the vpay maintainer, who decided on 2026-10-20 to build the
  Notch Pay adapter in full and to warn about its production risk rather than
  wait for Notch Pay's answer (quoted under Context). The wording of the rule
  below, its conditions and the boot warning's mechanism were drafted by the
  agent that wrote this ADR, from
  [RFC-0012](../rfc/0012-notchpay-adapter.md) § "The gate", which proposed
  them as "way B". Where this ADR goes beyond what the maintainer said, it
  says so.
- **Amends:** [ADR-0029](0029-card-and-bank-adapters-and-payment-method-routing.md)
  D1, **its third hard rule only**. Rules 1, 2 and 4, D1's "one adapter per
  provider", and D2–D17 are unchanged in every particular. ADR-0029's body
  is not edited; its Status carries a dated note pointing here, as ADR-0009's
  does for [ADR-0030](0030-staff-sign-in-federates-to-an-oidc-provider.md).
- **Implementation:** the change that carries this ADR — the `notchpay`
  adapter (`backends/crates/vpay-adapter-notchpay`), the port's
  `ProviderAdapter::accepted_exposure`, and the boot warning that reads it.
  [docs/status/providers.md](../status/providers.md) is the statement of what
  is admitted under which rule.
- **Number checked at branch time, not assumed:** `git ls-tree -r
origin/master docs/adr` at `b8be107` returns `0001`…`0030`. The open pull
  requests on 2026-10-20 are #294, #293 and #274; none touches `docs/adr/`.

## Context

ADR-0029 D1 admits a provider by four hard rules. The third reads:

> **A secret-authenticated status query.** Callbacks are hints, and the
> status read is the only thing that moves money. A provider whose only
> signal is a webhook, or whose status read accepts a publishable key, gets
> no adapter.

Notch Pay is the provider that rule was worded against. Its OpenAPI
specification (`openapi.yaml` 2.1.0, read 2026-10-20) declares
`GET /payments/{reference}` under `ApiKeyAuth` and `SyncAuth` only, and
`ApiKeyAuth` takes the key Notch Pay's own documentation calls public:
"Public keys (starting with `pk_`) … can be safely included in your frontend
code". The private key, in `X-Grant`, is required only on balance, transfers,
beneficiaries and webhooks. On its own documentation Notch Pay fails rule 3,
and RFC-0012 § "The gate" recorded that it does.

RFC-0012 offered the maintainer two ways forward. **Way A**: ask Notch Pay in
writing whether the read accepts, and an account can require, the private key.
**Way B**: amend rule 3, "a policy change for every future provider, not an
exception for this one". It refused a third: building the adapter under the
public key and calling rule 3 "satisfied in practice", which it called "way B
done silently".

The maintainer chose way A on 2026-10-20 ("Ask Notch Pay first, I'll email
them") and **emailed Notch Pay on 2026-10-20. Notch Pay has not answered.**
Later the same day the maintainer changed course and wrote:

> "Let's do a full implementation and warn the developer about the risks in
> prod. They never answered to me too"

That is way B, taken openly. This record is it.

### What rule 3 protected, and what it did not

Rule 3 bundles two properties, and only one of them is about the key.

- **The read's integrity.** Whether the answer vpay acts on is the provider's.
  That comes from TLS to the provider's host, not from which key
  authenticated the request: a third party who holds a public key still
  cannot forge what `api.notchpay.co` answers vpay. RFC-0012 § 4 adds a
  second check of its own — a success whose amount or currency differs from
  the charge's is `Malformed`, never `Succeeded`.
- **The account's confidentiality and control.** Who else can read, start or
  stop payments on the merchant's account. That is what a publishable key
  gives away. On Notch Pay, whoever holds the public key can read the status,
  amount and customer of any payment on the account; initialize a payment;
  process a pending payment to **any** phone number, which sends a USSD prompt
  in the merchant's name; and cancel any pending payment (RFC-0012 § "What
  rule 3 protects").

So a provider whose read takes a public key is not a provider vpay cannot
read correctly. It is a provider whose merchant carries an exposure a
secret-authenticated provider would not impose. Rule 3 as written decided
that exposure was disqualifying for every merchant. This record decides it is
the merchant's to accept, provided vpay never widens it and says so loudly.

## Decision

**D1 rule 3 is replaced by the following, for every provider from this date:**

> **3. A status query authenticated by a key the deployment keeps
> server-side.** Callbacks are hints, and the status read is the only thing
> that moves money. The read must be authenticated by a credential vpay holds
> server-side and never hands a browser or an app. That is a secret key; or,
> where the provider documents the key its read accepts as public, that key,
> held server-side only, with every operation the provider permits it
> recorded as the merchant's accepted exposure. A provider whose only signal
> is a webhook, or whose status read is unauthenticated, gets no adapter.

### The conditions on the second limb

A provider admitted on the second limb — "a key the provider calls public,
held server-side" — is admitted only while all of the following hold. Each
is a property of the adapter and its documents, so a reviewer can check it,
and none is specific to Notch Pay.

1. **Every secret the provider has is sent on every call.** If the provider
   issues a secret key at all, the adapter sends it on the read and on every
   other call, even where the provider does not ask for it. The day the
   provider starts enforcing it, nothing in vpay changes. On Notch Pay that
   is `X-Grant` with the private key, which the client crate already sends
   (`vpay_notchpay::Client`).
2. **Boot refuses a configuration without that secret.** The public key alone
   is never enough to start, so no deployment runs in the weaker shape by
   omission. On Notch Pay that is `credentials.private_key` in
   `vpay_config::config::REQUIRED_RAIL_KEYS`.
3. **The exposure is written down where an operator reads it.** The
   adapter's flow page carries a "Production risks" section listing every
   operation the public key permits, and the configuration files carry a
   comment beside the key. On Notch Pay that is
   [adapter-notchpay.md](../flows/adapter-notchpay.md) § "Production risks",
   RFC-0012 § 9, `config/application.yml`, `.env.example` and the Helm
   values.
4. **The running process says it too.** The adapter declares its exposure
   through the port (`ProviderAdapter::accepted_exposure`, `None` by
   default), and boot logs a `warn!` for every **configured** rail that
   declares one, naming the exposure and the page that explains it. In
   livemode the line says so, in capitals, and adds that real money is at
   stake. The line never prints a key. **In livemode, boot also refuses the
   rail until its block carries an explicit acknowledgement** — on Notch Pay
   `settings.accept_public_key_exposure: true`, a column of
   `vpay_config::config::REQUIRED_RAIL_KEYS` — and the refusal names the
   setting and the key, never a value. A sandbox only warns. An acknowledged
   livemode boot still logs the warning: the acknowledgement admits the rail,
   it does not make the key private. _Amended 2026-10-20, the day this record
   was written:_ it first said boot does not refuse and put the
   acknowledgement to the maintainer as a question, and the maintainer
   answered "Livemode acknowledgement: YES, require it."
5. **Integrity is checked on the answer.** A success the read reports is
   accepted only if its amount and currency equal the charge's. That check
   matters more on a provider whose key others may hold.
6. **The written question stays open.** The question to the provider — can
   the read require the secret, can the public key's payment operations be
   disabled — stays recorded as unanswered. If the provider answers yes, the
   provider moves to the first limb, the warning can be withdrawn by a
   one-line change to the adapter's declaration, and its admission cell says
   so.

### Why a policy and not an exception

RFC-0012 said this change "would be argued as such", and the argument is the
property split above. Nothing about it is specific to Notch Pay. Many African
payment aggregators issue a public and a private key and draw the line between
them somewhere other than where Stripe draws it. A rule that names one
provider's line would have to be re-argued for each of them, and the next
argument would be made under the same pressure as this one. The conditions
are what make the second limb reviewable: an adapter either sends its secret,
refuses to boot without it, documents its exposure and declares it through
the port, or it is not admitted.

### What this does not change

- **Rules 1, 2 and 4** stand as ADR-0029 wrote them. On Notch Pay, rule 1
  passes in shape on model A only, and model B (Sync, `X-Sync`) is still not
  offered until Notch Pay answers RFC-0012 § 10 item 9 in writing.
- **"Callbacks are hints"** stands, and on the second limb it carries more
  weight. A provider whose public key is in other hands may also have
  webhooks in other hands. `parse_callback` still returns identifiers only.
- **The livemode key rule** (RFC-0012 § 2) stands: a key's `test_` marker must
  agree with `deployment.livemode`, so a sandbox key cannot reach real money.
- **Way B done silently is still refused.** An adapter admitted on the second
  limb that does not declare its exposure through the port, or whose
  admission cell says "rule 3 passes", is a defect.

## Consequences

- `notchpay` is admitted under this record. The Admission cell in
  [docs/status/providers.md](../status/providers.md) says "admitted under
  ADR-0031 (rule 3 amended)", never "rule 3 passes".
- The port gains `ProviderAdapter::accepted_exposure`, `None` by default.
  `mtn_momo` and `orange_money` keep the default: both authenticate their
  reads with secrets (MTN's subscription key and an API-user bearer, Orange's
  client credentials). It lands with its first consumer, as ADR-0029 D2 asks
  of every port capability.
- Every boot of a deployment that configures `notchpay` prints a warning. A
  log line every operator will learn to scroll past is a weak control, and it
  is recorded as weak, which is why livemode does not rely on it alone: a
  livemode deployment must also set the rail's acknowledgement (condition 4),
  so the exposure is accepted by someone writing a line of configuration and
  not only by someone not reading a log.
- The merchant's exposure is real and unchanged by anything vpay does. vpay
  keeps the key server-side; a merchant who also embeds it in a front end, as
  Notch Pay's documentation invites, has given every public-key operation to
  anyone who loads that page.

## Alternatives considered

- **Keep rule 3 and wait for Notch Pay (way A).** Chosen first, on
  2026-10-20. The maintainer moved off it the same day because Notch Pay had
  not answered. It is not lost: condition 6 keeps the question open, and a yes
  moves Notch Pay to the first limb.
- **An exception for Notch Pay only.** Refused. RFC-0012 required that the
  change be argued as a policy, and an exception is the same exposure without
  conditions a reviewer can apply to the next provider.
- **Refuse to boot in livemode, unconditionally.** Not what the maintainer
  asked for ("warn the developer about the risks in prod"). It would make the
  adapter usable only in a sandbox, which is a different decision. The
  acknowledgement below is the refusal an operator can lift on purpose.
- **Require an acknowledgement key in livemode**
  (`settings.accept_public_key_exposure: true`). A middle way between a warning
  and a refusal. First left unbuilt and put to the maintainer as a question;
  **chosen** on 2026-10-20 when they answered "YES, require it", and now part
  of condition 4.
- **Build under the public key and call rule 3 satisfied.** Refused by
  RFC-0012 as way B done silently, and refused here for the same reason.
