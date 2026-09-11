# `vpay-api` — resource-server JWT validation (`resource_auth.rs`)

_Moved out of [docs/reference/vpay-api.md](../vpay-api.md) on 2026-09-11 by exp57, which split a 1 566-line reference into a page per surface. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the links changed: a `../` for the new depth, and, where a cross-reference pointed at a heading that is now on another page, the page it moved to._

## Resource-server JWT validation (`resource_auth.rs`)

`docs/api/README.md` defines three surfaces with three different protections;
this module builds the validation layer for the two that need one — `/v1`
(merchant, `client_credentials` + `private_key_jwt`) and `/dash/v1` (dashboard,
a staff OIDC session, one read-only scope). See
[ADR-0009](../../adr/0009-dashboard-oidc-provider.md) (vpay runs its own Authkestra
OP) and [ADR-0010](../../adr/0010-merchant-auth-private-key-jwt.md) (merchant
auth).

**Status:** the merchant half is live and guards real rows.
`vpay_api::require_merchant_token` — which validates through
`MerchantJwtValidator` and hands `AuthenticatedMerchant` its claims — is mounted
in front of the whole `/v1` nest by `vpay_api::router`, and `vpay-server` builds
the validator behind it against this process's own JWKS. Behind that boundary is
`/v1/payment_intents`, whose every query is filtered by the tenant the
middleware resolved.

**Corrected 2026-09-06.** This paragraph used to end "the dashboard half is
unmounted — `AuthenticatedDashboard` exists, no `/dash/v1/*` route does, and
nothing constructs a `DashboardJwtValidator`". All three clauses are now
false: `require_dashboard_token` validates through a `DashboardJwtValidator`
in front of two `GET` routes, and `vpay-server` builds one whenever the
deployment registers a `dashboard_client` — see
[the dashboard surface](dashboard-surface.md#the-dashboard-surface-dash). What is still true is
the sentence that mattered until 2026-09-07: no grant this deployment served could mint a token
for that surface. `AuthenticatedDashboard`, the extractor, is mounted on
nothing and stays that way; `/dash/v1` validates once in middleware, for the
merchant boundary's reason.

### Validation is local, not a network round trip per request

`vpay_api::jwks_cache::JwksCache` fetches the JWKS once and caches it
(`jwks_refresh_interval`); every call after that looks the key up by `kid` from
memory and verifies the signature locally with `jsonwebtoken`. That cache is a
narrowed port of `authkestra_resource::jwt::JwksCache` (see
[the JWKS cache](jwks-cache.md#the-jwks-cache-jwks_cachers) for why it had to be ported) and
it keeps the original's refresh policy bar one deliberate change: `get_key` only
calls `refresh()` (an HTTP GET) on a cache miss or once the TTL has elapsed,
never on every validation, and the TTL refresh re-checks the cache once it holds
the write guard, so a boundary crossed by any number of concurrent requests
costs exactly one fetch. That is what makes this safe to put in front of a
payment-processing route.

### …except on an unrecognized `kid`, which is why this module throttles

The other half of `get_key` — carried over from the original unchanged,
deliberately (`jwt.rs` ~181-193): when the cached JWKS does **not** hold the
requested `kid`, it calls `refresh()` unconditionally — "in case of rotation" —
and `refresh()` holds the cache's write lock _across_ the HTTP GET. On top of
that, key resolution happens **before** the signature is verified, so nothing
about the token has been checked at that point.

Unthrottled, that makes an unauthenticated request with a random `kid` in its
header a remote control for two things at once: one loopback
`GET /v1/oauth/jwks.json` per request (which in this deployment is a Postgres
`SELECT`, so the amplification lands on the database), and a write lock held
across that round trip, which blocks every _legitimate_ validation in the
process for its duration. `JwtValidator::validate` closes both by deciding,
before it delegates, whether this `kid` is one the process has ever seen a valid
token for; `UNKNOWN_KID_REFRESH_INTERVAL`'s own doc comment states the throttle
and the trade-off it makes.

### A sharp edge in `jsonwebtoken`'s default audience validation

`jsonwebtoken::Validation::validate_aud` defaults to `true`, but the check it
gates only runs _if the token has an `aud` claim at all_ — confirmed by reading
`jsonwebtoken-11.0.0/src/validation.rs`: the doc comment on `validate_aud`
itself says "Validation only happens if `aud` claim is present", and the
`_ => {}` fallthrough arm of `validate`'s `match (claims.aud, options.aud.as_ref())`
proves it — a token with no `aud` claim reaches that arm and passes regardless of
what `set_audience` was told. A token minted with no audience at all would
therefore sail through unchecked, which is exactly the kind of ambiguity this
module is required to fail closed on (a missing claim, not merely a wrong one).

The fix is not to hand-roll audience comparison: `JwtValidator::new` calls
`set_required_spec_claims(&["exp", "aud", "iss"])`, which makes the `aud`
claim's mere _presence_ mandatory — a token with no audience is rejected as a
missing required claim before the comparison logic ever runs — and
`set_audience` continues to do the real membership check with the library's own
(tested) logic. Covered by `a_token_with_no_audience_claim_at_all_is_rejected`.

### `authkestra_resource::jwt::JwtStrategy` was deliberately not used

`JwtStrategy<I>`'s `cache` and `validation` fields are both private with no
accessor — confirmed by reading `jwt.rs`: neither field is `pub`, and no getter
or setter method exists for either. Once a `JwtStrategy` is built via
`ValidationConfig`/`ValidationConfigBuilder`, nothing outside the crate can
inspect or adjust its `Validation` afterwards. That is a real limitation — a
sibling project had to work around it by setting `validation.validate_aud =
false` and hand-rolling the audience check itself — but it does not bite this
module, because this module never constructs a `JwtStrategy` at all. It drives
its own `jwks_cache` (a port of the `pub` `JwksCache`/`validate_jwt_generic`
pair) and builds its own `jsonwebtoken::Validation`, so every field — including
`validate_aud` and `required_spec_claims` — stays under this module's control
from construction onward. `ValidationConfigBuilder` does expose
`.audience()`/`.audiences()`, so the _audience_ half of the sibling project's
problem would not necessarily recur through `JwtStrategy` either — but the
`required_spec_claims` fix above has no equivalent builder method at all, which
alone would have forced hand-rolling (or living with the gap) had `JwtStrategy`
been used.

---
