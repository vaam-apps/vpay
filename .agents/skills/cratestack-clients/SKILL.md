---
name: cratestack-clients
description: Generating and consuming CrateStack clients — include_client_schema! for Rust, cratestack generate-dart (with the riverpod preset), cratestack generate-typescript (with swr/tanstack/rtk/refine layers), the @cratestack npm family, cratestack_cbor for Dart, the CBOR wire contract and reviveWireFields, WireMock stubs, and client state stores. Load when generating an SDK, wiring a React/Flutter front end to a CrateStack service, or debugging a Decimal or Bytes value that arrived as a string.
---

# Clients

> **Verified against CrateStack 0.12.0.** CrateStack is pre-1.0 and its crates version
> together, so a minor release can break any of this. Check what you are actually on —
> `cratestack --version`, and the `cratestack-*` version in `Cargo.toml` — before relying
> on a fact here. Anything that arrived in a specific release is marked *(since X.Y.Z)*;
> the full feature-to-release map is in
> [cratestack/references/version-history.md](../cratestack/references/version-history.md).

Three generated languages, one wire contract.

| Language | Produced by | Ships |
| --- | --- | --- |
| Rust | `include_client_schema!` | typed model + procedure clients over a reqwest runtime |
| Dart / Flutter | `cratestack generate-dart` | models, selection builders, API facades, `--preset riverpod` layer |
| TypeScript | `cratestack generate-typescript` | fetch client, plus `--swr` / `--tanstack` / `--rtk` / `--refine` layers |

## Rust — `include_client_schema!`

```toml
cratestack = { package = "cratestack-client", version = "0.12" }
```

Arguments: a path literal, plus an optional `decimal = RustDecimal | BigDecimal`.
That facade re-exports **only** `include_client_schema!` — not the other two
entry macros — and `cratestack-axum` (and therefore `axum`, `tower`, `hyper`,
`tower-http`) is structurally absent from its graph under default features.

Emitted into `cratestack_schema`: `models`, `types`, `inputs`
(`Create<M>Input`, `Update<M>Input`, `<M>Where`, `<M>OrderByClause`,
`<M>FindManyInput`), per-model field/selection modules, `procedures` (one
submodule per procedure with `Args` and `Output`), and `client` with `Client`,
per-model `<M>Client` and `ProceduresClient`.

Deliberately **absent** compared with the server macro: no policy constants, no
`validate()` on inputs, no `*_MODEL` descriptors. And a verb suppressed with
`@@internal(...)` emits **no client method at all** — a compile error for the SDK
consumer rather than a runtime 403.

```rust
let runtime = CratestackClient::cbor(ClientConfig { base_url: url })
    .with_request_authorizer(Arc::new(MyAuth));
let client = cratestack_schema::client::Client::new(runtime);

let posts = client.posts().list(query, headers).await?;
let one   = client.posts().get(id, headers).await?;
let out   = client.procedures().my_procedure(&args, headers).await?;
```

The model accessor is the **pluralised snake_case model name**. REST routes are
`/<plural>` and `/$procs/<ProcedureName>`.

Under `transport rpc` the outer shape is identical so call sites need not know
the transport, but: the envelopes (`RpcListInput`, `RpcPkInput`,
`RpcUpdateInput`) are built inside the methods so the surface stays `get(id)`;
list-returning procedures use `call_streaming` and return `RpcStream<Item>`;
errors are `RpcClientError`, not `ClientError`; and **per-call headers are
dropped from the RPC surface entirely** — auth flows through
`with_request_authorizer`. The RPC `Client` gains `rpc()`, `runtime()` and
`batch()`.

**Auth is an async trait** so an OAuth2 client-credentials provider can refresh
on a cache miss:

```rust
#[async_trait::async_trait]
pub trait RequestAuthorizer: Send + Sync {
    async fn authorize(&self, request: &AuthorizationRequest)
        -> Result<Vec<(String, String)>, ClientError>;
}
```

Header injection order: schema SHA, `Accept`, `Content-Type`, authorizer headers,
then **per-call extras last so they override**.

**`ensure_crypto_provider()`** installs a ring-backed rustls provider if none is
present. `CratestackClient::new` calls it for you; **`with_http_client` does
not** — call it yourself first or you get a rustls panic in your own code.

## Dart

```bash
cratestack generate-dart --schema schema.cstack --out ./client \
  --library-name my_client --preset riverpod --run-build-runner
```

`--preset riverpod` generates a provider per operation on top of the default
layout. Consumption in Flutter is one override:

```dart
ProviderScope(overrides: [
  flutterRiverpodClientAdapterProvider.overrideWithValue(CratestackDioAdapter(dio)),
])
```

Two REST adapters ship — `CratestackDioAdapter` (JSON) and
`CratestackCborDioAdapter` (CBOR) — and the consumer picks. Generated data
classes carry hand-rolled `fromWire` / `toWire`, both routing every field through
JSON-plain values (ISO-8601 for `DateTime`, `List<int>` for `Bytes`, decimal
string text), which is why bridging `cratestack_cbor`'s JSON-text boundary is
safe.

`cratestack_cbor` on pub.dev is the native codec, auto-selecting
flutter_rust_bridge natively and wasm-bindgen on web, byte-identical to the
server's `CborCodec`. `createCborCodec()` is idempotent.

**Hard constraint before `pub add`:** it pins `flutter_rust_bridge: 2.13.0`, and
in pub's grammar a bare version is an **exact** pin. If anything in your graph
wants a different flutter_rust_bridge, `pub get` fails during version solving and
you cannot add the package at all. That is upstream's requirement, not a
CrateStack choice. Vendored platforms cover Linux x86_64, Android, Windows x64,
macOS and iOS xcframeworks, and web — **Linux arm64 is the one gap**.

## TypeScript

```bash
cratestack generate-typescript --schema schema.cstack --out ./client \
  --package-name my-client --swr
```

The layer flags are additive; the default `src/` layout is always emitted. `--swr`
adds `src/swr/` reachable as `<package>/swr`, `--tanstack` adds
`src/react-query.ts`, `--rtk` adds `src/rtk-api.ts`, `--refine` adds
`src/refine.ts`.

`--rtk` *(since 0.12.0)*: its interesting part is that a **procedure's
invalidation tags are derived from the schema** — the generator walks the procedure's own args and return type
(recursing through `Page<T>` and `FindMany<T>`) and turns the models it finds
into that procedure's tag list. The two transports dispatch differently on
purpose: RPC endpoints call the resolved base query from inside `queryFn` so the
wire response still goes through revival, and REST endpoints use
`fakeBaseQuery()` and call this same package's REST client methods.

## The wire contract

**Default codecs differ by target, and this catches people:**

| Client | Default |
| --- | --- |
| Rust | `CborCodec` |
| TypeScript, `transport rpc` | native CBOR via `@cratestack/cbor` |
| **TypeScript, `transport rest`** | **JSON, hardcoded — there is no codec seam at all** |
| Dart REST | consumer picks the adapter |

**Regeneration hazard:** native CBOR became the default for generated TypeScript
RPC clients *in 0.8.14* (#746). Re-running `generate-typescript` on an RPC package
generated before that silently upgrades its wire codec from JSON to CBOR. A JSON-only server `CodecSet` then answers 406/415. Either mount
`CodecSet::new(CborCodec, JsonCodec)` or generate with `--no-native-cbor`.

### Always revive

Every generated TypeScript call site pipes through the helper unconditionally:

```ts
.then((value) => reviveWireFields(value, 'Board') as Board[])
```

The return is an **`as` cast**. So a hand-rolled call that skips revival
**compiles cleanly, and the types say `Decimal` and `Uint8Array` while the
runtime value is still a `string` and a `number[]`**. That exact regression
shipped once: a procedure declared `quote(): Decimal` decoded as an untouched
string.

The registry is keyed by **structural path, not a flat field-name set** — and
that was confirmed empirically, not theorised. With `Order.total: Decimal` plus a
related `Account.total: String` and the relation included, a flat set threw
`[DecimalError] Invalid argument` on a perfectly valid response, and silently
corrupted a numeric-looking `"00123"` into `Decimal("123")`.

Use `revivePagedWireFields` for `@@paged` responses.

### The schema-SHA header

Every generated client stamps its schema's hex SHA-256 as
`x-cratestack-schema-sha` on every request. The server **warns on mismatch and
never rejects**. An absent SHA simply omits the header. It is a drift signal, not
integrity.

## The npm family

| Package | What it is |
| --- | --- |
| `@cratestack/cli` | downloads the prebuilt CLI binary at postinstall |
| `@cratestack/ts-types` | the pinned wire/link contract every generated RPC project mirrors |
| `@cratestack/runtime-fetch` | `typeof fetch` transport adding a per-call timeout |
| `@cratestack/runtime-axios` | the same shape, backed by axios |
| `@cratestack/adapter-tanstack-query` | `rpcQueryOptions` / `rpcMutationOptions` over a generated RPC client |
| `@cratestack/adapter-rtk` | `createRpcBaseQuery`, an RTK Query `BaseQueryFn` |
| `@cratestack/refine` | refine.dev `DataProvider` (REST and RPC) — the *safe* admin-UI surface, since it goes through the generated API and therefore through policy, validation, `@version` and audit, unlike Studio's direct database access |
| `@cratestack/validator-zod` / `-yup` | an `RpcLink` validating input against per-op schemas |
| `@cratestack/link-batch` | automatic batch scheduling as an `RpcLink` |
| `@cratestack/link-logger` | reference logging link; never touches `response.body` |
| `@cratestack/cbor` | umbrella codec; conditional exports pick node or web |
| `@cratestack/cbor-node` | N-API codec wrapping the framework's own Rust CBOR crate |
| `@cratestack/cbor-web` | wasm-bindgen codec for browsers |

`@cratestack/api` is a compat umbrella re-exporting the whole split family behind
one package.

Links are **RPC-only** — the REST binding has no link chain. Streaming runs
through a separate `RpcStreamLink` chain. See `cratestack-rpc`.

## WireMock stubs

`cratestack generate-wiremock` emits one stub per procedure and five per model,
for contract tests without a live server.

**Scope limit, by design:** `transport rpc` model CRUD and every procedure have
**no per-record statefulness** — they always answer the same synthesised example
regardless of what was previously created, updated or deleted through them.
`transport rest` model CRUD stubs *are* stateful and need more than a plain
WireMock to serve.

## Client state stores

`ClientStateStore` lives in `cratestack-core` (not the HTTP client — that fixed a
layering back-edge):

```rust
pub trait ClientStateStore: Send + Sync {
    fn load(&self) -> Result<PersistedClientState, CratestackError>;
    fn save(&self, state: &PersistedClientState) -> Result<(), CratestackError>;
    fn append_request_journal(&self, entry: &RequestJournalEntry) -> Result<(), CratestackError> { … }
}
```

Only `load` and `save` are required. Adapters: `cratestack-client-store-sqlite`
and `cratestack-client-store-redis`, plus `InMemoryStateStore` and
`JsonFileStateStore` in the runtime. Wire one in with `.with_state_store(...)`.

**`cratestack-client-store-sqlite` is an offline request journal, not the
embedded ORM.** For on-device data see `cratestack-embedded`.

## Keeping committed clients honest

The framework commits two generated clients and regenerates them with
`just regen-examples`, which CI runs as `just regen-examples --check`. The recipe
*is* the check, so the local command and the CI gate cannot copy-paste diverge.
Adopt the same shape: one command, `--check` in CI, review `git diff` locally
after touching a template.

Remember `--check` honours `.gitignore` for its "unexpected file" arm, so run it
inside a real checkout.
