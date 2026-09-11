# `vpay-api` — the dashboard surface (`dash/`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## The dashboard surface (`dash/`)

Two `GET` routes under `/dash/v1`, added 2026-09-06. The product side is
[docs/flows/dashboard.md](../../flows/dashboard.md) and the authentication side
is [docs/flows/dashboard-auth.md](../../flows/dashboard-auth.md); this page
carries the shape decisions.

~~**Read the first paragraph of `dash/mod.rs` before anything else**: no
client of this deployment can obtain a token for this surface.~~ **Corrected
2026-09-07** ([ADR-0017](../../adr/0017-staff-authentication.md)): the
authorization-code grant _is_ served, for the dashboard client and nothing
else, and `vpay_api::staff` is the eight unauthenticated routes that produce
the `Identity` `authkestra-op` takes as a parameter and authenticates nobody
for. What follows describes a resource server that now has an issuer.

What is still true is that **nothing a person can click reaches any of it**:
`frontends/apps/dashboard` is the scaffold. See
[docs/flows/dashboard.md](../../flows/dashboard.md)'s "There are still no pages".

Its own module rather than routes under `v1` for the reason `browser` is one:
it is a different credential, a different tenancy rule and a different
authentication boundary. A route table shared with `/v1` would be one
`V1Route` entry away from a merchant token reaching a staff read — and
`V1_ROUTES` is walked by a boundary test that asserts every entry answers
`401` without a token, which `DASH_ROUTES` must also do but for a different
validator.

### What identifies the credential (ADR-0017), and what it replaced

`require_dashboard_token` checks three things about a validated token: the
**audience** (through the validator), the **merchant claim**, and the
**scope**.

The audience is the registered `dashboard_client.client_id`. It was the
constant `vpay:dash/v1` until 2026-09-07, and the constant could not survive a
login: `default_handle_authorization_code` mints with `aud = <client_id>` and
has **no requested-audience path at all**
(`authkestra-op-0.7.1/src/handlers/token.rs`, step 7), so no token from the
only grant that can produce a dashboard credential would ever have carried it.
ADR-0017 changed the _validator_ to expect what the grant produces rather than
forking the handler, and `Surface::audience()` is gone —
`JwtValidator::new` takes the audience as a value, because it is configuration
now and not a constant.

**The merchant claim is what this middleware adds.** The token must carry
`vpay_config::DASHBOARD_MERCHANT_CLAIM` and it must equal
`dashboard_client.merchant_id`. Two things follow, and both are the point:

- a `client_credentials` token carries no such claim, because nothing but the
  staff grant stamps one — so **no machine client may read this surface**, by
  construction rather than by a list. That is a tightening over what stood
  before, and `a_client_credentials_token_is_refused_on_dash_v1` pins it;
- the claim is _compared_, never _used_. The `MerchantScope` still comes from
  the binding, so a forged claim buys a `403` rather than another merchant's
  rows.

**What this replaced**, recorded because the replacement is the interesting
part: the middleware used to compare the token's `sub` to `binding.client_id`.
That is right under `client_credentials`, where
`TokenManager::issue_client_token` sets `sub` to the client id — and wrong for
every token a real staff login produces, where `sub` is the **staff member**.
The 2026-09-06 review measured it as finding F7 and left it, because which
claim identifies the credential was reserved for the maintainer. ADR-0017
takes it; `ResourceClaims::client_id` is renamed to `subject` so nothing can
read it as a client id again without saying so.

The boot rule
(`vpay_config::ConfigError::MerchantClaimsDashboardAudience`) is the other
half and neither is sufficient alone: it stops a _merchant_ registration from
being able to request the dashboard client's id at `/v1/oauth/token` at all.
That was a real hole before 2026-09-06 — `handle_client_credentials` honours
any requested audience `allowed_audiences` permits, and nothing restricted
what else a merchant could list beside `vpay:v1`. It was harmless only
because `/dash/v1` mounted nothing. The rule moved to whole-document scope in
the same change, because its forbidden value is no longer a constant one
registration could compare itself against.

Any method but `GET`/`HEAD` is refused in the same middleware, **before** the
router matches. Relying instead on `DASH_ROUTES` mounting no `post(..)` would
make "the dashboard cannot write" a property of a route table, and a route
table is the thing a future slice edits. ADR-0008 requires an `audit_log` row
per dashboard write and none exists, so a write that arrived here must not
reach a handler at all.

### The staff routes are outside the token layer, and the nest's fallback is not

`crate::router` builds the `/dash/v1` nest as `dash::routes()` wrapped in
`require_dashboard_token`, then `.merge(staff::routes())`. The merge is
**after** the layer, so the eight staff routes are outside it — they have to
be, because they exist to produce the credential that layer checks, and a
`/dash/v1/staff/login` behind a bearer-token requirement is a login nobody can
reach.

`Router::layer` wraps the routes _and the fallback_ present when it is called,
so `dash::routes`' own `.fallback(not_found)` stays inside: an unmatched
`/dash/v1/...` path answers exactly what it answered before this module
existed. `staff::routes` deliberately carries no fallback of its own, because
two routers with fallbacks cannot be merged.

The staff routes are mounted whether or not `staff_auth` is configured. A
deployment with a dashboard and no secrets answers the honest 404 through
`AppState::staff_login`, rather than the paths vanishing — a 404 on
`/dash/v1/staff/login` and a 404 on `/dash/v1/nonsense` are the same answer,
and making a route table depend on a Secret's presence is how a deployment
discovers a missing Secret by reading a route table.

### Why an absent registration mounts nothing

A deployment with no `dashboard_client` gets **no `/dash/v1` nest**, so every
path under it is the outer honest 404. A mounted nest whose middleware
refused everything would answer 401 and invite a caller to go looking for a
credential this deployment could never issue — and it would make "I forgot to
configure the dashboard" indistinguishable from "my token is wrong".

`require_dashboard_token` _also_ answers 404 when the validator or the
binding is absent. The two are redundant on purpose and the redundancy was
measured: mutating either one alone leaves the 404 intact, and only mutating
both makes `a_deployment_with_no_dashboard_client_mounts_no_dash_nest` fail
(2026-09-06). Neither is deleted, because a router assembled by a future
binary is not obliged to consult the first and the second is what holds if it
does not.

### The wire shapes, and why they are not Stripe's

The list is the ordinary `ListObject` envelope over the ordinary
`PaymentIntentObject`, deliberately: an operator and a merchant looking at
the same payment must not be told different things about its status or its
amounts, and two renderers would be two chances to diverge.

The **detail** is vpay's own shape, `object: "dashboard.payment_detail"`.
Stripe has no "everything about this payment" resource, and inventing one
under the merchant API's vocabulary would make an SDK author reasonably
expect it there. `docs/sdks/parity.md` does not cover these routes and must
not: a `/dash/v1` method in a merchant SDK would be a merchant credential
reaching for a staff surface.

Three things the detail deliberately omits:

- **`client_secret`.** `GET /v1/payment_intents/{id}` renders it so a
  merchant who lost the create response can recover it. A staff reader has no
  such need, and the value authorises confirming the payment from any browser
  (`vpay_api::browser`). The dashboard observes the object, not the
  credential that spends it. Asserted over the serialised body, not over the
  type, because rendering `PaymentIntentWithSecret` instead is a one-word
  change that still compiles.
- **The event `data` snapshot.** A timeline row carries the event's id, type,
  `object_id` and time. The snapshot is the whole object as it was at emit
  time, so rendering it would put a second, older payment intent inside a
  response whose first field is the current one, with nothing to tell a
  reader which is which. `GET /v1/events/{id}` is where the snapshot lives.
- **The charge's unmasked `payer_ref`, `provider_ref_extra`, `redirect_url`
  and `return_url`.** A rail's own blob and a payer's browsing path are not
  what "which rail, what did it say, when" needs.

`payer_ref_masked` **is** rendered, and is `null` on every row this system
has ever written — nothing populates the column (`open_attempt` stores
`None`). It is rendered from the column rather than derived in the renderer
on purpose: deriving a mask would mean reading the _unmasked_ value into a
staff surface, and the failure mode to avoid is this field quietly becoming
the unmasked number because the masked one was empty.

The filter predicates live in the SQL statement rather than in the handler,
because a caller cannot filter a page: `list_page` fetches `limit + 1` rows
to learn `has_more`, so a filter applied to the returned `Vec` would drop
rows out of a page that has already been counted and the next cursor would
skip them. `PaymentIntents::list_page` is `list_page_filtered` with an empty
filter — one statement, so the two surfaces cannot disagree about the cursor.

An unknown `status` is a `400` naming `status`, not an empty page. "No
payments are `succeded`" is a sentence an operator reads as an answer about
their payments rather than about their typo.
