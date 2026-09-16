---
name: cratestack-policy-auth
description: CrateStack access control and identity — the @@allow / @@deny policy expression language, how auth() resolves, implementing AuthProvider, CratestackContext, and the cratestack-auth crate (Ed25519 request signing, SD-JWT identity tokens, multi-issuer JWKS). Load when writing or debugging a model or procedure policy, when a read returns 404 or fewer rows than expected, or when wiring authentication into a CrateStack server.
---

# Policy and auth

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

Two dialects that look alike and are not:

- **Model / view policy** — `@@allow("action", expr)` and `@@deny(...)`, double
  `@`, compiled into **SQL** and evaluated by the database.
- **Procedure / query policy** — `@allow(expr)` and `@deny(...)`, single `@`,
  evaluated in **Rust** against the decoded arguments and the auth context.

Field-level `@allow` is rejected at parse time. It never existed and enforced
nothing.

## Deny by default

Both dialects follow the same rule:

```
deny present:  NOT (deny1 OR deny2 …) AND (allow1 OR allow2 …)
deny absent:                              (allow1 OR allow2 …)
empty allow list:                          FALSE
```

**A model with no `@@allow` for an action denies that action completely.** That
is a `FALSE` literal in the SQL, not an oversight.

## The action vocabulary

`list`, `detail`, `read`, `create`, `update`, `delete`, `all`. `read` fills both
the `list` and `detail` slots. A view supports only `read`.

## The predicate language

Terms combined with `&&`, `||` and parentheses.

| Form | Meaning |
| --- | --- |
| `auth() != null` / `auth() == null` | authenticated / anonymous |
| `auth().isSystem()` | true only for a `SystemContext`-minted context; never survives deserialisation |
| `hasRole("admin")` | role membership |
| `inTenant("acme")` | tenant membership |
| `field == auth().x` | compare a column to an auth claim (dotted paths work: `auth().organization.id`) |
| `field == "literal"` / `!=` | compare a column to a bool, int or string literal |
| `field in [A, B, C]` / `not in [...]` *(since 0.9.1)* | membership; an empty list is a compile error |
| `a.b.c` | a relation path, with quantifiers `ToOne`, `Some`, `Every`, `None` |
| a bare field | **only** for a `Boolean` + required field; anything else is a compile error |

Procedure policy adds predicates over the decoded arguments —
`InputFieldIsTrue { field }` and friends.

## The one behaviour that surprises everyone

**A denied model read is not a 403.**

Read policy becomes a `WHERE` predicate, so:

- a denied `find_unique` returns `Ok(None)`, and the generated GET handler turns
  that into **404**;
- a denied list read just returns **fewer rows, with no error at all**.

Only mutations produce `Forbidden` (403) — `"<action> policy denied this
operation"`. Mutations run a one-shot
`SELECT 1 FROM <table> WHERE pk = $1 AND (<policy>) LIMIT 1` first.

This is intentional: distinguishing "denied" from "does not exist" would be an
existence oracle. Design your clients around 404, not 403, for reads. It is also
why a `@version` mismatch re-reads through the read policy before answering —
if you cannot see the row you get `Forbidden`, keeping denials and missing rows
indistinguishable.

## Where policy is *not* enforced

- **The embedded SQLite backend.** Policies parse, malformed ones still fail
  compilation, the slots are still on the descriptor — and nothing reads them.
  Clients are untrusted; authorization is the server's job.
- **Streamed `@@subscribe` events.** A subscriber authenticates; per-row
  `@@allow` filtering is not replayed against the stream. Documented scope limit.
- **Studio's `rw` database targets.** `mode = "rw"` on a `[target.db]` is
  database-level access: Studio evaluates no policy and consults no `@@internal`
  suppression. The audit log is the compensating control, and it is detective,
  not preventive.
- **`@@internal("create")`** suppresses the *route*, not the policy. And it does
  not exempt handler-name collisions, because the handler functions are still
  emitted.

## Implementing `AuthProvider`

```rust
pub trait AuthProvider: Clone + Send + Sync + 'static {
    type Error: Into<CratestackError> + Send;
    fn authenticate(&self, request: &RequestContext<'_>)
        -> impl Future<Output = Result<CratestackContext, Self::Error>> + Send;
}
```

An `async fn authenticate` in the impl satisfies it directly — the verbose
`impl Future` form in generated docs is convention, not a requirement.

`RequestContext<'a>` gives you `method`, `path`, `query`, `headers`, `body` and
`extensions`. A plain closure
`Fn(&HeaderMap) -> Result<CratestackContext, E>` also works via a blanket impl.

Build the context with `CratestackContext::anonymous()`,
`::authenticated([(name, Value)])`, or `::from_principal(Some(p))`. Policies read
it through `ctx.auth_field(name)`, which resolves dotted paths through the auth
identity map and then the principal.

Every generated handler authenticates, then calls
`enrich_context_from_headers(ctx, &headers, trusted_proxy, peer)` before handing
`&ctx` to the delegates. That is where client IP resolution happens — see
`cratestack-data-integrity`.

`CachedAuthProvider::new(ctx)` returns a fixed context for every call. It is what
`POST /rpc/batch` uses so a signing provider authenticates the real batch
envelope once rather than per frame.

## `@default(auth().x)` is not enforcement

It fills a column on create and nothing more. A model with
`tenantId String @default(auth().tenantId)` and no tenant predicate in its read
policy is **fully cross-tenant readable**. Auth-defaulted columns are also
excluded from upsert's `DO UPDATE` set, so the update branch keeps whatever was
there — verifying that it belongs to the caller is the update policy's job.

Auth-defaulted columns are limited to `String`/`Cuid`, `Int` and `Boolean`, and
act as fallbacks: they fill the field only when the create input omits it.

## `@authorize` on procedures

`@authorize(Model, action, args.path)` runs a model's policy for you against a
primary key carried in the procedure's arguments. Actions are `detail`/`read`,
`update` and `delete` only, and the path's type must match the model's PK type.

## `cratestack-auth`

Optional, and broad. The pieces you are most likely to want:

**Ed25519 request signing** (SigV4-style canonical request) —
`sign_request(SignRequestParams)`, `SignedRequestVerifier`, `DeviceKeyResolver`,
`NonceStore` (`nonce_store_from_redis_url`), plus
`canonical_signature_base` / `canonical_query` / `content_sha256_base64url` if
you need to reproduce the canonicalisation on a client.

Bridge it in with
`SignedRequestAuthProvider::new(verifier).allow_transport_callers(mode)`, where
mode is `Never`, `SafeReadOnly` or `AllMethods`.

Env knobs: `CRATESTACK_AUTH_SIGNATURE_TRUSTED_KEYS`,
`…_TRUSTED_ISSUERS`, `…_MAX_SKEW_SECONDS`, `…_REPLAY_WINDOW_SECONDS`.

**SD-JWT identity tokens** — `issue_sd_id_token`, `IdTokenVerifier`,
`IdTokenClaims`, `decode_disclosures_unverified`. Grant constant:
`urn:cratestack:params:oauth:grant-type:id-sd-jwt`.

**JWKS** — `jwks()`, `MultiIssuerJwksVerifier`, `ServiceSigningKey`,
`mint_signed_token`, and `jwks_router()` behind the default-on `axum` feature.
`default-features = false` keeps axum out for signing-only consumers.

Bridges back into the framework: `auth_error_to_cratestack_error` and
`principal_to_cratestack_context`.

**Note on env lookups:** every one of these goes through an injectable lookup
closure rather than calling `std::env::var` directly, because
`std::env::set_var` is `unsafe` in edition 2024 and this workspace forbids
`unsafe_code`. Follow that pattern in your own config if you want it unit
testable.
