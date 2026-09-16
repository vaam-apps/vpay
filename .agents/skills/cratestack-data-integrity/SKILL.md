---
name: cratestack-data-integrity
description: CrateStack's data-integrity surface — idempotency keys, rate limiting, optimistic locking with @version and If-Match, @@audit logging and redaction, @@soft_delete, upsert and conflict targets, batch operations, @isolation, trusted-proxy client IP, @@paged, composite keys, computed fields and multi-tenancy. Load for money-path or compliance-shaped work, or when a question mentions Idempotency-Key, 412, ETag, tombstones, DO UPDATE, audit redaction, X-RateLimit headers, or tenant scoping.
---

# Data integrity

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

CrateStack calls this "banking readiness". Most of it is real and test-backed.
Some of it is declared-but-inert, and the difference is the single most important
thing on this page.

## Three tiers — know which one a feature is in

**Wired and enforced.** Idempotency, rate limiting, optimistic locking, audit,
soft delete, pagination, size bounds, trusted proxy, computed fields.

**Shipped but not wired to any route.** The five `batch_*` ORM primitives
(`batch_get`, `batch_create`, `batch_update`, `batch_delete`, `batch_upsert`) are
complete — per-item expected version, per-item policy, savepoint isolation, a
designed response envelope — and are referenced **nowhere** in the codegen. They
are callable only from hand-written Rust inside a procedure. The only batch
*endpoint* that exists is `POST /rpc/batch`. Composite `@@id([...])` similarly
parses and emits correct DDL, then is rejected by every entry macro.

**Validated and then discarded.** `@isolation("serializable")` records nothing
anywhere — no generated accessor, no auto-wrapping; you must call
`run_in_isolated_tx` by hand. `@@retain(days: N)` lands on the descriptor and
**nothing ever reads it** — there is no GC job.

## Idempotency

`@no_idempotency` *(actually enforced since 0.11.1, when admission moved into the
L3 `cratestack-exec` crate)* is a bare procedure attribute (`@no_idempotency(false)` is
rejected — it would read as re-enabling while doing the opposite). A `procedure`
(query kind) is idempotent by default; a `mutation procedure` is not. Model
reads are idempotent, model writes are not.

Wire behaviour, with `IdempotencyLayer` installed:

| Situation | Response |
| --- | --- |
| Replay (same key, same body hash) | Original status and **every** original header, plus `Idempotency-Replayed: true` |
| Same key, different body | **422**, code `idempotency_key_conflict` |
| Reservation still in flight | **409** plus `Retry-After: 1` |
| Body over 2 MiB with a key | **400** (not 413 — two source comments say 413 and are wrong) |
| No `Authorization` and no `ConnectInfo` | **412** |

The fingerprint is SHA-256 over `method \0 path_and_query \0 content_type \0 body`.
**The query string is included on purpose** — `POST /transfer?dry_run=true` must
not collide with `?dry_run=false`.

Only `POST`, `PATCH`, `PUT`, `DELETE` reach the logic. Keys must be ASCII,
non-empty, and at most 255 characters.

Three traps:

1. **The default principal fingerprint fails closed.** It hashes `Authorization`,
   falls back to the `ConnectInfo` peer IP, and **412s if it has neither**. No
   example in the repo serves via `into_make_service_with_connect_info`, so a
   cookie- or mTLS-authenticated deployment 412s every request until you supply
   `.with_principal_fingerprint(...)`.
2. **A nested router needs `build_rest_op_resolver_with_prefix` /
   `build_rpc_op_resolver_with_prefix`.** Otherwise every lookup misses and
   `@no_idempotency` silently does nothing.
3. **The exemption is wider than the attribute.** Installing a resolver exempts
   everything flagged idempotent-by-default, which includes every read op. Under
   `transport rpc` reads arrive as `POST /rpc/{op_id}` and really do reach the
   layer. Nothing stops a query `procedure` from writing — if one does, the
   resolver has just removed its protection.

Store contract worth knowing if you implement one: `reserve_or_fetch` must be
concurrent-safe (two simultaneous callers see exactly one `Reserved`), and
reclaiming an expired row **must rotate the reservation token**. `complete`
freezes the outcome — a 5xx replays as the same 5xx until a fresh key is used.

## Rate limiting

`@no_rate_limit` is bare and **requires `extension rate_limit { }`** in the
schema. Token bucket: `RateLimitConfig::new(burst, refill_per_second)`.

Allowed responses carry `X-RateLimit-Limit` and `X-RateLimit-Remaining`. A 429
carries `Retry-After` and **no budget headers**. There is **no
`X-RateLimit-Reset`** anywhere.

Store-failure policy is deliberately asymmetric: a **transport-class** failure
(Redis connection dropped) fails **open** with a warning, because nobody caused
it and it self-heals. A store that is **reachable and refusing** (Redis `OOM`,
which any unauthenticated caller can induce) fails **closed under every policy**.
`with_store_error_policy(StoreErrorPolicy::Deny)` makes even transport failures
refuse.

Bucket cardinality is bounded atomically in the store, not in the layer
*(since 0.11.1, breaking — #871)*:

| Request carries | Bucket key | Scope | Cap | Fallback |
| --- | --- | --- | --- | --- |
| `VerifiedPrincipal` extension | `princ:<sha>` | — | none | — |
| `Authorization` + `ConnectInfo` | `auth:<sha>` | `peer:<addr>` | 128 | `ip:<addr>` |
| `Authorization` only | `auth:<sha>` | `global` | 8192 | `overflow` |
| `ConnectInfo` only | `ip:<addr>` | — | none | — |
| neither | **refused, 412** | | | |

Past the cap it **collapses into the fallback bucket, never refuses** — refusing
would hand an attacker a deterministic global outage. IPv6 is aggregated to /64;
IPv4 is not. Redis Cluster is not a supported deployment for this store (three
un-hash-tagged keys give `CROSSSLOT`).

Two gaps: **`/rpc/batch` is always rate-limited wholesale**, regardless of the
ops inside — the filter runs before the body is decoded, and that is an accepted
tradeoff, not a bug. And there is **no `_with_prefix` variant of the rate-limit
filters**, so a nested `/api/rpc/...` mount fails closed and silently makes
`@no_rate_limit` inert.

## Optimistic locking

`@version` on a required `Int` field, at most one per model, never the PK. Zero
is fine and normal — "exactly one per model" is a docs-site error.

Flow: read, capture the `ETag` (a strong validator, `"7"`), send it back as
`If-Match` on `PATCH` or `DELETE`, retry on 412. **Omitting `If-Match` on a
versioned model is itself a 412.** `If-Match: *` is a 400.

On a version mismatch the runtime re-reads the row **through the read policy**
before answering. If you can see it and the version differs, you get 412 with
`"version mismatch: expected N, found M"`. If you cannot see it, you get
`Forbidden` — so **policy denials stay indistinguishable from missing rows**.

`update_many` has no `if_match` slot at all and bumps every matched row
unconditionally; bulk update is not an optimistic-locking idiom. `upsert` bumps
the version but **does not honour `if_match`** — for "update only if version = N"
use an explicit transaction with `find_unique` then `update.if_match(N)`.
`batch_update` does carry a per-item expected version.

## Audit

`@@audit` is bare. **There is no duplicate check**, so a second `@@audit` is a
silent no-op.

Rows go to `cratestack_audit`, inserted **inside the mutation's transaction** —
you can never see a committed row whose audit entry did not also commit.
Columns: `event_id, schema_name, model, operation, primary_key, actor, tenant,
before, after, request_id, occurred_at, delivered_at, attempts, last_error`.

Redaction markers, exactly:

| Attribute | Marker |
| --- | --- |
| `@pii` | `[redacted-pii]` |
| `@sensitive` | `[redacted-sensitive]` |
| `@server_only` | omitted entirely (the field is `skip_serializing`) |

**The docs site says `<redacted: pii>` and `<redacted: sensitive>`. Both are
wrong.** Match on the bracket forms. Redaction happens before the row is written,
so the value is unrecoverable from the audit table. Both the snake_case column
name and its camelCase transposition are redacted.

`AuditSink` fan-out happens **after** the owning transaction commits, never
inside it. A caller-managed `run_in_tx` cannot dispatch for you — it returns the
`AuditEvent`s and you call `dispatch_audit_sink` after your own commit. The
database table is canonical; the sink is a best-effort projection.

## Soft delete

`@@soft_delete` is bare, no duplicate check. The column is **hardcoded
`deleted_at`** — you cannot name a different one. No schema change is needed
beyond the attribute.

`DELETE` becomes `UPDATE … SET deleted_at = NOW()` (plus a `version` bump if the
model has one). Every read injects `deleted_at IS NULL` as the first `WHERE`
clause — but **a view's own SQL body must filter tombstones itself**; views
report no soft-delete column.

Re-deleting a tombstone matches zero rows and surfaces as not-found. There is no
`undelete` helper; reviving deliberately means an explicit
`UPDATE … SET deleted_at = NULL`.

**The one to remember: plain `.upsert().run()` on a `@@soft_delete` model revives
a tombstone.** The pre-flight probe deliberately treats tombstones as "no row",
but the `DO UPDATE` branch then revives one and reports it as
`UpsertOutcome::Inserted`, emitting a `Created` event and an
`AuditOperation::Create` with `before = None` — and **the update-policy gate does
not run**. `.do_nothing()` does not share the defect; it returns
`CratestackError::Conflict`. This is a known, documented defect, not a
misunderstanding.

`@@retain(days: N)` parses, validates, lands on the descriptor, and **is never
acted on**. Run your own scheduled job.

## Upsert

`.upsert(CreateXInput)` is generated only for models whose `@id` is
client-supplied. A server-generated PK makes it a **compile error**.

`.on_conflict(ConflictTarget::columns(&["owner_id", "provider"]))` targets a real
`UNIQUE` constraint; the input must carry a value for every column in the tuple.
`ON CONFLICT ON CONSTRAINT <name>` is not exposed. Partial indexes work via
`ConflictTarget::predicate()`.

Both the **create and update policies must allow** — evaluated at call time,
before the branch is known. Stricter than "gate the path that runs", but
pre-flighting a read to pick a policy slot would leak row existence to denied
callers.

`upsert_update_columns` is: scalar columns, minus the PK, minus `@version`, minus
`@readonly`, minus `@server_only`, minus anything with `@default(...)`.
Auth-derived defaults are excluded specifically so upsert cannot become "take
ownership of any row I name" — which also means the framework does **not** verify
on the update branch that the existing row belongs to the caller. That is the
update policy's job.

`.do_nothing()` is server-only and returns
`UpsertOutcome::{Inserted, Existing}`. It still evaluates the update policy
against an existing row it will never mutate, so it cannot be used as an
existence oracle.

## Batches

Three unrelated things share the word.

1. **`POST /rpc/batch`** — a transport aggregator. Not atomic, each frame its own
   transaction, envelope always 200, per-frame status in the frames. It passes
   one cloned header map to every frame, so it **structurally cannot express N
   different `If-Match` values** — batching versioned updates through it would be
   silently wrong. Cap 1000 frames, checked before any frame runs, over it is a
   422.
2. **`update_many` / `delete_many`** — one filter, one shared patch. Policy
   compiles once into the `WHERE`, so **"denied by policy" and "did not match the
   filter" are indistinguishable**. Refuses optimistic locking outright.
3. **`batch_get` / `batch_create` / `batch_update` / `batch_delete` /
   `batch_upsert`** — item-addressed, per-item version, per-item policy, one
   outer transaction with a savepoint per item so a failing item rolls back only
   itself. **Not reachable from any generated route.**

The `BatchResponse { results, summary }` / `BatchItemStatus { Ok, Error }`
envelope in `cratestack-core` documents `POST /<model>/batch-*` routes that **do
not exist**.

## Transaction isolation

`@isolation("read_committed" | "repeatable_read" | "serializable")`, case- and
separator-insensitive. No `READ UNCOMMITTED`.

**It is inert.** Call it yourself:

```rust
run_in_isolated_tx_with_retries(pool, TransactionIsolation::Serializable, 3, |tx| async { … })
```

Retries on SQLSTATE `40001` (serialization failure) and `40P01` (deadlock),
**including errors raised from `commit()` itself** — SSI defers write-skew to
commit time. Composing write-builder `run_in_tx` calls inside does not get
automatic audit-sink fan-out or `@@emit` delivery; dispatch after `Ok` returns.

## Trusted proxy and client IP

```rust
router.layer(Extension(
    TrustedProxyConfig::trusting([net])
        .max_hops(2)
        .forwarded_header(ForwardedHeader::XForwardedFor)
))
```

The default, with nothing configured, is `client_ip: None`. **Headers are never
trusted by default and nothing is guessed.**

Four hazards, all found by adversarial review and all fixed — worth knowing
because they are the shape of the mistake if you reimplement any of this:

1. **Exactly one header is ever honoured**, selected by `ForwardedHeader`,
   defaulting to `X-Forwarded-For`, never falling through. The first
   implementation checked RFC 7239 `Forwarded` first — and since real proxies set
   `X-Forwarded-For` and never touch `Forwarded`, a `Forwarded` header at the
   origin is entirely attacker-authored and was never hop-counted.
2. **Hop values are validated as IPs.** Otherwise `666.666.666.666` lands in the
   audit trail verbatim.
3. **Duplicate headers are concatenated in wire order.** `HeaderMap::get` returns
   only the first, so a proxy appending its hop as a second header line was
   silently dropped in favour of whatever the attacker sent first.
4. **Hops are counted right-to-left**, from the end nearest the trusted proxy.
   The left end is exactly what an untrusted client controls.

The likely operator mistake: applying `TrustedProxyConfig` **without**
`into_make_service_with_connect_info` silently degrades `client_ip` to `None`
forever. A once-per-process warning now fires for that.

Forwarded headers are never a substitute for `ConnectInfo` in the rate limiter or
the idempotency fingerprint.

## Pagination

`@@paged` is bare, and **duplicates are rejected** (unlike `@@audit` and
`@@soft_delete`). It changes only the list route.

```json
{ "items": [], "totalCount": 0,
  "pageInfo": { "limit": 50, "offset": 0, "hasNextPage": false, "hasPreviousPage": false } }
```

`MAX_LIST_LIMIT` is **1000**, applies to every list route paged or not, on both
transports, and has **no per-model override**. Omitting `limit` defaults it to
the cap, not to unbounded.

`totalCount` costs a second `COUNT(*)` that reuses the page query's exact filters
and policy scope — a divergence there would let a caller learn the size of a
result set policy does not let them read.

**Cursor pagination is not implemented.** No cursor type, no `after`/`before`.

## Size bounds

2 MiB request body (matching axum's own implicit `Bytes` default, so the explicit
limit was provably a no-op on upgrade), 8 MiB in-process response rebuffer, 1000
batch frames.

**Change it only through the `body_limit_bytes` parameter of `router()` /
`rpc_router()`.** Re-layering `DefaultBodyLimit` on the returned router does
nothing in *either* direction — loosening is ignored and tightening is ignored —
because axum applies layers bottom-to-top and the innermost restriction wins,
which is structurally always CrateStack's own.

## Multi-tenancy

**`@@unique_per_tenant` is not a real attribute.** Its only appearance in the
entire codebase is a test asserting its absence. Use a composite unique:
`@@unique([tenantId, name])`.

`@default(auth().tenantId)` — dotted paths work — fills a column on create.
**It does not enforce tenant isolation.** Reads, updates and deletes are scoped
only by `@@allow` / `@@deny` predicates. A model with an auth-defaulted tenant
column and no tenant predicate in its read policy is fully cross-tenant readable.

Resolution order matters and is deliberate: an **anonymous** caller with a
missing auth field gets `Forbidden` (403) *before* the required-ness check runs,
so the error cannot leak which auth claim the schema expects. An authenticated
caller missing a required auth field gets a `Validation` error regardless of
whether the model field is nullable — a required auth field silently resolving to
NULL was a real policy bypass, because SQL's `NULL != X` is NULL, not true.

## Computed fields

`@computed` and `@computed(params: T?)` — see `cratestack-schema` for the
authoring rules. Runtime facts that matter here:

- **Params now have full REST/RPC parity.** REST uses
  `?computedParams=<URL-encoded JSON object>`; RPC carries
  `computedParams: Option<String>` on `RpcListInput` and on a dedicated
  `RpcGetInput`. It is a raw JSON *string*, not a nested value, for three real
  reasons — CBOR `Option::None` corruption on round trip, surviving the batch
  re-encode, and reusing the REST validator byte-for-byte.
- Malformed JSON, unknown top-level keys, keys naming a param-less field, or
  params for a field excluded by `?fields=` are all **422** on both transports.
- **Only top-level keys are validated before the database is touched.** Decoding
  a key's value happens at response-serialisation time, after rows are fetched.
- **Unknown keys *inside* a params object are silently ignored** — plain serde,
  not `deny_unknown_fields`.
- **Create/update/delete commit the write before resolvers run.** A resolver
  error always describes a failed *response* for a write that already happened.
- The Rust client's `computed_params` is **positional** — adding a model's first
  parameterised computed field changes `get(id, headers)` into
  `get(id, computed_params, headers)` and breaks call sites. Dart and TypeScript
  are additive.
- Computed fields never appear in event payloads, are a parse error on `@stream`
  items, and are a compile error under `include_embedded_schema!`.

## Composite primary keys

`@@id([a, b])` requires at least two distinct scalar fields, is mutually
exclusive with any field-level `@id`, and rejects `@readonly`, `@server_only`,
`@version` and `@computed` members.

**It then fails at macro expansion** with a clear error pointing at issue #136 —
all three entry macros reject it. What works today is the migration half: the
DDL emitters already collect every flagged column into one
`PRIMARY KEY (a, b)` clause, so you can adopt it for real composite constraints
while the rest catches up.

Do not confuse this with composite **conflict targets** for upsert, which work
today on both backends.
