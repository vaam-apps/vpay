# `vpay-api` — Checkout Sessions (`v1/checkout_sessions.rs`, `browser/checkout_sessions.rs`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## Checkout Sessions (`v1/checkout_sessions.rs`, `browser/checkout_sessions.rs`)

Step 9. Four merchant routes and three payer routes over one object. The
schema reasoning is in [vpay-db.md](../vpay-db/payment-intents-and-checkout-sessions.md#checkout_sessions); what
follows is what the HTTP layer adds.

The merchant surface is ordinary `/v1`: token-authenticated, tenant-scoped,
`Idempotency-Key` on both POSTs through the same `PostRequest` the payment
intents use — shared rather than copied, because a second claim/finish/release
dance is exactly how one of the two ends up leaving a merchant's key stuck
`in_flight`.

`create` refuses three things with a `409` and not a `400`: an intent that is
not `requires_payment_method`, an intent that already has a charge, and an
intent that already has an open session. All three are facts about an
_object's state_ rather than about the request's shape. The third is checked
twice on purpose: `find_open_by_intent` first, so the merchant gets a sentence
naming the session in the way, and then the partial unique index, which is the
actual guard — between that read and the insert a concurrent create can commit
one, and the `UniqueViolation` is turned into the same `409` so a merchant
cannot see two different errors for one situation depending on timing.

The URL rules (`checked_forward_url`) deliberately do **not** parse the URL:
`{CHECKOUT_SESSION_ID}` (D5) is a literal substring a merchant writes, and
`url::Url::parse` percent-encodes the braces. A validator that normalised
would have to either store the normalised form — breaking the substitution the
placeholder exists for — or discard its own parse, proving nothing. So the
rules are a scheme prefix from a closed two-entry list, a character count, and
`https` under `deployment.livemode`; the column's CHECKs in migration `0028`
are the backstop for a writer that forgets, not the primary guard.

### The credential ladder

| Route                                           | Presents                                  | May read                                                                            |
| ----------------------------------------------- | ----------------------------------------- | ----------------------------------------------------------------------------------- |
| `GET /v1/checkout/sessions/{id}`                | a merchant bearer token                   | the session, `client_secret` and `url` (which carries `?key=` and `#client_secret`) |
| `GET /v1/browser/checkout/sessions/{id}`        | `key` + the **session's** `client_secret` | the session, `payment_intent` expanded **with the intent's own `client_secret`**    |
| `GET /v1/browser/checkout/sessions/{id}/return` | `key` + the session's `return_token`      | the session, `payment_intent` expanded **without** it                               |
| `GET /v1/browser/checkout/origins`              | `key` alone                               | the tenant's `checkout_origins`                                                     |

The escalation this closes, read upwards: `return_token` (a query-string value
that reaches access logs) → the session read → the intent's `client_secret` →
`confirm`. Every hop is refused, and two of them are worth stating because
they are easy to reintroduce:

- **The two browser reads are separate path patterns, not one handler with
  two optional parameters.** Each builds only the expected value it accepts,
  so a return token cannot open the session read.
- **Neither browser read renders the session's `url`.** It carries the
  session's own `client_secret` in its fragment, so echoing it on the return
  read would hand the weaker credential's holder the stronger one. It costs
  the page nothing: a payer on the session read is already _at_ that URL, and
  a payer on the return page has no use for it.

The choice is expressed as a _type_ — `ExpandableIntent::Expanded` versus
`::ExpandedWithSecret` — rather than as a field one handler clears, so a route
that wanted to render a credential has to say the word.

`browser::checkout_sessions` reuses `browser::secrets_match`, and therefore
`browser::ct_compare`, rather than introducing a second constant-time compare.
There is one in the crate, proven once, in the place its own doc comment
explains.

The origins route answers `200 {"origins": []}` for an unknown key rather than
a 404, which is the same confidentiality property arrived at from the other
side: an empty list is also what a _registered_ tenant with no origins gets,
so the two are indistinguishable and nobody can enumerate a deployment's
merchants by trying keys. It is also the fail-closed answer — no origins means
no embedding.

### `payment_intent` is an id on `/v1` and expanded on `/v1/browser`

Stripe's `expand` shape (`model::ExpandableIntent`, `#[serde(untagged)]`: a
string or an object, no discriminator).

- `/v1/checkout/sessions` — **the id**. A merchant already holds the intent
  they created, so expanding would put a second, possibly stale copy of every
  amount on the wire, multiplied by the page size on the list.
- `/v1/browser/checkout/sessions/{id}` and `.../return` — **the object**.
  vpay's own page has only a session id and a session secret; it needs the
  amount, the currency, the status, `payment_method_types` (which rails to
  offer), `next_action` and `last_payment_error` before it can paint anything,
  and two round trips for that would mean two loading states instead of one.
  The session read adds `client_secret`; the return read does not.

`PaymentIntentObject` itself is untouched by any of it — the twelve-key
tripwire (`every_documented_key_is_present_including_the_null_ones`) still
stands, and `PaymentIntentWithSecret` is still the only wrapper that carries a
credential.

### Which publishable key a session pins

Every URL vpay mints for the checkout app carries `?key={pk}`, because all
three browser routes authenticate by it and the return page cannot use a
fragment — a payer arrives there from a URL the _rail_ replays.

`create` takes an optional `publishable_key`. Named and registered to this
tenant → that one; omitted → the tenant's **first configured key**, in the
order the operator wrote it (`first()` and not "any", so a merchant can
predict their own link from their own YAML); named but not theirs → `400`
naming the parameter; **no keys at all** → `checkout_not_configured`.

That last answer shares its `code` with a missing `checkout.public_base_url`
and differs only in the sentence: from a merchant's side both are "this vpay
cannot do hosted checkout for me", and only the message tells whoever they
forward it to which line of YAML to add.

The unregistered-key answer is a `400` and **not** the uniform 404 the browser
surface gives, and the asymmetry is the point. On `/v1/browser` the caller is
an unauthenticated payer and any distinction between "unknown key" and
anything else is an enumeration oracle. Here the caller is the merchant,
authenticated, asking about their own registration: there is nothing to hide
from them, and a uniform refusal would leave them unable to tell a typo from a
key they forgot to register. The key is echoed back for the same reason.

The chosen key is **stored on the row**, not re-derived on read — see
[vpay-db.md](../vpay-db/payment-intents-and-checkout-sessions.md#publishable_key-is-a-column-and-return_page_url-is-a-method)
for why a key rotation would otherwise strand payers mid-flight.

The hosted `url` is `{base}/c/{cs_id}?key={pk}#{client_secret}`, and the two
halves are on opposite sides of the `#` because they need opposite things: the
page reads `key` **server-side**, in `middleware.ts`, to look up
`checkout_origins` and set `frame-ancestors` before any script runs (D4) — and
a fragment never reaches a server — while the credential must never reach one
(D6).

### `checkout_not_configured` answers 500, not the plan's 503

`ApiError::CheckoutNotConfigured` carries the code the Step 9 plan asks for
and **not** the status. This is a deliberate departure, recorded here because
it is the kind of thing that otherwise looks like an oversight.

ADR-0011 derives the status from the `Category`, never from a call site, and
`Category::Storage` is the only one that answers `503`. Classifying a missing
`checkout.public_base_url` as storage would be wrong twice: it would tell an
operator Postgres was unreachable, and — the part that actually costs
something — `Category::Storage`'s `Retry::AfterBackoff` would tell a
merchant's SDK to retry a request that cannot succeed until someone deploys a
configuration change. `Category::Configuration` says exactly what is true
("the deployment is misconfigured for this operation … fixed by a deploy,
never by retrying") and its status is `500`.

Making it a `503` honestly would mean either a new `Category` or moving
`Category::Configuration` to `503` — both ADR-level changes affecting every
error in the workspace, and both a maintainer's decision rather than a lane's.
The `code` is what an SDK branches on, and it is `checkout_not_configured`
either way.

### What ends a browser read: the clock, and `open`

Two rules that are not the same rule, and both are on the **read**.

**`expires_at` ends both reads, whatever the `status`.** The `return_token`
travels in a query string — it has to; a fragment does not survive a rail's
redirect — so a copy of it is in the rail's own storage, in whatever the rail
logs, and in the checkout app's access logs. D10's 24 hours is the bound on how
long that copy is worth anything, and until Step 9's lane 1b it bounded
nothing: `expires_at` was written at create and read by no one. Past the
horizon both reads answer the uniform 404, byte-identical to a wrong
credential's, so a stale token cannot learn that the session existed.

Deliberately not conditioned on `status`: a `complete` session's return page is
the screen the whole redirect leg exists to reach, and refusing it would break
the successful case. What ends the reads is the clock.

And deliberately **not** left to the expiry sweep
([vpay-worker.md](../vpay-worker.md#the-housekeeping-sweep-retires-a-fourth-thing-and-tells-someone-about-it)).
The sweep leaves a session with a live charge `open` on purpose, it runs at
most once an hour, and a deployment whose worker was down would keep answering
these reads for the length of the outage. The sweep makes `status` honest to a
_merchant_; the read is what refuses a payer credential.

**`status = 'open'` gates the intent's `client_secret`, and nothing else.**
That credential is handed over so the page can drive
`POST /v1/browser/payment_intents/{id}/confirm`. Once the session is `complete`
or `expired` there is nothing left to confirm, and re-issuing it on every later
read would keep a live intent credential in circulation for a checkout that is
over — reachable by anyone holding the session secret, which is in the URL the
payer was sent. A settled session still reads `200` with the outcome; the page
loses nothing, because it read the secret on its first call and polls with the
copy it holds.

### `merchant.name`, and why there is a fallback

Both browser reads render `merchant: { name }` — the one fact about the
merchant a payer is shown, and a member `frontends/apps/checkout` _requires_:
its `isSessionEnvelope` guard refuses a session envelope without it, so a
server that rendered nothing made every session read `error.unexpected`. The
field is `name` and not `display_name` because that is what the guard reads;
this is a wire contract with a TypeScript app that cannot be type-checked
against this crate, so an integration test pins the literal.

It rides on `model::CheckoutSessionForPayer`, a wrapper that flattens the
session, rather than on `CheckoutSessionObject` — `CheckoutSessionWithSecret`'s
argument pointing the other way. A field on the object would put a
deployment-configured value on all four `/v1` responses and on every row of the
list, where a merchant reading their own sessions already knows who they are.

The value is `merchant_clients[].display_name`, and when a merchant configured
none the `merchant` member is **absent** — `ResourceConfig::merchant_display_name`
answers `None` and nothing is rendered in its place. ~~The first version of this
lane fell back to the tenant id~~ (retired 2026-09-04 by the integrator, once
lane 3b made the page tolerate a missing `merchant`): a payer is asked to hand
over money on the strength of recognising who they are paying, and a tenant id
or a `client_id` is an internal identifier, not a name. The fix for a nameless
merchant is configuration, and `config/application.yml` says so beside the
field. The page paints a neutral heading for the absent case.
