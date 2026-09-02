# Roadmap

**Snapshot date: 2026-08-11.** A point-in-time read of the repository, for
someone opening it cold or coming back after a break, organized as the
sequence of phases from scaffold to a deployable gateway. It goes stale the
moment code moves and nobody is obliged to update it on the same schedule as
a commit. **On any disagreement, [`docs/status.md`](status.md) and the ADRs
in [`docs/adr/`](adr/) win, always** — they are the machine-checked and
immutable sources of truth respectively. This page does not restate
`docs/status.md`'s per-feature rows; if you're here to check whether some
specific thing works, go there instead.

**vpay still cannot take a payment.** No HTTP call to any rail has ever been
made by this code. Everything below explains what stands between here and
that no longer being true.

| # | Phase | Status |
|---|---|---|
| 1 | Foundations | ✅ Complete |
| 2 | Authentication — merchant (`/v1`) | 🟡 In progress — **the repo is here now**; the merchant OP is built, the evidence is one manual run (see the 2026-09-02 addendum) |
| 2b | Authentication — dashboard login (`/dash/v1`) | ⛔ Not started — split out of Phase 2 on 2026-09-02 |
| 3 | Payment API (`/v1`) | ⛔ Not started |
| 4 | The rails | ⛔ Not started |
| 5 | The worker | ⛔ Not started |
| 6 | Webhooks | ⛔ Not started |
| 7 | Operability | ⛔ Blocked by environment, not by unwritten code |

Phases 3 and 4 are listed in build order but genuinely interleave — see
Phase 3's note. `docs/status.md`'s own "What would have to be true to call
this an MVP" list is a numbered checklist, not a stated build sequence; read
that way it doesn't conflict with the order below. Read as a sequence it
would, on two points — flagged inline at Phases 3 and 2.

---

## Phase 1 — Foundations

**Goal.** The binaries boot, self-check, connect to a real database, and
load reviewed configuration before serving anything.

**Status.** Complete. Seven commits on `master`: `61e404b` (scaffold) →
`237c716` (#1, CLI/env config) → `9e92d02` (#2, kebab-case rename) →
`879e2cb` (#3, schema/migrations/auth storage) → `286d75a` (#4,
signal-handler race fix) → `3d7635a` (#5, CrateStack re-verify) →
`932d8a4` (#6, YAML config + Postgres connectivity).

**Scope.**
- Workspace on edition 2024/resolver 3, lint policy enforced (`unwrap`/
  `expect`/`panic`/`todo`/float arithmetic denied, `unsafe` forbidden —
  [ADR-0007](adr/0007-lint-policy.md)).
- `clap` CLI on both binaries, every option resolving from an env var with
  an explicit flag winning (`vpay-config::cli`).
- Graceful shutdown on SIGINT/SIGTERM, including a fix for a real startup
  race where a SIGTERM delivered before the first signal-future poll bypassed
  shutdown entirely (`vpay_config::signal::ShutdownSignals`).
- All 12 Postgres migrations (`backends/migrations/0001`–`0012`), applied
  and constraint-tested against a real database.
- YAML configuration loading (`vpay_config::Config::load`: Figment layers +
  hand-rolled `${ENV}` resolution + `garde` validation), wired into both
  binaries as a hard startup requirement — [ADR-0003](adr/0003-yaml-configuration.md).
- `vpay-db`'s connect/migrate/healthcheck, wired into both binaries.
- `schemas/vpay.cstack` re-verified against real CrateStack 0.7.10 grammar —
  a design sketch, excluded from the build graph, not part of this phase's
  runtime scope.

**Definition of done (met).** `cargo xtask verify-status` and
`cargo xtask verify-no-mocks` pass; both binaries exit non-zero, naming the
problem, on a missing or invalid `--config`/`--database-url`
(`a_missing_config_is_a_non_zero_exit_naming_the_problem` and siblings in
each binary's `tests/cli.rs`); `/healthz` returns 200 or 503 based on a real
`SELECT 1`, not a static string; all 12 migrations apply cleanly and
idempotently with their constraints proven to fire in
`backends/tests/integration/tests/postgres_smoke.rs`.

**Unblocks.** Everything below — no later phase can start without a durable
process and a database.

**Risks and open questions carried by this phase.**
- MSRV `1.88` (`Cargo.toml`) is derived only from dependency metadata
  (`cargo metadata`'s max declared `rust_version`), never actually compiled
  against — only stable 1.95.0 was available when set. 63 of 317 graph
  packages declare no `rust_version` at all, so the true floor could be
  higher (`rust-toolchain.toml`'s own header comment).
- `.config/nextest.toml` sets `max-threads = 1` for every Postgres-backed
  integration test, tuned against a 4-vCPU Docker Desktop VM allocation on
  the authoring machine, not the host's own CPU count. Whether this should
  relax on a real CI runner with a larger Docker allocation is open — the
  file's own comment already flags the wall-clock trade-off.
- `ProviderHost::settings` and `::credentials` are both plain
  `BTreeMap<String, String>`; only `credentials` is redacted in `Debug`
  output. A value placed in the wrong map leaks in plaintext, and nothing —
  no test, no type — catches that misclassification (`docs/status.md`,
  "Secret redaction" row).

---

## Phase 2 — Authentication (OP assembly and mounting)

**Goal.** A staff member can complete an authorization-code + PKCE login
against `/dash/v1`; a merchant can exchange a `private_key_jwt` assertion
for an access token against `/v1`; both surfaces reject the other's
audience; a disabled client is refused.

**Status.** In progress — **this is where the repo is now.** Commit `#7`
(`adbcb89`/`33f2913`, "model OAuth clients, add OP persistence and JWT
validation") landed every prerequisite below in isolation, each with its own
tests, but wired nothing together. The router still serves only `/healthz`
plus the Stripe-shaped 404.

> **Addendum, 2026-09-02 (Step 1) — the merchant half is built; the
> dashboard half is split out.** The paragraph above and the scope list
> below describe this phase as it was *planned*, and are left standing
> rather than rewritten. What actually happened is that this phase turned
> out to contain two deliverables with almost no shared remaining work
> beyond the signing key, and only one of them was built.
>
> **Built (merchant, `/v1`):** steps 1, 2, 4, 5 and 6 below are done, and
> step 3 is done except for runtime rotation. `vpay_api::op` now serves
> `POST /v1/oauth/token`, discovery and `/v1/oauth/jwks.json`; every other
> `/v1` path sits behind `AuthenticatedMerchant`; a merchant exchanges a
> `private_key_jwt` assertion for a `vpay:v1`-audienced access token and
> reaches the honest 404 behind the boundary. Signing keys are generated
> (`cargo xtask gen-signing-key`), loaded from a file at boot, announced in
> `oauth_signing_keys` under an advisory lock so replicas rotate once
> between them, and published across a 24 h overlap window.
>
> **Not built (dashboard, `/dash/v1`):** no `/login`, no `/authorize`, no
> session store — `authkestra-engine` is pinned without its `sql-postgres`
> feature — and no dashboard route of any kind. This is now its own later
> phase; see "Phase 2b" immediately after this one. It was always a
> parallel deliverable rather than a prerequisite for Phases 3–6 (see
> **Unblocks** below), so splitting it changes sequencing on paper, not in
> fact.
>
> **Step 5's scope shrank on purpose.** The plan said "mounting discovery,
> `/jwks.json`, `/authorize`, `/token`, `/userinfo`". What is mounted is
> discovery, `/jwks.json` and `/token`. `/authorize` and `/userinfo` belong
> to the authorization-code grant, which no merchant client can use;
> `authkestra-axum` is deliberately **not** a dependency for exactly that
> reason — its bundled router would mount them, and its JWKS handler
> publishes one key rather than a rotation window.
>
> **Of this phase's Definition of done, two of the four bullets are met and
> two are not.** Met: the `private_key_jwt` → `client_credentials` →
> authenticated `/v1` call round trip, and the disabled-client refusal
> (`an_sdk_client_authenticates_and_reaches_the_honest_404`,
> `a_disabled_client_is_refused_with_invalid_client_and_401`). Half-met: a
> dashboard-audience token is rejected on `/v1` over a real router
> (`a_dashboard_audience_token_is_refused_on_v1`), but the other direction
> — a merchant token rejected on `/dash/v1` — cannot be tested, because
> `/dash/v1` does not exist. Unmet: the authorization-code + PKCE round
> trip. **And the evidence for the met ones is thinner than "done" usually
> implies:** those integration tests have run once, manually, against a
> scratch database, and never under Docker or in CI. `docs/status.md`'s
> header paragraph states it exactly; read that before treating this
> addendum as a completion notice.
>
> Two of this phase's open questions below were given **defaults, not
> answers**, by the code: the access-token TTL is 900 s and the
> rotation-overlap window is 24 h. Both are constants a maintainer should
> still decide; neither is recorded in an ADR or configurable.

**Scope, in dependency order.**
1. ~~`YamlClientStore`: convert configured `vpay_config::oauth::MerchantClient`
   / `DashboardClient` into `authkestra_op::client::ClientRegistration` so
   the OP can look a configured client up at all.~~ **Done 2026-09-02 for
   `MerchantClient`** (`vpay_api::op::clients`); `DashboardClient` has no
   conversion, and will not need one until Phase 2b.
2. ~~`CompositeOpStore<C, A, R, D, J, P>` filling all six type slots.~~
   **Done 2026-09-02** — `MerchantOp::new` builds exactly this, with the
   three `SqlxOpStore` slots serving no `/v1` grant (they exist because
   `OpStore` is a supertrait) and `SqlClientAssertionStore` wired in for
   replay. Original text, for the record: `CompositeOpStore<C, A, R, D, J, P>` filling all **six** type slots
   (*corrected 2026-09-02 against the pinned `=0.7.1` `store.rs`; the
   original text said five, from `0.3.4`* — `P` is the DPoP replay store,
   `NoDpopReplayStore` for vpay, which then fails closed): the new
   `YamlClientStore` for `C`, `SqlxOpStore` for the code/refresh/device
   slots, `vpay_db::SqlClientAssertionStore` wired in via
   `with_client_assertion_store` for `J`.
3. Signing-key loading from a Kubernetes Secret / env at boot, with
   `/jwks.json` reading `oauth_signing_keys` so every replica publishes an
   identical set. **Partially done 2026-09-02.** Generation exists
   (`cargo xtask gen-signing-key --out <dir>`, 3072-bit PKCS#8, mode 0600),
   loading exists (`vpay_api::op::keys::LoadedSigningKey::from_file`, RFC
   7638 thumbprint `kid`, public JWK cross-checked against
   `TokenManager::public_jwk`), boot-time activation exists
   (`vpay_db::ensure_active_signing_key`, one advisory-locked transaction so
   N replicas rotate once between them), and `/v1/oauth/jwks.json` publishes
   `publishable_signing_keys` across the overlap window. **Runtime rotation
   does not exist:** `TokenManager` holds one key for the life of the
   process, so rotating means restarting with a new Secret, nothing re-reads
   the file, a rollback to a retired `kid` is refused, and no runbook
   describes the sequence.
4. ~~`rustls::crypto::CryptoProvider::install_default()` in both binaries'
   `main()`, before the first JWKS fetch~~ — **done 2026-09-02**, see
   `docs/status.md`'s "rustls `CryptoProvider` process default" row.
5. Mounting discovery, `/jwks.json`, `/authorize`, `/token`, `/userinfo` —
   explicitly **not** device or refresh handlers (see Decisions). **Done
   2026-09-02 for the merchant subset only**: discovery, `/jwks.json` and
   `/token` are mounted under `/v1/oauth`. `/authorize` and `/userinfo`
   serve the authorization-code grant, which no merchant client can use;
   they move to Phase 2b.
6. ~~The `disabled_clients` kill-switch check on the token-issuance path.~~
   **Done 2026-09-02**, inside `YamlClientStore::find_client` — the single
   point every token request passes through for every grant, and therefore
   the only place a kill switch on `client_credentials` can be enforced at
   all. A failed lookup fails closed (`OpError::Storage` → `server_error`),
   never open.

**Definition of done.**
- An integration test drives a real authorization-code + PKCE round trip
  against `/dash/v1` and receives a token that a subsequent authenticated
  call accepts.
- An integration test signs a `private_key_jwt` assertion, exchanges it at
  `/v1`'s token endpoint for an access token via `client_credentials`, and a
  subsequent authenticated `/v1` call accepts it.
- A dashboard-audience token is rejected on `/v1` and a merchant-audience
  token is rejected on `/dash/v1` — both directions, over a real mounted
  router (the unit-level version of this already exists in
  `resource_auth.rs`'s tests; this phase needs the router-level version).
- A client present in `disabled_clients` is refused token issuance even
  though it is otherwise valid in YAML.

**Unblocks.** `/v1` request authentication (Phase 3). `/dash/v1` login
itself is not on the critical path to "does this take payments" — it shares
this phase's OP-assembly work but is a parallel deliverable, not a
prerequisite for Phases 3–6. This is one place this roadmap's ordering
differs from a strictly sequential reading of `docs/status.md`'s MVP list,
which places `/dash/v1` login last (its item 7): that ordering is fine as an
unordered checklist, but if read as a build sequence it would incorrectly
imply dashboard login blocks the payment path, which it does not.

**Decisions this phase rests on.**
- **Dashboard is read-only, one scope, until a mutating use case lands.**
  [ADR-0008](adr/0008-dashboard-scope.md) (accepted) still describes
  per-record write actions (re-poll/replay/refund/annotate) as the
  architecture. [`docs/flows/dashboard-auth.md`](flows/dashboard-auth.md)'s
  "Scope" section is the reconciliation: no mutating use case is being built
  now, so the client registration requests exactly one read-only scope and
  no `audit_log`-writing code exists — a sequencing decision, not an
  architectural reversal.
- **No refresh tokens on either surface.** `/v1`
  ([ADR-0010](adr/0010-merchant-auth-private-key-jwt.md)) matches RFC 6749
  §4.4.3 and `authkestra-op`'s own hardcoded `refresh_token: None` on the
  `client_credentials` handler. `/dash/v1`
  ([`docs/flows/dashboard-auth.md`](flows/dashboard-auth.md), Token
  lifetimes) is a deliberate exposure narrowing: a short-TTL access token
  with no long-lived refresh token for `authkestra-op`'s revocation-less OP
  to also have to protect.
- **Device flow dropped.** ADR-0010 states no `/v1` client is offered it;
  no `/dash/v1` client this deployment registers uses it either.
  `oauth_device_codes`/`oauth_refresh_tokens` (migration `0006`) still exist
  because `authkestra_op::store::OpStore` is a supertrait over
  `ClientStore + AuthorizationCodeStore + RefreshTokenStore +
  DeviceCodeStore` — a `SqlxOpStore` must satisfy all four concrete stores
  to exist at all. The enforcement point is the router (step 5 above), not
  an absent table.
- **Postgres over Redis, deliberately.** JWT validation is local against a
  cached JWKS (`authkestra_resource::jwt::JwksCache`), so a shared cache is
  not on that hot path. The one place `authkestra-op` itself would benefit
  from something Redis-shaped — a TTL'd single-use `jti` guard — is exactly
  where vpay chose Postgres durability instead
  (`vpay_db::SqlClientAssertionStore`, `INSERT … ON CONFLICT DO NOTHING`,
  proven race-safe by a 10-way concurrent test). There was never a drop-in
  Redis option to choose against *at the time*: `authkestra-op` shipped no
  Redis-backed store at `0.3.4`, confirmed against an open upstream issue,
  [marcjazz/authkestra#185](https://github.com/marcjazz/authkestra/issues/185)
  ("no Redis-backed OpStore or ClientAssertionStore, though the docs point
  integrators at Redis"). *Stale as of `=0.7.1`* (noted 2026-09-02):
  `authkestra-op-0.7.1/src/redis_store.rs` ships a
  `RedisClientAssertionStore`. The Postgres decision stands on its own
  reasoning — durability, one fewer moving part — but no longer on "there
  was nothing to choose against." Revisit on a measured trigger, not on
  principle.
- **`disabled_clients` supplements YAML identity as a kill switch.**
  ADR-0010: YAML stays authoritative for identity; the table only ever
  *subtracts* access, so revocation is an `INSERT`, not a deploy. Cost: a
  correct "is this client allowed" answer needs checking both — no runbook
  documents that yet (Open questions, below).
- **No secret material in the database at all.** Migration
  `0010_reshape-oauth-signing-keys.sql` replaced `private_key_pem` with
  `public_jwk JSONB`; the private PEM is meant to come from a Kubernetes
  Secret at boot and never be persisted.

**Risks and open questions carried by this phase.**
- ~~**`CryptoProvider::install_default()` missing is a live landmine once
  this phase mounts anything.**~~ **Closed** — both binaries install it at
  the top of `run()` (step 4). And it stopped being hypothetical on
  2026-09-02: `vpay-server` now builds a `JwtValidator` at boot and the
  first authenticated `/v1` request makes a real `Jwks::fetch`, over
  loopback to this same process's own `/v1/oauth/jwks.json`.
- **RUSTSEC-2023-0071** (Marvin Attack timing side-channel in `rsa`) is
  accepted deliberately in `deny.toml`, and became a **non-dev** dependency
  of every shipping binary the moment commit `#7` made `authkestra-op` a
  production dependency of `vpay-db`: `cargo tree -i rsa -e normal` shows
  `rsa v0.9.10 ← authkestra-engine ← authkestra-op ← vpay-db ←
  vpay-api/vpay-server/vpay-worker-bin`, no `(dev)` marker anywhere on that
  path (confirmed by running the command against this tree). `cargo deny
  check` still exits 0; nothing here is a CI regression, but "no shipping
  binary pulls it in" is no longer accurate.
- `oauth_client_assertion_jtis` **is now being written to** (every `/v1`
  token request records a `jti`), and still has no cleanup *job*. The
  stopgap that landed instead is `vpay_db::delete_expired_client_assertion_jtis`,
  called **once at `vpay-server` boot**, non-fatally: it bounds the table at
  "assertions since the last restart" rather than "assertions forever". A
  long-lived process still grows it monotonically. The worker's job loop
  (Phase 5) should call this on a timer — schedule this function, do not
  replace it.
- No config hot-reload ([ADR-0003](adr/0003-yaml-configuration.md)): merchant
  onboarding stays a PR-then-deploy, and a rolling deploy has a real window
  where old and new pods disagree about the client list.
- **Open — Keycloak/ZITADEL comparison, parked.** [ADR-0009](adr/0009-dashboard-oidc-provider.md)
  records vsms's own recommendation to compare Authkestra against
  Keycloak/ZITADEL "before milestone 1, not after." Still hasn't happened.
- **Open — access-token TTL and the revocation mitigation.** ADR-0009: "the
  mitigation this decision implies is short access-token TTLs... and/or a
  server-side deny-list. Which of these vpay will actually build is not
  decided." **Still open.** A constant now exists —
  `vpay_api::op::ACCESS_TOKEN_TTL_SECS = 900` — but it is a default this
  code picked, not an answer: no ADR states it, it is not configurable, and
  no deny-list exists. The disabled-clients kill switch acts on *issuance*
  only, so a stolen token stays valid for its remaining 900 s.
- **Open — signing-key rotation overlap window.** **Still open, and now
  concrete.** Key generation and rotation-on-boot exist, and
  `vpay_api::op::keys::ROTATION_OVERLAP` is 24 h — again a default this code
  picked, recorded in no ADR and not configurable. The only property under
  test is that it comfortably exceeds the access-token TTL
  (`the_rotation_overlap_dwarfs_the_access_token_ttl_it_has_to_cover`,
  `the_access_token_ttl_fits_inside_the_key_rotation_overlap`), not that 24 h
  is the right length. A maintainer should settle it together with the TTL
  above, since the two are related by that constraint.
- **Open — the `disabled_clients` + YAML dual-authority runbook.** ADR-0010
  says a future revocation runbook must check both explicitly. None exists
  in `docs/runbooks/` yet.
- **Open — the revocation-endpoint gap on `authkestra-op` itself.**
  ADR-0009: a stolen access token cannot be revoked mid-lifetime through the
  OP. Whether vpay builds a deny-list or accepts this as bounded by TTL is
  the same open call as the TTL question above.
- `authkestra-op` has no `/token` rate limiting, deliberately left to
  Kubernetes ingress (ADR-0009 Consequences) — not this phase's problem to
  solve, but worth confirming ingress config actually does it before relying
  on the assumption. **`/v1/oauth/token` is now publicly reachable and
  unauthenticated by necessity (the credential is the request body), so this
  moved from theoretical to live on 2026-09-02.** Nothing in this repository
  rate-limits it or verifies that anything else does.
- **New, 2026-09-02 — the resource validator fetches its JWKS over loopback
  HTTP from its own process.** `vpay-server` binds first, then builds the
  validator against `http://127.0.0.1:{bound_port}/v1/oauth/jwks.json`. It is
  always loopback, never the public URL (unit-tested:
  `the_validators_jwks_url_is_always_loopback_on_the_bound_port`), so no
  external dependency is added — but a process validating its own tokens by
  asking itself over TCP exists because `authkestra_resource` offers no
  in-process key source, not because anyone wanted it. Worth revisiting if
  upstream grows one.
- **New, 2026-09-02 — the signing-key PEM is not zeroized.** It is read into
  a `String` and dropped normally, so key bytes may linger in freed heap.
  `vpay_api::op::keys`'s module docs state this deliberately rather than
  implying the handling is airtight; closing it means a `zeroize`-backed
  secret-string type, which is its own change.

---

## Phase 2b — Dashboard login (`/dash/v1`)

**Open — build the dashboard on CrateStack's refine integration?** (noted
2026-09-02.) `@cratestack/refine` ships a tested refine.dev `DataProvider`
over a CrateStack-generated REST/RPC client, and `cratestack
generate-typescript --refine` emits the resource manifest from a `.cstack`
schema — so most of an operator admin panel would be generated rather than
hand-written in the Next.js scaffold. The price is that `schemas/vpay.cstack`
would have to become an authoritative *service* model for the staff surface
(it is a design sketch today, excluded from the build graph and already
diverged from the migrations on two `CHECK` constraints), served by a
CrateStack service beside the hand-written Stripe-shaped `/v1`, which stays
as it is. That is an ADR-level decision (it touches ADR-0008 and the
migrations' status as the schema of record) and is not made here.

**Split out of Phase 2 on 2026-09-02**, when the merchant half of that phase
landed and the dashboard half did not. This is a bookkeeping change, not a
re-plan: Phase 2's own **Unblocks** paragraph already said `/dash/v1` login
is a parallel deliverable and not a prerequisite for Phases 3–6. Giving it
its own heading stops "Phase 2 is done" from ever being read as "login
works". Nothing below is new scope; it is Phase 2's dashboard scope, moved.

**Goal.** A staff member completes an authorization-code + PKCE login
against `/dash/v1` and a subsequent authenticated call accepts the token; a
merchant-audience token is rejected on `/dash/v1`.

**Status.** Not started. **No login has ever been performed and no
`/dash/v1` route exists.** What Phase 2 left behind for it: the schema
(migrations `0006`/`0013`, proven compatible with the real
`SqlxOpStore<Postgres>`), the dashboard client modelled and validated in
config (`vpay_config::oauth::DashboardClient`), signing keys and a JWKS
endpoint, and `JwtValidator`/`AuthenticatedDashboard` pinned to
`Surface::Dashboard` and unit-proven to reject a merchant-audience token.
None of that is login.

**Scope.**
1. A `SessionStore`. `authkestra-engine` is pinned
   `features = ["rustls-no-provider", "token", "session"]` — **without
   `sql-postgres`** — so no SQL-backed session store is compiled into the
   workspace today. Enabling that feature is a supply-chain change
   (`sqlx/chrono`, `sqlx/json`) and needs `cargo deny` re-run.
2. `/login`, `/authorize` and `/userinfo`, plus the callback the dashboard
   needs. Phase 2 mounted none of them, deliberately: no merchant client can
   use the authorization-code grant, and `authkestra-axum` is not a
   dependency (its bundled router would mount them and would publish a
   one-key JWKS instead of the rotation window vpay serves).
3. **Resolve the audience problem first.** `authkestra-op`'s
   `default_handle_authorization_code` mints the access token with
   `Some(client_id)` as the audience and has **no requested-audience path at
   all** (`authkestra-op-0.7.1/src/handlers/token.rs`, step 7). A token from
   that grant would carry `aud = <client_id>`, and
   `Surface::Dashboard.audience()` (`vpay:dash/v1`) rejects every one of
   them. `handle_client_credentials` *does* honour a requested audience,
   which is why `/v1` does not hit this. Options — a custom grant handler, a
   different `Surface::Dashboard` audience rule, or an upstream change —
   are a maintainer's call, not a default to pick in passing.
4. The dashboard's own server-side session handling ([ADR-0008](adr/0008-dashboard-scope.md):
   the dashboard never holds a merchant API key and calls `/dash/v1`
   server-side under an OIDC session).

**Definition of done.**
- An integration test drives a real authorization-code + PKCE round trip
  against `/dash/v1` and receives a token a subsequent authenticated call
  accepts.
- A merchant-audience token is rejected on `/dash/v1`, over a real mounted
  router — the missing half of the pair Phase 2 could only prove in one
  direction.
- A signing key is rotated at least once and a token minted under the old
  key still verifies for the whole of its lifetime. **Runtime rotation does
  not exist** (Phase 2, scope item 3): rotation is restart-based today, so
  this bullet needs either a rotation mechanism or an explicit decision that
  restart-based rotation is the answer, written down.

**Decisions and open questions.** Every one Phase 2 lists still applies here
— the dashboard scope ([ADR-0008](adr/0008-dashboard-scope.md)), no refresh
tokens, the Keycloak/ZITADEL comparison ADR-0009 asked for and nobody has
done, and the revocation-endpoint gap.

---

## Phase 3 — The payment API (`/v1`)

**Goal.** Merchants can create and confirm `PaymentIntent`s through `/v1`,
authenticated, idempotent.

**Status.** Not started. The object model and state machine
(`vpay-core::state`) are implemented and tested; nothing routes an HTTP
request through them.

**Scope.**
- `POST /v1/payment_intents` (create) — writes a row via the existing
  `vpay-core` types. **Has no rail dependency**; it can be built and tested
  before Phase 4 lands.
- Idempotency-key handling on create.
- ~~Request-auth middleware consuming Phase 2's merchant token validation.~~
  **Done ahead of this phase, 2026-09-02**: `AuthenticatedMerchant` is
  mounted in front of the whole `/v1` nest, so a route added here is
  authenticated by construction rather than by remembering to add a layer.
  What that nest currently holds is one honest 404.
- `POST /v1/payment_intents/{id}/confirm` — submits to the adapter. **This
  one genuinely depends on Phase 4**: [`docs/flows/payment-lifecycle.md`](flows/payment-lifecycle.md)
  and [`docs/flows/crash-safety.md`](flows/crash-safety.md) both describe
  `confirm` as calling the adapter's `submit()`; until that call is real
  (Phase 4), `confirm` can only reach the `NotImplemented` stub.

**Note on ordering.** Phases 3 and 4 interleave rather than strictly
sequence — `create` doesn't need a rail at all, `confirm` needs one to do
anything beyond call a stub. `docs/status.md`'s MVP checklist lists adapters
(its item 2) ahead of `/v1` (its item 4); read as a strict build order that
disagrees with placing Payment API before The rails here. Read as an
unordered checklist of exit criteria it doesn't — both lists agree on what
must be true, just not on a claimed sequence. This roadmap places `create`
first because it is buildable now, and treats `confirm`'s completion as
gated on Phase 4 regardless of which phase number it sits under.

**Definition of done.**
- An integration test drives create → confirm → a terminal state over real
  HTTP and asserts the object shape at each step.
- `one_charge_per_intent` is proven at the API level (a second confirm
  attempt does not produce a second charge), not just as a bare DB
  constraint test.
- A replayed idempotency key returns the same object without a second row.

**Unblocks.** Webhooks (Phase 6, needs a state change to notify about);
worker (Phase 5, needs charges to poll).

**Decisions this phase rests on.** [ADR-0002](adr/0002-provider-port.md)
(core branches on capability values, never a provider code) governs how
`confirm` picks push vs. redirect handling.

**Risks carried by this phase.**
- `docs/flows/crash-safety.md`'s write-first-network-second discipline
  ("generate the reference, persist it, only then call the rail") is
  documented but unimplemented — this phase is where it has to land, and
  getting the ordering wrong is the exact failure mode the doc exists to
  prevent.

---

## Phase 4 — The rails

**Goal.** The eight `ProviderError::NotImplemented` tokens are replaced with
real HTTP calls, passing the shared conformance suite.

**Status.** Not started. Capabilities are declared and tested (✅); every
wire call is ⛔.

**Scope, in dependency order.**
- `mtn_momo::{submit, query_status, parse_callback, refund}`.
- `orange_money::{submit, query_status, parse_callback, refund}` — with
  `refund` staying a hard `false` per its declared capability
  (`supports_refunds: false`), not a rail-specific branch in the core
  ([ADR-0002](adr/0002-provider-port.md)).
- The push-rail recovery table
  ([`docs/flows/crash-safety.md`](flows/crash-safety.md)): disambiguating a
  `submitting` charge via `provider_requests` (no row → resubmit; row with
  `status_code IS NULL` → poll, 3 consecutive `NotFound` over ≥60s before
  treating as never-received).
- The redirect-rail commit-gates-redirect discipline: `ref_extra` must
  commit before `redirect_to_url` is ever emitted.

**Definition of done.** The shared conformance suite
(`backends/tests/conformance/tests/adapter_conformance.rs`) passes with its
`#[ignore]`s removed, parameterized over both adapters — per `AGENTS.md`'s
own rule, adding a rail means making the one suite pass, never writing a
rail-specific one.

**Unblocks.** A meaningful Phase 3 `confirm`; Phase 5 (nothing to poll
without a real submission); Phase 6 (nothing to sign a webhook about).

**Risks carried by this phase.**
- Rail testing depends on WireMock hosts (`compose.yml`), which in turn
  depends on Phase 7's environment blocker (Docker Hub unreachable here) —
  local conformance-suite runs against stubs may themselves be blocked on
  this machine specifically, independent of the code being ready.

---

## Phase 5 — The worker

**Goal.** Charges in `processing` reach a terminal state without operator
intervention, and a crash at any of three documented points resolves
without double-charging.

**Status.** Not started. `poll_delay` ladder logic is implemented and
tested in isolation; the job loop, the reconciler, and the crash-injection
tests are all ⛔. `vpay-worker-bin` currently stays up answering shutdown
signals and logs a heartbeat stating the loop is not implemented.

**Scope.**
- The job loop consuming charges needing action.
- The poll ladder (already implemented) wired to that loop.
- The reconciler (a distinct MVP item, not [RFC-0001](rfc/0001-settlement-and-payouts.md)'s
  settlement/payouts scope — that RFC is explicitly parked pending a
  licensing conversation and is not part of this roadmap).
- The three crash-test injection points from
  [`docs/flows/crash-safety.md`](flows/crash-safety.md): after the charge
  insert and before any `provider_requests` row; after that row and before
  the rail's response; after the response and before the state update.

**Definition of done.** The crash tests described above all pass, proving
the documented recovery table resolves every injection point without a
double charge — currently a `#[ignore]`d, unwritten test class, not a
green suite with weak assertions.

**Unblocks.** Reliable terminal states for Phase 6 to notify on.

**Risks carried by this phase.**
- `--shutdown-grace-seconds` is accepted and logged on `vpay-worker-bin`
  today but does nothing — there is no drain to bound because there is no
  job loop yet. This phase is where that flag needs to start doing real
  work, or be reconsidered.

---

## Phase 6 — Webhooks

**Candidate, not decided (2026-09-02):** `cratestack-outbox` implements
exactly this shape — `OutboxClient::persist_in_tx` inside the caller's
transaction, `drain` in insertion order, an axum drain/gc handler pair — and
carries no schema macro. Whether to take the dependency or write the
~200-line outbox by hand is for whoever builds this phase; ADR-0006's
"no in-process fake receiver" rule applies either way.

**Goal.** Merchants receive signed webhook notifications for intent/charge
state changes, delivered at-least-once via a durable outbox.

**Status.** Not started (⛔ in `docs/status.md`).

**Scope.**
- An outbox row written in the same transaction as the state change it
  reports.
- A signing scheme matching Stripe's, so existing merchant webhook-handling
  code needs no new verification logic.
- A delivery worker with a retry policy.

**Definition of done.** An integration test proves a state transition never
commits without a corresponding outbox row in the same transaction; a
delivery test proves signature verification succeeds against a real HMAC
computation, driven against a real or WireMock receiver — never an
in-process fake ([ADR-0006](adr/0006-no-mocks-in-main-processes.md)).

**Unblocks.** Nothing further downstream on the payments path — this is the
last functional phase before the MVP claim in `docs/status.md` can move.

**Risks carried by this phase.** None new beyond the standing no-mocks
constraint above.

---

## Phase 7 — Operability

**Goal.** The whole stack (server, worker, Postgres, two rail stubs) starts
with one command, the e2e specs run green against it, and runbooks have
been walked against a real fault, not just written.

**Status.** *Corrected 2026-09-02:* this used to say "blocked by
environment, not by unwritten code," and that was wrong on two counts. The
`ci` workflow had run five times and failed five times at the same
self-inflicted step (`CYPRESS_INSTALL_BINARY: 0` set for the very job that
runs Cypress), and the compose stack could not have booted even with
registry access — nothing supplied the `VPAY_CONFIG` both binaries require,
and the image did not contain a config file to name. Both are fixed; see
`docs/status.md`'s "GitHub Actions" and "Docker / compose" entries for what
is proven by what. The environment description that follows is still true
of the original authoring machine and is kept for context. Docker Hub is
unreachable from this development machine: `docker pull alpine:3.22` did
not complete in five minutes, and `rust:1.95.0-alpine3.22`,
`node:22-alpine`, `wiremock/wiremock:3.9.2` and `docker/dockerfile:1` are
all missing from the local image cache and unpullable here. Only
`postgres:16-alpine` is cached, which is why Phases 1–4's Postgres-backed
tests work fine while this phase's own exit criteria cannot be met on this
machine at all. Cypress is blocked the same way: its binary needs
`pnpm exec cypress install` against a CDN this environment cannot reach.

**This blocker is independent of the other phases and can be lifted at any
time on a machine with real registry/CDN access** — it does not need
Phases 2–6 to finish first to start being *attempted*. What it does need
those phases for is a *meaningful* green run: the one Cypress spec that
exists today (`frontends/tests/e2e/cypress/e2e/dashboard.cy.ts`) only
exercises the dashboard's scaffold notice, not a real payment flow, so
running it green proves the environment works before it proves anything
about payments.

**Scope.**
- Build `backends/Dockerfile` and `frontends/Dockerfile` to completion (both
  rewritten this cycle — musl target, non-root UID 65532, `.dockerignore` —
  neither ever built).
- Bring up `compose.yml`/`compose.e2e.yml`.
- Install the Cypress binary and run the existing spec(s); write the
  payment-flow specs that give this phase something meaningful to prove
  once Phases 3–6 exist.
- Walk each `docs/runbooks/*` document against a real, not imagined,
  instance of the condition it addresses.

**Definition of done.** `just test-e2e` exits 0 against the compose stack;
both Dockerfiles build to completion; each runbook's steps have been
exercised at least once for real.

**Unblocks.** Nothing further — this is "can we run what we built,"
verified, not a functional dependency of any other phase.

**Risks carried by this phase.** The environment blocker itself: until
Docker Hub / the Cypress CDN is reachable from wherever this work continues,
this phase cannot be closed regardless of how much other work is done.

---

*Written 2026-08-11 against `master` at `33f2913`. If this page and the code
disagree, the code — and `docs/status.md`'s machine-checked account of it —
is correct.*

**Addendum, 2026-09-02.** Two things landed that this snapshot does not
place in a phase: the merchant SDKs (`sdks/rust`, `sdks/nodejs`) implement the
*client* half of Phase 3's `/v1` contract — pinned down in
[`docs/flows/merchant-auth.md`](flows/merchant-auth.md) — ahead of any server
route existing, so Phase 3 now has a consumer to build against; and the
dependency floor moved (`authkestra-*` 0.5.4 → 0.7.1 with migration `0013`,
CrateStack re-verified at 0.10.1). See `docs/status.md` for the row-by-row
account.

**Second addendum, 2026-09-02 (Step 1).** Phase 2's "assembled but not
mounted" status is no longer accurate for the merchant half: the OP is
mounted at `/v1/oauth` and `/v1` has an authentication boundary in front of
it. The dashboard half is unbuilt and is now Phase 2b. The phase's own
Status block carries the detail, including the one thing that matters most
about the evidence — the six integration tests that cover the flow have run
once, manually, against a scratch database, and never under Docker or in
CI. Phases 3–7 are untouched.
