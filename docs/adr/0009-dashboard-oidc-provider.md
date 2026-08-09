# ADR-0009: vpay runs Authkestra as its own OpenID Provider for `/dash/v1`

- **Status:** Accepted
- **Date:** 2026-08-09
- **Deciders:** vpay maintainers

## Context

ADR-0008 decided the dashboard authenticates with "OIDC sessions, never an API
key," but left open which OP issues those sessions and who runs it. This ADR
answers that: vpay runs [Authkestra](https://github.com/marcjazz/authkestra)'s
`authkestra-op` in-process as its own OP, rather than federating to an external
IdP or building one from scratch. Authkestra is the user's own project,
published on crates.io as `authkestra*`, dual-licensed MIT OR Apache-2.0.

## Decision

Staff log in to `/dash/v1` via authorization-code + PKCE. vpay hosts
`authkestra-op`'s handlers (`/authorize`, `/token`, `/userinfo`,
`/.well-known/openid-configuration`, `/jwks.json`) itself, issues its own
access and ID tokens, publishes its own JWKS, and owns signing-key generation
and rotation. This is the concrete mechanism behind ADR-0008's "OIDC
sessions" sentence, not a change to it.

Persistence goes through `authkestra_op::sqlx_store::SqlxOpStore`, a
ready-made SQLx-backed implementation of `OpStore` (client lookup,
authorization-code storage, refresh-token storage, device-code storage) —
confirmed by reading `crates/authkestra-op/src/sqlx_store.rs` in the local
`~/dev/authkestra` checkout. vpay does not need to hand-write a store; it
needs schema (`oauth_clients`, `oauth_codes`, …) and wiring once a database
layer exists.

**Scope boundary — what this does not cover:**

- `/v1`, the merchant API, keeps Stripe-shaped opaque `sk_live_`/`sk_test_`
  bearer keys and does **not** move to Authkestra. Merchants expect that
  credential shape, Authkestra has no opaque-API-key primitive, and routing
  merchant auth through an OP that itself has no revocation endpoint (see
  below) would reimport the exact problem ADR-0008 exists to avoid.
- `/provider/{code}/callback` stays public and unauthenticated by necessity —
  a payment rail cannot present an OIDC session.

## Consequences

**No revocation endpoint is a real, not hypothetical, gap.** Reading
`crates/authkestra-op/src/handlers/mod.rs` confirms the exposed surface is
discovery, jwks, authorize, token, userinfo, device_authorization,
device_verify and enrolment — there is no `/revoke` handler.
`RefreshTokenStore::revoke_token` exists, but it is an internal store method
the OP itself never calls from an HTTP endpoint, not an RFC 7009 revocation
endpoint. ADR-0008's entire argument for sessions over API keys was that
bearer credentials have "no expiry or revocation story" — an OP with no
revocation endpoint partially reintroduces that for dashboard sessions
themselves. The mitigation this decision implies is short access-token TTLs
(`OpConfig::access_token_ttl_secs` exists as a knob) and/or a server-side
deny-list. **Which of these vpay will actually build is not decided by this
ADR** — it is available, not chosen. Treat it as unresolved until a follow-up
either picks one or supersedes this ADR.

**vpay is the first project to actually build this flow.** vsms — the only
other project running `authkestra-op` against real traffic, pinned at
`=0.3.3` — never built human login: its README states plainly that
"Human login (authorization code + PKCE) is designed but not built — nothing
needs it until the admin console does." vsms's shipped auth path is
`client_credentials` + `private_key_jwt` for machine callers, a different
code path (`token.rs`'s client-credentials arm, not `authorize.rs`). This ADR
has no sibling-project prior art to lean on for the exact flow it commits to.

**vsms's own open question is relevant and unresolved for vpay too.** Its
`AGENTS.md` open question #4 records, as of that writing: `authkestra-op`
"needs a custom `ClientStore` to work around a grant-type authorisation bug,
plus `/token` rate limiting it lacks, with no revocation endpoint and no
proof-of-possession. Compare against Keycloak / ZITADEL before milestone 1,
not after." vsms's own later notes say the `GrantType` `#[serde(untagged)]`
bug is fixed upstream in `0.3.2` and that `/token` rate limiting is treated
as a reverse-proxy concern rather than a blocker — but the local
`~/dev/authkestra` checkout read for this ADR is pinned at `0.2.4`, so the
fix was not independently re-verified here. The Keycloak/ZITADEL comparison
vsms recommended has not happened for vpay. This ADR proceeds without it, on
the basis that Authkestra is the user's own project and is meant to be
improved rather than swapped out.

**This sits on top of a database connectivity layer that does not exist.**
The schema itself has since landed in full — `docs/status.md` now lists eight
migrations applied to a real Postgres with their constraints proven to fire,
including the three this ADR specifically anticipated:
`authkestra.oauth_clients`/`oauth_codes`/`oauth_refresh_tokens`/`oauth_device_codes`
(migration `0006`, transcribed verbatim from `authkestra-op` `=0.3.4`'s own
`SqlxOpStore::migrate()`) and `oauth_signing_keys` (migration `0007`, vpay's
own — Authkestra ships no signing-key type, confirmed by the same
source-reading method this ADR used throughout). A new acceptance test,
`backends/tests/integration/tests/authkestra_op_smoke.rs`, drives the real
`SqlxOpStore<Postgres>` against migration `0006` and proves `find_client`,
`store_code` and `consume_code` (including its single-use enforcement) all
work against it — but that test runs `SqlxOpStore` directly inside a
dev-only test crate; it is not the same thing as a usable database layer
reachable from a shipping binary. No crate in `vpay-server` or
`vpay-worker-bin`'s dependency graph opens a connection pool, runs a query,
or consumes `hmac`, `sha2`, `subtle` or `aes-gcm` — the latter four still
exist only as unused version pins in the root `Cargo.toml`. `SqlxOpStore`
cannot be wired up, and no dashboard-auth code can be written, until a
connectivity/repository layer exists on top of the schema. This ADR is a
decision made ahead of its own prerequisite on purpose: it sets the shape so
the database work can build toward it directly, but nothing here is
buildable yet.

**A TLS crypto-provider collision was a known constraint at the time this
ADR was written; it is now resolved, and observed to hold, not merely
argued.** vpay pins `rustls` to the `ring` crypto provider (`Cargo.toml`:
`rustls = { features = ["ring", …] }`). `authkestra-op` and
`authkestra-engine` are now real dependencies (`[dev-dependencies]` of
`vpay-tests-integration`, pinned `=0.3.4` with `default-features = false,
features = ["rustls-no-provider"]` in the root `Cargo.toml`), and `deny.toml`
now bans `aws-lc-rs`/`aws-lc-sys` outright (`[[bans.deny]]`) so a second
default rustls crypto provider cannot silently reappear. Both are verified,
not assumed: `cargo tree -i ring` shows `ring` present in the resolved graph
(pulled in via `rustls`) and `cargo tree -i aws-lc-rs` finds no matching
package at all — `aws-lc-rs` is absent from the graph entirely, not merely
unused — and `cargo nextest run --workspace` (78 passed, 3 skipped,
including `authkestra_op_smoke.rs`) built and ran the full dependency graph
with `authkestra-op` present without a provider collision. This closes the
open question this ADR originally left unresolved — the caveat now is
narrower: this was verified with `authkestra-op` as a *dev*-dependency of
one test crate, not yet with it linked into `vpay-server` itself; re-check
this once that happens.

**This supersedes nothing in ADR-0008.** The dashboard-acts-on-records,
never-on-configuration boundary is unchanged; this ADR only decides how the
"OIDC sessions" half of that boundary is implemented.
