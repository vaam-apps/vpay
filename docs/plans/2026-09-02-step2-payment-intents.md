<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 2 — payment intents without a rail: implementation-ready design

Decisions taken by the orchestrator (do not reopen):
- D1 keep `one_charge_per_intent` unique and unscoped; add NON-unique partial `charges_live_idx`.
- D2 charge carries the intent's currency verbatim; no conversion, no per-rail currency check this step.
- D3 **Keep** the `/v1` auth layer (authenticated by construction) but make it validate ONCE: replace
  `from_extractor_with_state::<AuthenticatedMerchant,_>` with a `from_fn_with_state` middleware that runs the
  validator once and inserts `ResourceClaims` (plus the resolved `merchant_id`) into request extensions; the
  `AuthenticatedMerchant` extractor used by handlers reads extensions and FAILS CLOSED with `ApiError::Internal`
  if absent (never silently None). Add a router test that walks every registered `/v1` path and asserts 401
  without a token.
- D4 Postgres enums are `String` in vpay-db; vpay-core parses.
- D5 wire DTOs live in `vpay-api/src/model.rs`.
- D6 duplicate the four-line `adapters()` into `vpay-worker-bin/src/main.rs`.
- D7 `Idempotency-Key` REQUIRED on every POST under `/v1` (400 `invalid_request_error`, `param: "idempotency_key"`); update `examples/merchant-curl`.
- D8 `ending_before`: `ORDER BY seq ASC WHERE seq > cursor`, reverse in Rust; `data` always newest-first.
- `merchant_id` is a new required field on `MerchantClient` (YAML), mapped from the token's `client_id`.
- Stripe-bracket form decoding hand-rolled; accept `k[0]=v` and `k[]=v`; `+` is a literal.
- `currencies`/`providers` seeded by boot step 4 (one transaction) in BOTH binaries; unknown provider code in YAML = exit 78.
- `GET /v1/balance`, `/v1/events`: not routed (honest 404).
- `confirm` built through persistence + write-first ordering; stops at the adapter's `NotImplemented` → 501.
- `ApiError` gains `NotFound { resource, id }`, `Conflict { message }`, `Forbidden`.

## 0. Facts about the tree (post Step 1)
1. `vpay-api` links no adapter crate; `adapters()`/`adapter_registry()` live in `backends/apps/vpay-server/src/lib.rs:11,19`,
   consumed only by a log line in `main.rs`. `RouterDeps` (`vpay-api/src/lib.rs:~149`) has `pool`, `merchant_op`, `merchant_validator`.
2. `RouterDeps` holds no `Config`; `livemode` (`config.deployment.livemode`) is unreachable from handlers today.
3. `ProviderAdapter::submit` is synchronous (`vpay-provider/src/lib.rs`). Do not `.await` it.
4. `from_extractor_with_state` discards the extracted value (axum 0.8) — see D3.

## 1. Migrations (0014–0018) — implementer A
**0014_payment-intent-api-fields.sql** (hard cutover on 0003):
```sql
ALTER TABLE payment_intents
  ADD COLUMN seq BIGINT GENERATED ALWAYS AS IDENTITY,
  ADD COLUMN metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN description TEXT,
  ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  ADD COLUMN last_payment_error_code failure_code,
  ADD COLUMN last_payment_error_message TEXT,
  DROP COLUMN last_payment_error;
ALTER TABLE payment_intents
  ADD CONSTRAINT lpe_paired CHECK ((last_payment_error_code IS NULL) = (last_payment_error_message IS NULL)),
  ADD CONSTRAINT lpe_message_length CHECK (last_payment_error_message IS NULL OR char_length(last_payment_error_message) <= 512),
  ADD CONSTRAINT description_length CHECK (description IS NULL OR char_length(description) <= 1000),
  ADD CONSTRAINT metadata_is_object CHECK (jsonb_typeof(metadata) = 'object'),
  ADD CONSTRAINT pmt_is_array CHECK (jsonb_typeof(payment_method_types) = 'array');
CREATE UNIQUE INDEX payment_intents_seq_key ON payment_intents (seq);
CREATE INDEX payment_intents_merchant_seq_idx ON payment_intents (merchant_id, seq DESC);
ALTER TABLE charges ADD COLUMN updated_at TIMESTAMPTZ NOT NULL DEFAULT now();
CREATE INDEX charges_live_idx ON charges (state) WHERE state IN ('submitting','submitted','pending','unresolved');
```
(check the exact `charge_state` enum labels in 0004 and use them.)
**0015_create-idempotency-keys.sql**: PK `(merchant_id, idempotency_key)`; `request_method TEXT`, `request_path TEXT`,
`request_hash BYTEA CHECK (octet_length(request_hash)=32)`, `state TEXT CHECK (state IN ('in_flight','complete'))`,
`response_status SMALLINT`, `response_body JSONB`, `created_at`, `completed_at`, `expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '24 hours'`;
`CHECK (state <> 'complete' OR (response_status IS NOT NULL AND response_body IS NOT NULL AND completed_at IS NOT NULL))`; index on `expires_at`.
**0016_create-provider-requests.sql**: `id BIGINT IDENTITY PK`, `charge_id TEXT REFERENCES charges(id)`, `provider_code TEXT REFERENCES providers(code)`,
`operation TEXT CHECK (operation IN ('submit','query_status','refund'))`, `provider_reference_id UUID NOT NULL`, `attempt INT NOT NULL DEFAULT 1`,
`status_code INT`, `error_kind TEXT`, `sent_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `responded_at TIMESTAMPTZ`,
`CHECK ((status_code IS NULL) = (responded_at IS NULL))`, index `(charge_id, sent_at)`. NOT unique on charge_id (one row per attempt).
**0017_create-refunds.sql** (schema only): `refund_status` enum ('pending','succeeded','failed','canceled'); `refunds(id TEXT PK, payment_intent_id → payment_intents,
charge_id → charges NULL, amount BIGINT CHECK > 0, currency_code → currencies, status refund_status, reason TEXT, failure_code failure_code, failure_raw TEXT,
metadata JSONB NOT NULL DEFAULT '{}', provider_reference_id UUID, created_at, updated_at)`.
**0018_create-events.sql** (schema only): `events(id TEXT PK, seq BIGINT IDENTITY, merchant_id, livemode, type TEXT, object_id TEXT, data JSONB,
fanout_state TEXT DEFAULT 'pending' CHECK IN ('pending','done'), created_at)`; `type` constrained to the seven types in docs/flows/webhooks.md;
index `(seq) WHERE fanout_state='pending'`; unique on `seq`; index `(merchant_id, seq DESC)`.

## 2. vpay-db — implementer A
`DbError` gains `UniqueViolation { constraint, source }` (Category::Conflict, code `resource_conflict`) and
`ForeignKeyViolation { constraint, source }` (Category::InvalidRequest, code `invalid_reference`); helper `classify_write(sqlx::Error) -> DbError`
(23505 → Unique, 23503 → FK, else Query). Every exhaustive `match` in `impl Classify for DbError` gets both arms.

Row structs (FromRow; `time::OffsetDateTime`; `serde_json::Value`): `PaymentIntentRow { id, seq: i64, merchant_id, livemode: bool, amount: i64,
amount_received, amount_refunded, amount_refund_pending, currency_code, status: String, last_payment_error_code: Option<String>,
last_payment_error_message: Option<String>, payment_method_types: Value, metadata: Value, description: Option<String>, created_at, updated_at }`,
`NewPaymentIntent` (same minus seq/updated_at/amount_*), `ChargeRow { id, payment_intent_id, provider_code, provider_reference_id: Uuid,
provider_ref_extra: Option<Value>, redirect_url: Option<String>, state: String, amount, currency_code, payer_ref: Option<String>, ... }`,
`IdempotencyRecord { request_hash: Vec<u8>, state: String, response_status: Option<i16>, response_body: Option<Value> }`,
`enum IdempotencyClaim { Fresh, Replay(IdempotencyRecord), InFlight, Mismatch }`.

Signatures C codes against, verbatim:
```rust
// payment_intents.rs
pub async fn insert(pool: &PgPool, new: &NewPaymentIntent) -> Result<PaymentIntentRow, DbError>;
pub async fn get_for_merchant(pool: &PgPool, merchant_id: &str, id: &str) -> Result<Option<PaymentIntentRow>, DbError>;
pub struct ListPage { pub limit: i64, pub starting_after: Option<String>, pub ending_before: Option<String> }
pub async fn list_page(pool: &PgPool, merchant_id: &str, page: &ListPage) -> Result<(Vec<PaymentIntentRow>, bool), DbError>; // limit+1 internally
pub async fn transition(pool: &PgPool, merchant_id: &str, id: &str, expected: &str, new: &str) -> Result<Option<PaymentIntentRow>, DbError>;
pub async fn cancel(pool: &PgPool, merchant_id: &str, id: &str) -> Result<Option<PaymentIntentRow>, DbError>; // transition(.., "requires_payment_method", "canceled")
// charges.rs
pub async fn insert_for_intent(tx: &mut PgConnection, new: &NewCharge) -> Result<ChargeRow, DbError>; // 23505 on one_charge_per_intent → UniqueViolation
pub async fn get_for_intent(pool: &PgPool, payment_intent_id: &str) -> Result<Option<ChargeRow>, DbError>;
// idempotency.rs
pub async fn claim(pool: &PgPool, merchant_id: &str, key: &str, method: &str, path: &str, request_hash: &[u8; 32]) -> Result<IdempotencyClaim, DbError>;
pub async fn store(pool: &PgPool, merchant_id: &str, key: &str, status: u16, body: &Value) -> Result<(), DbError>;
pub async fn sweep_expired(pool: &PgPool) -> Result<u64, DbError>;
// provider_requests.rs
pub async fn insert_pending(pool: &PgPool, charge_id: &str, provider_code: &str, operation: &str, reference: Uuid, attempt: i32) -> Result<i64, DbError>;
pub async fn record_response(pool: &PgPool, id: i64, status_code: Option<i32>, error_kind: Option<&str>) -> Result<(), DbError>;
// config_reconcile.rs — boot step 4, one transaction, plain seed structs, NO adapter dependency
pub struct ProviderSeed { pub code: String, pub display_name: String, pub flow: String, pub supports_refunds: bool, pub supports_partial_refunds: bool, pub delivers_callbacks: bool, pub requires_ip_allowlist: bool, pub enabled: bool }
pub struct CurrencySeed { pub code: String, pub exponent: i32 }
pub async fn reconcile(pool: &PgPool, currencies: &[CurrencySeed], providers: &[ProviderSeed]) -> Result<(), DbError>;
```
`claim`: `INSERT … ON CONFLICT (merchant_id, idempotency_key) DO NOTHING RETURNING …`; zero rows → SELECT existing and compare hash in constant time.
`reconcile`: BEGIN; upsert currencies (check exponent against existing rows); upsert providers ON CONFLICT (code) DO UPDATE; `UPDATE providers SET enabled=false WHERE code <> ALL($codes)`; COMMIT.
Docker tests in `vpay-db/tests/repositories.rs`: second charge → `UniqueViolation{constraint:"one_charge_per_intent"}` not Query; unknown currency → FK; stale `transition` → Ok(None) untouched;
two concurrent `claim`s → exactly one Fresh; list cursor round trip over 25 rows; `reconcile` idempotent and flips enabled=false for a dropped code.

## 3. vpay-core — implementer B
`state.rs`: `pub enum Transition { Create, Confirm(ProviderFlow), Cancel }`, `pub const fn next_status(from: IntentStatus, t: Transition) -> Option<IntentStatus>`;
Confirm(Push): RequiresPaymentMethod → Processing; Confirm(Redirect): RequiresPaymentMethod → RequiresAction (route through `ProviderFlow::status_after_confirm`);
Cancel: RequiresPaymentMethod → Canceled; everything else None. Initial `ChargeState` is `Submitting`.
`ids.rs`: `pi_`/`ch_`/`re_`/`evt_` + 24 lowercase Crockford base32 chars from a UUIDv4's 128 bits (charset `[a-z0-9]`, percent-encodes to itself; fits the 1..64 CHECK).
`uuid` moves from vpay-api dev-deps to deps as needed.

## 4. vpay-api — B (form, idempotency extractor, model, ApiError variants) and C (handlers, wiring)
Wire object (`vpay-api/src/model.rs`), exactly `sdks/rust/tests/support/mod.rs:134-149`; EMIT EVERY KEY incl. explicit nulls:
`{ "id":"pi_…","object":"payment_intent","amount":5000,"currency":"xaf","status":"requires_payment_method","payment_method_types":["mtn_momo"],
"next_action":null,"last_payment_error":null,"metadata":{},"description":null,"created":1753401600,"livemode":false }`;
`next_action` = `{"type":"redirect_to_url","redirect_to_url":{"url":…,"return_url":…}}`; list envelope `{"object":"list","data":[…],"has_more":bool,"url":"/v1/payment_intents"}`.
`currency` lowercase at the boundary; `created` = unix seconds; `livemode` from config.

**form.rs (B)**: `VpayForm<T>` `FromRequest<S>` with `Rejection = ApiError`; split `&` then `=`; percent-decode per segment THEN split brackets
(the SDK escapes `[` inside a key segment as `%5B` — `metadata[a%5Bb]=v` yields key `a[b`); numeric or empty segment → array (`k[0]=v` and `k[]=v`); else object;
`serde_json::from_value::<T>`; failures → `ApiError::InvalidParam{param}` (offending top-level key or "body"); `+` literal; body limit 64 KiB via
`tower_http::limit::RequestBodyLimitLayer` on the `/v1` nest; same tree for GET query strings. Unit tests against the exact byte strings in
`sdks/rust/src/form.rs`'s `node_parity` module.
**idempotency.rs (B)**: `IdempotencyKey` extractor — required on every POST under `/v1`; absent/empty → `ApiError::InvalidParam{param:"idempotency_key", message:"An Idempotency-Key header is required on every POST to /v1."}`; max 255 bytes; request hash = SHA-256 over method + path + raw body.
**error.rs (B)**: `NotFound { resource: &'static str, id: String }` → Category::NotFound code `resource_missing`; `Conflict { message: String }` → Category::Conflict code `invalid_state`, public message = message; `Forbidden` → Category::Forbidden. All five Classify matches get arms. No new pub `*Error` types.

**Handlers (C)** `vpay-api/src/v1/payment_intents.rs`, mounted in the `v1` nest:
POST /v1/payment_intents (amount, currency, payment_method_types[], metadata[…], description): claim → insert(status requires_payment_method) → store → 200.
GET /v1/payment_intents/{id}: get_for_merchant → 200 | 404. POST …/{id}/confirm (payment_method_data[type], payment_method_data[mtn_momo][msisdn], return_url).
POST …/{id}/cancel: cancel → 200; Ok(None)+row exists → 409. GET /v1/payment_intents?limit&starting_after&ending_before → list envelope.
Validation: amount > 0 and ≤ 2^53-1; currency uppercased then `Currency::from_code` AND present in `currencies`; every payment_method_types ∈ enabled providers.code
(else param payment_method_types); metadata ≤ 50 keys, key ≤ 40, value ≤ 500; description ≤ 1000; limit default 10 ceiling 100.
Tenant scoping: merchant_id from `MerchantClient.merchant_id` looked up by `ResourceClaims.client_id`; every query filtered; foreign id → NotFound (never Forbidden).
**confirm order**: 1 load for merchant (404) ; status ≠ requires_payment_method → 409 Conflict; existing charge → 409 without insert. 2 resolve adapter by
payment_method_data[type] from `RouterDeps.adapters`; branch on `capabilities().flow` ONLY; push needs msisdn, redirect needs return_url. 3 `reference = Uuid::new_v4()`;
TX1 insert charge state submitting with provider_reference_id; COMMIT. 4 `provider_requests::insert_pending(.., status_code NULL)`. 5 `adapter.submit(..)` →
`Err(ProviderError::NotImplemented("mtn_momo::submit"|"orange_money::submit"))`. 6 `record_response(id, None, Some("not_implemented"))`, return the error →
501 `{"error":{"type":"api_error","code":"not_implemented","message":"This operation is not implemented yet."}}`; the intent stays requires_payment_method;
the submitting charge row + NULL-status provider_request row are left on purpose (Phase 4's recovery state).
`RouterDeps` gains `adapters: Arc<BTreeMap<String, Box<dyn ProviderAdapter>>>` and `resource_config: Arc<ResourceConfig>` (livemode, merchant_id_by_client_id,
enabled currency codes, enabled provider codes + ProviderConfig per rail), with `FromRef<AppState>` impls.

## 5. vpay-config — C
`MerchantClient.merchant_id: String` (garde min 1); uniqueness across clients → new `ConfigError::DuplicateMerchantId` in `Config::validate_all`;
add `merchant_id:` to `config/application.yml` and every fixture. `ProviderHost.enabled: bool` (`#[serde(default = "default_true")]`); no capability fields.
Boot step 4 call site in BOTH binaries between `run_migrations` and `ensure_active_signing_key`: build `Vec<ProviderSeed>` by joining `config.providers`
against the binary's own `adapters()`; a YAML provider code with no linked adapter is exit 78 (ConfigError).

## 6. Tests — C (`backends/tests/integration/tests/payment_intents.rs`, harness cloned from merchant_token_flow.rs)
1 create_then_retrieve_round_trips_through_the_sdk; 2 a_replayed_idempotency_key_returns_the_same_object_and_no_second_row (count(*)=1);
3 a_reused_key_with_a_different_body_is_the_400_envelope (type idempotency_error, code idempotency_key_in_use — check Category::Idempotency's actual code);
4 a_second_confirm_cannot_produce_a_second_charge (count=1, second is 409); 5 confirm_reaches_the_adapter_and_renders_the_documented_501 (+ submitting charge row + NULL provider_request row);
6 cancel_is_legal_only_from_requires_payment_method; 7 list_pages_forward_and_backward_with_cursors (25 rows, limit 10); 8 merchant_b_cannot_read_merchant_as_intent (404 byte-identical to missing);
9 (D3) every registered /v1 path answers 401 without a token; 10 missing Idempotency-Key on POST → 400 param idempotency_key.
`just sdk-conformance-node` cannot target the server (it verifies an assertion only) — say so, do not claim Node parity.
`just verify-ignored`: bump `expected_suites` by exactly the number of new test binaries.

## 7. Work split
A: migrations 0014–0018, `vpay-db/src/{error,payment_intents,charges,idempotency,provider_requests,config_reconcile}.rs`, `vpay-db/src/lib.rs`, `vpay-db/tests/repositories.rs`.
B: `vpay-core/src/{state,ids}.rs`, `vpay-api/src/{form,idempotency,model,error}.rs`, `vpay-api/Cargo.toml`.
C: `vpay-config/src/{oauth,config,lib,error}.rs`, `config/application.yml`, fixtures, `vpay-api/src/v1/**`, `vpay-api/src/lib.rs` (RouterDeps, FromRef, routes, D3 middleware),
`vpay-api/src/resource_auth.rs` (extractor reads extensions), both `main.rs` (adapters(), boot step 4, RouterDeps fields), `examples/merchant-curl`, `backends/tests/integration/tests/payment_intents.rs`, `justfile` expected_suites.
Shared files: none between A/B; C owns lib.rs/main.rs; docs/status.md is updated by the orchestrator after all three.

## 8. Docs (orchestrator, after)
status.md: HTTP surface, Idempotency ⛔→🟡, Database schema (5→10 migrations), Config guard rails / YAML config loading (boot step 4 exists), ApiError row; no new NotImplemented token;
`mtn_momo::submit`/`orange_money::submit` are reached from a shipping request path for the first time (501 is a real answer now). docs/api/README.md, flows/payment-lifecycle.md,
crash-safety.md, merchant-auth.md, configuration.md Status; roadmap Phase 3; examples/merchant-demo extended to create + retrieve.
