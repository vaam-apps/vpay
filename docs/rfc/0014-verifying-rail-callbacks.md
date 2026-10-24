# RFC-0014: Verifying rail callbacks

- **Status:** **The port and Notch Pay's check are built (2026-10-23),
  stub-proven; never verified against a real Notch Pay delivery.** The maintainer asked,
  on 2026-10-23, "Webhook signature verification (RFC-0014) needs Notch Pay's
  hash key held by vpay as a secret. Lift the earlier 'hash key is evidence
  only, never vpay configuration' rule for this?", and answered "Yes, build
  RFC-0014". Built: § 2's port (`CallbackRequest`, `CallbackVerdict`,
  `verify_callback` defaulting to `NotChecked`, `CallbackCheck` and its
  coherence rule, `Measured` forwarding), § 3's step 4 in the route, Notch
  Pay's `Signature` check (§ 4), the secret as a rail credential (§ 5, in a
  different place: [Built](#built-2026-10-23)), § 6's boot rule, § 7's
  counter, and § 8's tests, each mutation-checked
  ([verification](../status/verification/2026-10-23-webhook-signatures.md)).
  **Not built:** Orange's `BodyToken` check (open question 2), Stripe's and
  PayPal's checks (no adapter), RFC-0013's sealed store (§ 5 phase 2), and
  per-account secrets (RFC-0009). Every proof is against WireMock's Notch Pay
  and a scheme transcribed from Notch Pay's guide: **no webhook Notch Pay sent
  has ever been delivered**, so the signature's format, header, encoding and
  retries are unmeasured (RFC-0012 § 10 item 7). The 2026-10-23 sandbox run
  found that an endpoint created through `POST /webhooks` is stored with
  `"url": null` and received nothing in 21 minutes; verifying a real webhook
  needs the Hash Key **and** an endpoint registered in Notch Pay's dashboard
  ([adapter-notchpay.md](../flows/adapter-notchpay.md) § "The signature
  check"). _(This said "Draft. **Nothing in
  this document is built.**" until 2026-10-23.)_
- **Author:** vpay maintainers
- **Date:** 2026-10-20
- **Decision this designs:** asked on 2026-10-20 whether vpay should verify
  Notch Pay's webhook signatures, the maintainer answered **"Yes, as its own
  piece of work"**: not inside the Notch Pay adapter's pull request
  (vaam-apps/vpay#295), and for every rail that signs, not for Notch Pay
  alone.
- **Related:** [RFC-0010](0010-stripe-adapter.md) § 6 and open question 4
  (the question this RFC answers), [RFC-0011](0011-paypal-adapter.md) § 6 and
  open question 6, [RFC-0012](0012-notchpay-adapter.md) §§ 2, 6 and § 10
  item 7, [RFC-0013](0013-vpay-registers-its-own-callbacks.md) §§ 6–7 and
  delivery step 6, [RFC-0009](0009-provider-accounts-per-merchant.md),
  [ADR-0002](../adr/0002-provider-port.md),
  [ADR-0029](../adr/0029-card-and-bank-adapters-and-payment-method-routing.md)
  D2, [reconciler.md](../flows/reconciler.md),
  [adapter-mtn-momo.md](../flows/adapter-mtn-momo.md),
  [adapter-orange-money.md](../flows/adapter-orange-money.md),
  [rails.md](../reference/rails.md)
- **Sources:** no vendor page was read for this RFC. Every statement about a
  provider's signing scheme is taken from the RFC that read it, with that
  RFC's date: RFC-0010 § 6 (Stripe), RFC-0011 § 6 (PayPal), RFC-0012 § 6
  (Notch Pay), all 2026-10-20, and `adapter-mtn-momo.md` and
  `adapter-orange-money.md` for the shipping rails. Repository facts were read
  at `master` `b8be107` on 2026-10-20.

## Problem

### What the callback route does today

`POST /provider/{code}/callback` is unauthenticated. Its handler parses the
body into a reference, finds the charge, and in one transaction enqueues the
charge's `poll_charge` job if none is queued and pulls it forward to now
(`provider_callback.rs`, `callback`). It never writes charge or intent state.
"The authenticated status query is the only thing that moves money"
(AGENTS.md, "Callbacks are hints").

The module states what that leaves unbounded:

> A POST naming a live charge that is parked further out than the floor still
> causes one real, authenticated `query_status` within about a second, […] so
> a caller who repeats can hold one charge at roughly one rail request per
> worker claim. […] **there is none** [rate limit], and `docs/status.md` says
> so.

What stands in the way is that the caller must know a v4
`provider_reference_id` belonging to a live charge on this deployment. On
Orange there is a second barrier: `parse_callback` refuses a body without a
`notif_token`. That token is never compared with anything, so any non-empty
value passes (`rails.md`, "`parse_callback` fails closed").

### Three of the five rails sign, and one sends a token vpay already holds

| Rail           | What a genuine callback carries                                                                                                                                                                     | Secret vpay would need                                                             | Source                                |
| -------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- | ------------------------------------- |
| `mtn_momo`     | Nothing: "MTN signs nothing and sends no shared secret"                                                                                                                                             | —                                                                                  | `adapter-mtn-momo.md`                 |
| `orange_money` | A `notif_token` in the body. Orange also returns one in the `webpayment` response, and `submit` stores it in the charge's `ref_extra` when present (`vpay-adapter-orange-money/src/mapping.rs:156`) | **None new**: the stored `notif_token` is per payment and already in the database  | `adapter-orange-money.md`, `rails.md` |
| `stripe`       | `Stripe-Signature: t=…,v1=…`, HMAC-SHA256 over `"{t}.{body}"`, "byte-identical to vpay's own `Vpay-Signature`"                                                                                      | The endpoint's `whsec_…`                                                           | RFC-0010 § 6                          |
| `paypal`       | RSA over `transmission_id\|timestamp\|webhook_id\|crc32(body)` with `PAYPAL-TRANSMISSION-ID`, `-TIME`, `-SIG`, `PAYPAL-CERT-URL`, `PAYPAL-AUTH-ALGO`; or a call to `verify-webhook-signature`       | The `webhook_id`, which "is not a secret" (RFC-0013 § 6), and PayPal's certificate | RFC-0011 § 6                          |
| `notchpay`     | `x-notch-signature`, a hex HMAC-SHA256 keyed by the endpoint's Hash Key (`hsk_test_…` in the sandbox). The raw body or a re-serialisation (to confirm). No timestamp                                | The endpoint's Hash Key                                                            | RFC-0012 § 6, § 10 item 7             |

Stripe and PayPal have RFCs and no adapter. Notch Pay's adapter is in review
(vaam-apps/vpay#295) and checks no signature. Orange ships, and its one
callback credential is already in the database, unread.

### What verification buys, and what it does not

It does **not** move money, and it does not let a callback carry a status.
Each RFC that met a signature reached the same sentence: an unverified webhook
"is worth one authenticated status read and no money". That stays true.

What it buys:

- **An anonymous POST stops moving a poll** on every rail whose callback can be
  checked. The unbounded paragraph above narrows from "anyone who knows a
  reference" to "anyone who holds the rail's secret or a genuine callback".
- **The low-latency path RFC-0013 builds becomes trustworthy.** RFC-0013 makes
  sure a rail knows where to call. This RFC makes sure that what arrives there
  is the rail.
- **A precondition for ever using a callback's content.** The route's module
  docs record that repairing a lost `ref_extra` from a callback "needs the
  stored `notif_token` compared against the received one first, and that is
  not built". `docs/status/backend.md` lists "Orange `notif_token`
  verification, and `ref_extra` repair from a callback" as ⛔. This RFC builds
  the comparison. It does not build the repair (open question 5).

What it costs: a secret per signing rail (per account, once RFC-0009 lands),
one port method, one capability field, and a rail-specific check whose
mistakes cost latency.

## Proposal

### 1. The invariant does not move

A verified callback is still a hint. The only thing a verdict decides is
whether the route pulls the poll forward. The authenticated `query_status`
remains the only thing that moves money, on every rail, whatever any callback
said. RFC-0012 § 6 called Notch Pay's signature "a quality signal and not a
gate". In this design it gates **the pull-forward**, which is latency, and
never the money.

### 2. The port: `verify_callback`, defaulted, and `parse_callback` unchanged

```rust
/// What the route hands an adapter to check. Borrowed; nothing is stored.
pub struct CallbackRequest<'a> {
    /// The request's headers, as the route received them.
    pub headers: &'a http::HeaderMap,
    /// The raw body bytes: the bytes that were signed, not a re-parse.
    pub body: &'a [u8],
    /// When the route received the request, for schemes with a timestamp.
    pub received_at: std::time::SystemTime,
}

#[non_exhaustive]
pub enum CallbackVerdict {
    /// The rail's own check passed.
    Verified,
    /// The check ran and failed. The reason is a fixed sentence; it never
    /// quotes a header or a body byte.
    Invalid(&'static str),
    /// This adapter checks nothing (MTN; or a rail whose check is unbuilt).
    NotChecked,
    /// The adapter checks, but holds nothing to check this callback with:
    /// no secret configured, or (Orange) no token stored for this charge.
    NoSecret,
    /// The check could not complete (PayPal's certificate unreachable).
    Unavailable(&'static str),
}

#[async_trait]
trait ProviderAdapter {
    // … existing methods; `parse_callback(&self, body: &[u8])` is unchanged.

    async fn verify_callback(
        &self,
        request: &CallbackRequest<'_>,
        charge: &ChargeRef,
        config: &ProviderConfig,
    ) -> CallbackVerdict {
        CallbackVerdict::NotChecked
    }
}
```

`http::HeaderMap` is the type axum hands the route and the type `reqwest`
re-exports. The port already depends on `reqwest`, and `Cargo.lock` resolves
one `http`, 1.5.0, so no new package is needed.

**Why a new method and not a wider `parse_callback`** (RFC-0010's alternative,
"a change to every rail's signature"):

- **Verification needs the charge, and parsing is how the charge is found.**
  Orange's check compares against the charge's stored `ref_extra`. After
  RFC-0009, every secret "is the charge's account's" (RFC-0013 § 7), so the
  secret is not known until the charge is. Parsing must come first, and it
  must stay what its contract says: "touches no network, holds no credential
  and reads no configuration".
- **Two rails out of five change nothing.** MTN keeps the default. Orange
  gains a method and keeps its `parse_callback` byte for byte.
- `ChargeRef` already carries `ref_extra` (`lib.rs:296`), which is everything
  Orange needs.

`Measured` forwards the method **unmeasured**, as it forwards `parse_callback`
(`measured.rs`, "Forwarded unmeasured"). Like parsing, an HMAC or a comparison
touches no rail, and `provider_operation` deliberately lists only the methods
that can. PayPal's certificate fetch is the one exception. It is measured when
step 4 builds it.

**A capability says what the adapter checks.** `Capabilities` gains
`callback_check: CallbackCheck`, with values `None`, `BodyToken` (Orange) and
`Signature` (Stripe, PayPal, Notch Pay). It records **what vpay's adapter
checks**, not what the rail sends. So Notch Pay declares `None` until its
check is built, and `docs/status/providers.md`'s Callbacks column records both
facts. The coherence rule: `delivers_callbacks == false` implies
`CallbackCheck::None`. The conformance suite enforces the other direction
(§ 8).

### 3. Where it runs in the route

The handler's order becomes:

1. resolve the adapter by `code`, or answer the router's own `404`
   (unchanged);
2. `parse_callback` the body, or answer `400` (unchanged);
3. find the charge under this rail and that reference, or answer `202` and
   log (unchanged);
4. **new:** resolve the charge's `ProviderConfig` (the rail's today; the
   charge's account's after RFC-0009), build the `ChargeRef` the poll would
   build, and call `verify_callback`;
5. enqueue and pull the poll forward **only if** the verdict admits it, then
   answer `202`.

| Verdict          | Pull the poll forward?                                                      | Log                          |
| ---------------- | --------------------------------------------------------------------------- | ---------------------------- |
| `Verified`       | yes                                                                         | `info`, `verdict = verified` |
| `NotChecked`     | yes: today's behaviour, for a rail that signs nothing or is not checked yet | `info`                       |
| `NoSecret`       | yes: today's behaviour. Boot has already said why (§ 6)                     | `info`                       |
| `Invalid(_)`     | **no**                                                                      | `warn`, with the reason      |
| `Unavailable(_)` | **no**: the poll ladder still runs                                          | `warn`, with the reason      |

**Every case still answers `202`, byte for byte.** The module's argument for
making "unknown reference" and "known reference" indistinguishable applies
again here. An answer that differed for "known charge, bad signature" would be
an oracle for "this charge exists". It would also make the real rail retry a
callback that vpay can never accept, when the likely cause is the operator's
own rotated secret. The verdict is visible only in the log and the counter
(§ 7).

The route gets no faster to answer: a verify is an HMAC, or a comparison,
over at most 16 KiB. PayPal's certificate fetch is the one network step, and
§ 4 bounds it. Timing already differs between the two `202` answers today
(the known reference runs a transaction), so verification adds no new signal
an attacker can read.

### 4. Rail by rail

- **`mtn_momo`: nothing changes.** Default method, `CallbackCheck::None`. MTN
  signs nothing. The route stays bounded only by what the module already
  says.
- **`orange_money`: the first consumer, with no new secret.** `verify_callback`
  re-reads the body's `notif_token` and compares it with
  `charge.ref_extra["notif_token"]` **in constant time** (`subtle`, already a
  workspace dependency, which `staff_auth::totp` uses the same way). A
  different token is `Invalid`. A charge with no stored token is `NoSecret`
  (Orange's `webpayment` response may omit one, and `submit` accepts that).
  This is exactly the comparison `rails.md` calls "the caller's job" and
  `docs/status/backend.md` lists as unbuilt. **To confirm:** that the token in the
  notification is the one the `webpayment` response returned.
  `adapter-orange-money.md`'s own "To confirm" list asks "Whether callbacks
  are verifiable beyond `notif_token`". Nobody has yet observed whether the
  two are equal. A wrong assumption here costs every Orange charge its
  callback latency, and nothing else (open question 2).
- **`stripe`: HMAC with a timestamp**, inside RFC-0010's adapter. Split
  `Stripe-Signature` into `t` and every `v1`, compute HMAC-SHA256 over
  `"{t}.{body}"` with each configured secret, and accept any match. Refuse a
  `t` outside a tolerance of 300 s, the same as vpay's own
  `vpay_sdk::webhooks::DEFAULT_TOLERANCE` (`sdks/rust/src/webhooks.rs:37`).
  Several secrets are accepted so that a rotation verifies with either, which
  is how `vpay_worker::signing::signature_header` already signs with several.
- **`paypal`: RSA with a pinned certificate**, inside RFC-0011's adapter.
  Recommended: verify locally. Fetch the certificate from `PAYPAL-CERT-URL`
  **only if** its host is PayPal's own API host for the configured
  environment (to confirm against PayPal's documentation; any other host is
  `Invalid`, never fetched), cache it by URL until it expires, and check the
  RSA signature over
  `transmission_id|transmission_time|webhook_id|crc32(body)`. That needs an
  RSA verification crate: a new dependency, reviewed under `cargo deny`
  (open question 3). Not recommended: PayPal's `verify-webhook-signature`
  call from the route, because it gives every anonymous POST an
  authenticated outbound call. That is a worse amplification than the one
  this RFC narrows.
- **`notchpay`: HMAC without a timestamp**, on top of vaam-apps/vpay#295.
  Compute hex HMAC-SHA256 over the raw body with the configured Hash Key, and
  compare in constant time. RFC-0012 § 10 item 7 decides first whether the
  raw body is what is signed, and whether the header is always lower-case.
  The step-1 sandbox session after #295 measures it with a test Hash Key; until
  then this rail's check is not written. With no timestamp, "a captured webhook
  verifies forever" (RFC-0012 § 6). A replay of a genuine callback can still
  pull a poll forward, bounded by the dedupe key and the floor. Its capturer
  already had to break TLS to `/provider/notchpay/callback` (open question 4).

HMAC comparison lives once in the port crate, as a small helper over `hmac`,
`sha2` and `subtle`. All three are workspace pins already, and `sha2` is
already a dependency of `vpay-provider`. Stripe and Notch Pay call the same
helper, and no adapter writes its own `==` on a MAC.

### 5. Where the secrets come from

**Phase 1: configuration**, because "vpay's rail secrets are configuration
today, never database state" (RFC-0013 § 6):

```yaml
# Illustrative.
providers:
  - code: notchpay
    callbacks:
      signing_secrets: ["${NOTCHPAY_HASH_KEY}"] # a list, for rotation
  - code: paypal
    callbacks:
      webhook_id: "${PAYPAL_WEBHOOK_ID}" # not a secret
```

- Each value is a `${VAR}` placeholder resolved at boot, held as a `Secret`
  with a `Debug` canary, and registered in `schemas/privacy-inventory.yaml`.
- The operator copies the value from the provider's dashboard when they
  register the endpoint by hand. Every endpoint on a rail that registers is
  registered by hand until RFC-0013 lands. _(2026-10-22: RFC-0013 landed for
  Notch Pay, whose endpoint vpay now registers. Nothing here changes: Notch
  Pay's create answer carries no secret (sandbox, 2026-10-21), so its value
  is still copied from the dashboard.)_
- This **reverses RFC-0012 § 2's refusal on its own terms**. It refused to
  hold the hash key because "a secret that nothing reads is a secret held for
  no reason". Under this RFC something reads it. RFC-0012 § 2 gets a dated
  correction in the step that builds Notch Pay's check, not before.

**Phase 2: RFC-0013's sealed store**, for endpoints vpay creates itself.
Stripe's `whsec_…` "arrives once" at creation, and an operator who never
created the endpoint never sees it. RFC-0013 open question 2 recommended
discarding the secret "until the port can verify". This RFC is what lets the
port verify. So **if vpay registers an endpoint itself, verifying it needs
RFC-0013 step 6's store**, and configuration cannot stand in for it. The two
RFCs meet at that step, and neither blocks the other's first build.

**Per account.** Under RFC-0009 the secret comes from the charge's account's
merged `ProviderConfig`, never from anything in the request (RFC-0013 § 7):
"An account in the URL would be an input the caller chooses".

### 6. Boot

For a rail whose adapter declares `CallbackCheck::Signature`:

- **Secret configured:** verify, and an `Invalid` callback moves nothing.
- **No secret, test mode:** boot logs a `WARN` naming the rail, and its
  callbacks keep today's behaviour (`NoSecret`, pulled forward).
- **No secret, livemode: recommended, boot refuses** unless the rail's
  configuration says `callbacks.verify: off` explicitly. That setting logs a
  `WARN` at every boot. This is the shape the maintainer chose on 2026-10-20
  for a rail admitted under ADR-0031's public-key limb (in review in
  vaam-apps/vpay#295): livemode needs an explicit acknowledgement, and test
  mode only warns (open question 1).

`BodyToken` (Orange) needs no configuration and cannot be misconfigured. Its
`NoSecret` is per charge, and boot does not check it.

### 7. Observability

- A counter in `vpay_core::metrics`, `vpay_provider_callbacks_total{rail,
verdict}`, one increment per request that reaches step 4.
- The existing `info` line ("a rail callback moved a charge's poll
  forward") gains `verdict`. `Invalid` and `Unavailable` log at `warn`.
- **Never logged:** a signature header's value, a token, a body byte or a
  secret. `CallbackRequest` has a hand-written `Debug` that prints header
  names only, with a canary test.
- The alert worth having: `Invalid` on a live rail. Its ordinary cause is a
  rotated secret the deployment does not hold yet. Until someone fixes it, the
  cost is callback latency on every charge of that rail.

### 8. Proving it

- **Unit tests per adapter, from known answers.** Stripe's vectors can be
  produced with the same algorithm `vpay_worker::signing` implements. Notch
  Pay's come from a callback captured in the sandbox session. Orange's are a
  stored and a received token, equal and unequal.
- **A conformance table function**,
  `a_forged_callback_is_refused_by_a_rail_that_checks`. For every rail whose
  capability says `BodyToken` or `Signature`, a request with a wrong token or
  signature must answer `Invalid`. For a rail that says `None`, it must
  answer `NotChecked`. **Mutation-checked:** a `verify_callback` that always
  answers `Verified` must fail it.
- **Integration cases** in `backends/tests/integration/tests/provider_callback.rs`:
  a forged Orange callback answers `202` and leaves the poll's `run_at` where
  it was; a genuine one pulls it forward; the response bodies for unknown
  reference, forged callback and genuine callback are byte-identical.
- **The boot rule:** livemode refusal, the `verify: off` acknowledgement, and
  test mode's warning, each tested, with no secret in any message.

## Delivery

| Step | Contents                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Depends on                                                                 |
| ---- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------- |
| 0    | The open questions below                                                                                                                                                                                                                                                                                                                                                                                                                                         | the maintainer                                                             |
| 1    | **The port and its first consumer:** `CallbackRequest`, `CallbackVerdict`, `verify_callback` defaulting to `NotChecked`, `Capabilities::callback_check` and its coherence rule, `Measured` forwarding, the route's new step 4, the counter, the conformance table function; Orange's `BodyToken` check; corrections to `rails.md`, `adapter-orange-money.md`, `reconciler.md` and `docs/status.md`. One pull request against `master`, one against `vpay-skills` | 0                                                                          |
| 2    | Notch Pay's `Signature` check, `callbacks.signing_secrets`, the boot rule, the privacy inventory, RFC-0012 § 2's dated correction                                                                                                                                                                                                                                                                                                                                | 1; vaam-apps/vpay#295 merged; RFC-0012 § 10 item 7 answered in the sandbox |
| 3    | Stripe's check, inside RFC-0010 steps 2–4                                                                                                                                                                                                                                                                                                                                                                                                                        | 1; RFC-0010                                                                |
| 4    | PayPal's check, inside RFC-0011, with the RSA dependency                                                                                                                                                                                                                                                                                                                                                                                                         | 1; RFC-0011; open question 3                                               |
| 5    | Secrets of endpoints vpay registers itself: RFC-0013 step 6's sealed store                                                                                                                                                                                                                                                                                                                                                                                       | 1; RFC-0013 step 3                                                         |

ADR-0029 D2's rule is that a port addition lands with its first consumer.
Orange is that consumer. It ships today, needs no new secret, and closes a
comparison this repository has listed as unbuilt since Step 8.

## Alternatives considered

- **Widen `parse_callback` to take headers** (RFC-0010 § "Alternatives").
  Refused in § 2. Parsing runs before the charge is known, and the secret
  belongs to the charge's account.
- **Answer `401` to a bad signature.** Refused in § 3. It is an oracle for
  "this charge exists", and it makes a real rail with a stale secret retry a
  callback vpay will never accept.
- **Let a verified callback carry the status.** Refused: it would move the
  invariant every adapter and every RFC rests on. The read stays the only
  thing that moves money.
- **Verify in the worker**, by storing the raw request in the job's payload.
  Refused: it writes attacker-chosen bytes into `jobs`, and it turns the
  route's "two statements against `jobs`" bound into a payload store.
- **PayPal's `verify-webhook-signature` from the route.** Refused in § 4: an
  outbound authenticated call per anonymous POST.
- **Secrets only from RFC-0013's store.** Refused for phase 1. Every
  deployment registers endpoints by hand today, and none could verify until
  the store exists. _(2026-10-22: Notch Pay's endpoint is registered by vpay
  since RFC-0013 landed for it, and the refusal still holds: Notch Pay
  returns no secret to store.)_ _(2026-10-23: not by default any more. Since
  vaam-apps/vpay#353 an unset `register` is `off` on Notch Pay, so its
  endpoint is registered by hand in the dashboard again, and the signing
  secret is that endpoint's Hash Key, configured by hand too.)_
- **Source-IP allowlists instead of signatures.** Refused. No shipping rail
  publishes a callback address range that this repository has confirmed:
  RFC-0013 § 11 records that no MTN page read supports an IP-allowlisted
  callback host. And an address proves where a packet came from, not who
  wrote it.

## Open questions

1. **Livemode, a rail that signs, and no secret:** refuse to boot unless
   `callbacks.verify: off` (recommended, the 2026-10-20 acknowledgement
   pattern), or warn only? (Maintainer.)
2. **Orange: enforce the `notif_token` comparison from the first build, or
   count verdicts for one release first?** Recommended: enforce. If the
   assumption in § 4 is wrong, the worst case is the callback latency every
   rail had before Step 8, and the counter shows it on the first day.
   (Maintainer.)
3. **PayPal: local RSA verification with a new dependency and a pinned
   certificate host, or no PayPal check until the verify API can be called
   somewhere other than the route?** (Maintainer, with RFC-0011.)
4. **Replay.** Stripe's 300 s tolerance follows vpay's own. Notch Pay's
   signature has no timestamp. Is a replayed genuine callback acceptable,
   given that all it can do is pull a poll forward? (Maintainer, with
   RFC-0012 § 10 item 7.)
5. **Should a `Verified` callback be allowed to repair a lost `ref_extra`?**
   This RFC makes that possible and does not build it. It would be the first
   time callback content reaches the charge row. (Maintainer; a later RFC.)

## Built (2026-10-23)

Built on `master` `82a68f4e`. Where this RFC left a choice, or sketched
something only as illustration, the safer option was taken and is recorded
here.

- **The first consumer is Notch Pay, not Orange.** The delivery table makes
  Orange's `BodyToken` the port's first consumer (step 1) and Notch Pay's
  check step 2. The maintainer's decision was to build Notch Pay's, and
  ADR-0029 D2's rule (a port addition lands with its first consumer) is met
  by it. Orange stays `CallbackCheck::None` and `NotChecked`: open question 2
  is unanswered, and nobody has observed whether the notification's
  `notif_token` equals the one `webpayment` returned.
- **The secret is a rail credential, not `callbacks.signing_secrets`.**
  § 5's YAML was marked illustrative. The secret is
  `providers[].credentials.callback_signing_secret`, with an optional
  `callback_signing_secret_previous` for a rotation: two names the **port**
  fixes (`vpay_provider::CALLBACK_SIGNING_SECRET`), so boot can ask "does this
  rail that signs hold a secret?" without learning any rail's vocabulary
  (ADR-0002). The credentials map already carries what § 5 asked of a new
  list: a `${VAR}` placeholder enforced in livemode, `Debug` redaction on the
  way in, no new path through `vpay_config`. A list was one more secret path
  with its own placeholder rule and its own redaction to get wrong. Rotation
  is two keys rather than a list of any length.
- **The acknowledgement is `callbacks.verify: on | off`**, in the block
  RFC-0013 added, defaulting to `on`. It is an acknowledgement, not a switch:
  a configured secret is checked whatever it says.
- **Boot refuses four things** (§ 6 named one), each exit 78 and each
  naming the rail and the setting, never a value:
  `IncoherentCallbackCheck` (a check on a rail that never calls back);
  `CallbackSigningSecretMissing` (livemode, `Signature`, no secret, no
  `verify: off`); `CallbackSigningSettingUnread` (a signing secret, or
  `verify: off`, on a rail whose adapter checks no signature: a setting that
  does nothing is one somebody believes in); and
  `CallbackVerifyOffWithSecret` (`off` beside a secret: an acknowledgement of
  something that does not happen). A blank secret is no secret, in boot and
  in the adapter alike, through one function
  (`vpay_provider::callback::signing_secrets_in`). The security review the
  same day added two more, in both modes: `CallbackSigningSecretWhitespace`
  (a non-blank secret, current or previous, with surrounding whitespace,
  which would key the HMAC on a newline) and
  `CallbackSigningPreviousWithoutCurrent` (a previous secret with no current
  one: a rotation whose new key is missing). _(This bullet said "Boot
  refuses four things" until then, and a previous secret alone booted.)_
- **`callbacks.verify` on a per-request rail stays refused.** RFC-0013's
  `CallbacksOnPerRequestRail` refuses any `callbacks` block on such a rail,
  and no per-request rail signs today. The first one that does needs that
  rule narrowed to `register`.
- **The warning runs in `vpay-server serve` only**, the process that answers
  callbacks. The refusal is in `boot_seeds`, which every process runs.
- **The counter is `vpay_provider_callbacks_total{provider,verdict}`.** § 7
  wrote `{rail, verdict}`; every other per-rail series labels the rail
  `provider`. It counts callbacks that reach step 4: an unparseable body and
  an unknown reference are not counted.
- **Two things the route cannot build are `Unavailable`**, not passed to the
  adapter: a rail with no loaded configuration, and a charge row whose
  currency or amount does not parse. Neither admits the pull-forward.
- **What is signed is the raw body**, compared in constant time
  (`Mac::verify_slice`), lower- or upper-case hex. Notch Pay's guide shows
  both a raw-body (PHP, Python) and a `JSON.stringify(req.body)` (Node)
  example, and the latter is the raw body only for a compactly serialised
  body. RFC-0012 § 10 item 7 is the measurement that decides it.
- **Replay is accepted** (open question 4's recommendation): Notch Pay signs
  no timestamp, and `a_replayed_genuine_webhook_still_verifies` pins that as
  a decision.
- **A rotation ends visibly.** A match under the previous secret is
  `CallbackVerdict::VerifiedPrevious`, counted as
  `verdict="verified_previous"`, and `vpay-server serve` warns at every boot
  while `callback_signing_secret_previous` is set. _(Added by the security
  review: until then either key was `Verified`, the previous one was
  accepted silently for as long as it stayed configured, and a rotation
  after a leak had no end signal.)_
- **A declared check that is not performed fails closed.** The route answers
  `Unavailable`, with an `error`, when an adapter that declares a
  `callback_check` other than `None` returns `NotChecked`, the port's
  default. _(Added by the security review: until then such an adapter would
  have had its secret demanded at boot and admitted every forgery.)_
- **Security review, 2026-10-23.** An independent review of the branch found
  no security defect in the comparison, the raw-body handling, rotation's
  blank handling, the fail-closed configuration, the 202 or secret
  redaction. Its should-fix findings are built (the two items above, the
  whitespace refusal, a borrowed rather than cloned configuration on the
  route, tests for both `Unavailable` branches, and a non-canonical body at
  the route). Two are recorded and not changed: each forged callback logs one
  `warn` and counts `verdict="invalid"`, so a caller who knows one live
  reference can inflate both (the runbook's "is `verified` also non-zero?"
  is the mitigation); and `the_comparison_is_the_constant_time_one` is a
  source-text tripwire, not a proof. The
  [verification page](../status/verification/2026-10-23-webhook-signatures.md)
  lists every finding and what was done.
- **HMAC lives once**, in `vpay_provider::callback`, with `hmac` and `hex`
  rather than `hmac`, `sha2` and `subtle`: `verify_slice` is `subtle`'s
  comparison underneath, and `hex` was already a workspace pin.

## Impact on existing invariants

- **"Callbacks are hints":** unchanged (§ 1).
- **ADR-0002:** each rail's scheme stays in its adapter. The port gains one
  defaulted method, one capability field and one HMAC helper. `vpay-api`
  learns no rail's header names.
- **The route's answers:** unchanged, every one. Its bound tightens on every
  rail that checks.
- **"Rail secrets are configuration":** holds in phase 1. Phase 2 is
  RFC-0013's decision, and this RFC makes it due.
- **`verify-status`:** no `NotImplemented` token is planned. A rail whose
  check is unbuilt answers `NotChecked` and declares `CallbackCheck::None`,
  which is the truth about vpay rather than a gap in an operation.
