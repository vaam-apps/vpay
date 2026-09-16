---
name: cratestack-server
description: Building a CrateStack HTTP server — include_server_schema! with db = Postgres or db = None, the generated Cratestack runtime and Axum routers, AuthProvider wiring, the REST query-parameter contract, error envelope, codecs, events and the transactional outbox, idempotency and rate-limit layers. Load when working in a crate that depends on package = "cratestack-pg" or "cratestack-api", or when a question concerns generated routes, 403/404/422 behaviour, list filters, or router() arguments.
---

# Server (`include_server_schema!`)

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

```toml
# owns a Postgres database
cratestack = { package = "cratestack-pg", version = "0.12" }
# procedures only, no database at all
cratestack = { package = "cratestack-api", version = "0.12" }
```

```rust
include_server_schema!("schema.cstack", db = Postgres);
include_server_schema!("schema.cstack", db = None);
```

Third, optional argument: `decimal = RustDecimal | BigDecimal`, **required** if the
schema declares a `Decimal` anywhere. Those three are the only accepted
arguments.

The macro checks that `db = …` and the schema's own
`datasource { provider = … }` agree (`Postgres` ↔ `"postgresql"`, `None` ↔
`"none"`), and that the facade you depend on can actually satisfy it — asking for
`db = Postgres` under `cratestack-api` is one clear error rather than a wall of
missing-`sqlx` errors.

## The minimal server

```rust
use cratestack::axum::Router;
use cratestack::include_server_schema;
use cratestack::sqlx::PgPool;
use cratestack::{AuthProvider, CratestackContext, CratestackError, RequestContext, Value};
use cratestack_codec_json::JsonCodec;

include_server_schema!("examples/server_basic.cstack", db = Postgres);

#[derive(Clone)]
struct HeaderAuthProvider;

impl AuthProvider for HeaderAuthProvider {
    type Error = CratestackError;

    async fn authenticate(&self, request: &RequestContext<'_>)
        -> Result<CratestackContext, Self::Error>
    {
        let mut fields = Vec::new();
        if let Some(id) = request.headers.get("x-auth-id").and_then(|v| v.to_str().ok()) {
            fields.push(("id".to_owned(), Value::Int(id.parse().map_err(
                |e: std::num::ParseIntError| CratestackError::BadRequest(e.to_string()))?)));
        }
        Ok(if fields.is_empty() {
            CratestackContext::anonymous()
        } else {
            CratestackContext::authenticated(fields)
        })
    }
}

#[derive(Clone)]
struct Procedures;
impl cratestack_schema::procedures::ProcedureRegistry for Procedures {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = PgPool::connect(&std::env::var("DATABASE_URL")?).await?;
    let db = cratestack_schema::Cratestack::builder(pool).build();

    let app: Router = cratestack_schema::axum::router(
        db,
        Procedures,
        (),                                  // computed-field resolver
        JsonCodec,
        HeaderAuthProvider,
        cratestack::DEFAULT_BODY_LIMIT_BYTES,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    cratestack::axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
```

`async fn authenticate` satisfies the trait directly. The `()` in third position
is the computed-field resolver — the macro emits `impl ComputedFieldResolver for ()`
only when the schema declares zero `@computed` fields.

Under `db = None` the builder takes **no arguments**:
`cratestack_schema::Cratestack::builder().build()`.

## What you get, by name

Everything lands in `pub mod cratestack_schema`. For a `model Article`:

| Name | What it is |
| --- | --- |
| `Article`, `ArticleBuilder` | the row struct (field names verbatim from the schema) |
| `CreateArticleInput`, `UpdateArticleInput` | inputs, with `validate()` |
| `ArticleWhere`, `ArticleSortField`, `ArticleOrderByClause`, `ArticleFindManyInput` | typed query surface |
| `models::ARTICLE_MODEL` | the `ModelDescriptor` |
| `article::<field>()` | `FieldRef` accessors and relation paths |
| `events::ArticleCreatedEvent` … | under `db = Postgres` with `@@emit` |

Runtime, under `db = Postgres`: `Cratestack::builder(pool)` →
`.with_audit_sink(…)` → `.build()`. Then `db.pool()`, `db.transaction(|tx| …)`,
`db.events()`, `db.views()`, `db.queries()`, `db.bind_context(ctx)`,
`db.bind_auth(principal)`, and one delegate accessor per model.

**Under `db = None`, most of that does not exist** — no `pool()`, no
`transaction()`, no `events()`, no `views()`, no `queries()`, no
`with_audit_sink()`. They are omitted entirely rather than stubbed, which is the
right call but does mean "method not found" is the expected error, not a bug.

Routers: `router(db, registry, resolvers, codec, auth_provider, body_limit_bytes)`,
plus `model_router(db, resolvers, codec, auth_provider)` (Postgres only),
`procedure_router(db, registry, resolvers, codec, auth_provider)`, and
`rpc_router(…)` under `transport rpc`. `body_limit_bytes` is applied as the
outermost `DefaultBodyLimit` and **cannot be overridden by re-layering**.
`DEFAULT_BODY_LIMIT_BYTES` is 2 MiB.

## Routes

`<plural>` = `pluralize(snake_case(ModelName))`, so `Article` → `/articles`.

| Method | Path | Success |
| --- | --- | --- |
| GET | `/<plural>` | 200 |
| POST | `/<plural>` | **201** |
| GET | `/<plural>/{id}` | 200 |
| PATCH | `/<plural>/{id}` | 200 |
| DELETE | `/<plural>/{id}` | 200 |

Procedures: `POST /$procs/<procedureName>` — the procedure name **verbatim**, not
snake-cased — or `POST /<version>/$procs/<name>` with `@api_version`. The request
body is `{"args": {…}}`.

A verb suppressed with `@@internal("create")` is never registered, so it is a
plain axum 404 with no fallback.

## The list-route query contract

```
?limit=50&offset=100
&fields=id,title
&include=author,author.org
&includeFields[author]=id,email
&sort=-createdAt,title            (or orderBy=, but never both)
&where=(a=1,b=2)|not(c=3)         ( , = AND   | = OR   () groups )
&or=a=1|b=2                       (legacy)
&computedParams={"field":{…}}
&<field>__<op>=<value>            (everything else)
```

Filter operators, by field shape:

- always: `eq`, `ne`, `in` (comma-separated). No `__` suffix means `eq`.
- comparable fields: `lt`, `lte`, `gt`, `gte`
- `String` / `Cuid`: `contains`, `startsWith`
- optional fields: `isNull` (value parsed as a bool; `false` means `IS NOT NULL`)

Relation filters use the path: `?author.email__eq=x@y.z` for to-one, and a
**mandatory quantifier** for to-many — `?comments.some.body__contains=foo`, or
`.every.`, or `.none.`. Omitting the quantifier is a 400 that names the three.

Two things that surprise people: **an omitted `limit` is coerced to
`MAX_LIST_LIMIT` (1000)**, not to "unbounded"; and the fetch route
(`GET /<plural>/{id}`) accepts **only** `fields`, `include`, `includeFields[…]`
and `computedParams` — anything else is a 400.

A `@@paged` model returns
`{ items, totalCount, pageInfo { limit, offset, hasNextPage, hasPreviousPage } }`,
where `totalCount` is a real `COUNT(*)` reusing the page query's exact WHERE and
policy scope.

## Errors

```json
{ "code": "VALIDATION_ERROR", "message": "…", "details": null }
```

Encoded with the same negotiated codec as a success body.

| Variant | code | status |
| --- | --- | --- |
| `BadRequest`, `Codec` | `BAD_REQUEST`, `CODEC_ERROR` | 400 |
| `Unauthorized` | `UNAUTHORIZED` | 401 |
| `Forbidden` | `FORBIDDEN` | 403 |
| `NotFound` | `NOT_FOUND` | 404 |
| `NotAcceptable` | `NOT_ACCEPTABLE` | 406 |
| `Conflict` | `CONFLICT` | 409 |
| `PreconditionFailed` | `PRECONDITION_FAILED` | 412 |
| `UnsupportedMediaType` | `UNSUPPORTED_MEDIA_TYPE` | 415 |
| `Validation` | `VALIDATION_ERROR` | **422** |
| `TooManyRequests` | `TOO_MANY_REQUESTS` | 429 |
| `Database`, `Internal` | `DATABASE_ERROR`, `INTERNAL_ERROR` | 500 |
| `Unavailable` | `UNAVAILABLE` | 503 |

4xx messages are the caller-supplied string; **5xx messages are canned** and the
real detail goes to tracing only. `details` is always `null` on the wire.

**A denied model read is not a 403.** Read policy compiles into the `WHERE`
clause, so a denied `find_unique` returns `Ok(None)` and the handler turns that
into a **404**; a denied list read simply returns fewer rows and no error at all.
Only mutations produce `Forbidden`. Design your clients accordingly — and see
`cratestack-policy-auth`.

## Codecs

`JsonCodec` is `application/json`, `CborCodec` is `application/cbor`. Pass one,
or pass `CodecSet::new(CborCodec, JsonCodec)` — the set holds **exactly two**;
there is no N-codec variant.

Content negotiation is real: request decode is driven by `Content-Type` against
the route's declared request types (415 on a mismatch), and the response type is
negotiated from `Accept` with proper `q=`/specificity scoring, **intersected with
what the codec you actually passed can encode** (406 if nothing matches). All of
it runs as a preflight *before* any handler side effect, so a `create` cannot
write a row and then fail to encode.

`application/cbor-seq` is response-only and procedures-only, and requires a CBOR
codec.

**The practical trap:** generated `transport rpc` TypeScript clients default to
native CBOR. A router mounted with `JsonCodec` alone answers those 406/415. Mount
`CodecSet::new(CborCodec, JsonCodec)`, or generate with `--no-native-cbor`.

## Events and the outbox

`@@emit(created, updated, deleted)` generates, inside `cratestack_schema::events`:
type aliases `ArticleCreatedEvent` and friends, and a `Subscriptions` handle:

```rust
db.events().on_article_created(|event| async move { /* … */ Ok(()) });
db.events().drain().await?;
```

Writes enqueue an outbox row **inside the mutation's transaction**; `drain()`
flushes undelivered rows — which is also the required opt-in after a
caller-managed `run_in_tx` commit.

`@@subscribe` (RPC only) adds an SSE endpoint. **Row-level `@@allow` policy is
not replayed against streamed events** — header auth applies, per-row filtering
does not. Deliberate and documented; do not stream a model whose rows are
per-caller sensitive.

**`cratestack-outbox` is a different thing.** It is a separate, hand-written
transactional outbox for *application* domain events; it deliberately does not go
through `include_server_schema!`. Copy `OUTBOX_EVENTS_DDL` into your own
migration, then use `OutboxClient::{persist, persist_in_tx, drain, gc_older_than}`
and mount `axum_handler::{drain_handler, gc_handler}` behind your own auth — the
crate has no auth opinion.

## Idempotency and rate limiting

```rust
let store = Arc::new(SqlxIdempotencyStore::new(pool.clone()));
let router = generated.layer(IdempotencyLayer::new(store, Duration::from_secs(24 * 3600)));

let store = Arc::new(InMemoryRateLimitStore::default());
let router = router.layer(RateLimitLayer::new(store, RateLimitConfig::new(100, 1.0)));
```

Three things bite:

1. **The default principal fingerprint hashes `Authorization`, falling back to
   `ConnectInfo<SocketAddr>`. With neither, the request is refused with 412.**
   Cookie or mTLS deployments must supply `.with_principal_fingerprint(…)`, or
   serve via `into_make_service_with_connect_info::<SocketAddr>()`.
2. **A nested router needs the `_with_prefix` resolver variants**
   (`build_rest_op_resolver_with_prefix`, `build_rpc_op_resolver_with_prefix`), or
   every descriptor lookup misses and `@no_idempotency` silently no-ops.
3. **Rate-limit store failure is nuanced by design**: a transport-class failure
   (Redis connection dropped) fails **open** with a warning; a store that is
   reachable and refusing (OOM) fails **closed** under every policy.

Replay semantics: same key and same body hash replays the stored response; same
key with a different body is a **422** with code `idempotency_key_conflict`.

Redis-backed stores live in `cratestack-redis`; the Postgres idempotency store is
in `cratestack-sqlx`. `cratestack-exec`'s `OpExecutor` is the transport-neutral
layer underneath — you rarely name it directly.

## Service scaffolding and observability

`cratestack-service` gives you `telemetry::init(prefix)`,
`ServiceConfig::from_env(prefix, name, default_port)` (reading
`{prefix}_SERVICE_HOST`, `_SERVICE_PORT`, `_PUBLIC_BASE_URL`, `_DATABASE_URL`,
`_REDIS_URL`, `_ENV`, `_LOG_FORMAT`), `health::router()` mounting `/healthz` and
`/healthz/ready`, and `run(router, &config)`.

**`cratestack-service::run` has no graceful shutdown.** It serves until the
process is killed. If you need it, call
`axum::serve(listener, app).with_graceful_shutdown(signal)` yourself.

Generated routes carry `info_span!`s — but **only the list route and the
procedure routes**. GET/POST/PATCH/DELETE model handlers emit events, not spans.
Every failure branch logs at `WARN` with `cratestack_error` and
`cratestack_detail`. Nothing installs a subscriber for you.

## `db = None` in one paragraph

Procedures, types, enums, `auth` blocks, procedure policies, both transports,
codecs, idempotency and rate limiting, tracing. **No `model` blocks** — rejected
at parse time, not at codegen — and no `query` blocks. Prefer the
`cratestack-api` facade, which has no `cratestack-sqlx` dependency under any
feature, so the mistake is a clear error rather than a link failure.
