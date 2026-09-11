# Roadmap — Phase 2: Authentication (OP assembly and mounting)

_Moved out of [docs/roadmap.md](../roadmap.md) on 2026-09-11 by exp57, which split a 1 504-line page into one page per phase. **The text below is the original, unedited** — every struck-through claim, every "Open —" maintainer question and every dated amendment is here as it was written, because on this page those are most of the content; only the relative links gained a `../` because the file moved one directory down._

**The open maintainer questions live here**: the Keycloak/ZITADEL comparison, the
access-token TTL and its revocation mitigation, the signing-key rotation overlap
window, and the revocation-endpoint gap on `authkestra-op` itself. Doc comments in
`backends/crates/vpay-api/src/op/` and `backends/crates/vpay-db/src/` cite them by
name. A default chosen in code is not an answer to one.

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
> below describe this phase as it was _planned_, and are left standing
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
>
> **Update, 2026-09-03 (evening): the "this is where the repo is now"
> sentence in the Status paragraph above is history, and is left standing
> rather than edited.** The marker moved to Phase 7 in the table at the top
> of this page. Phases 3, 4a, 4b/5 and 6 have all landed since, along with
> two deliverables this document had no phase for (5b and 5c), and the merchant
> half of this phase now has CI evidence rather than one manual run — CI's
> `rust` job is green on `master` (run `33792230584`). The dashboard half
> (Phase 2b) has not moved at all.

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
   (_corrected 2026-09-02 against the pinned `=0.7.1` `store.rs`; the
   original text said five, from `0.3.4`_ — `P` is the DPoP replay store,
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
   the file, and a rollback to a retired `kid` is refused. **The "no runbook
   describes the sequence" half of this is fixed as of 2026-09-03 (Step 6,
   block C):** [runbooks/rotate-signing-key.md](../runbooks/rotate-signing-key.md)
   documents the restart-based rotation, the 24 h overlap window, and the
   exit-78-not-69 crash loop a rollback to a retired `kid` produces. Runtime
   rotation itself still does not exist, and the runbook has never been
   followed against a deployment.
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
  [ADR-0008](../adr/0008-dashboard-scope.md) (accepted) still describes
  per-record write actions (re-poll/replay/refund/annotate) as the
  architecture. [`docs/flows/dashboard-auth.md`](../flows/dashboard-auth.md)'s
  "Scope" section is the reconciliation: no mutating use case is being built
  now, so the client registration requests exactly one read-only scope and
  no `audit_log`-writing code exists — a sequencing decision, not an
  architectural reversal.
- **No refresh tokens on either surface.** `/v1`
  ([ADR-0010](../adr/0010-merchant-auth-private-key-jwt.md)) matches RFC 6749
  §4.4.3 and `authkestra-op`'s own hardcoded `refresh_token: None` on the
  `client_credentials` handler. `/dash/v1`
  ([`docs/flows/dashboard-auth.md`](../flows/dashboard-auth.md), Token
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
  (`vpay_db::client_assertion_store`, `INSERT … ON CONFLICT DO NOTHING`,
  proven race-safe by a 10-way concurrent test). There was never a drop-in
  Redis option to choose against _at the time_: `authkestra-op` shipped no
  Redis-backed store at `0.3.4`, confirmed against an open upstream issue,
  [marcjazz/authkestra#185](https://github.com/marcjazz/authkestra/issues/185)
  ("no Redis-backed OpStore or ClientAssertionStore, though the docs point
  integrators at Redis"). _Stale as of `=0.7.1`_ (noted 2026-09-02):
  `authkestra-op-0.7.1/src/redis_store.rs` ships a
  `RedisClientAssertionStore`. The Postgres decision stands on its own
  reasoning — durability, one fewer moving part — but no longer on "there
  was nothing to choose against." Revisit on a measured trigger, not on
  principle.
- **`disabled_clients` supplements YAML identity as a kill switch.**
  ADR-0010: YAML stays authoritative for identity; the table only ever
  _subtracts_ access, so revocation is an `INSERT`, not a deploy. Cost: a
  correct "is this client allowed" answer needs checking both —
  [runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md)
  §5 documents that check as of 2026-09-03, having never been followed
  against a deployment.
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
  token request records a `jti`), and still has no cleanup _job_. The
  stopgap that landed instead is `vpay_db::delete_expired_client_assertion_jtis`,
  called **once at `vpay-server` boot**, non-fatally: it bounds the table at
  "assertions since the last restart" rather than "assertions forever". A
  long-lived process still grows it monotonically. The worker's job loop
  (Phase 5) should call this on a timer — schedule this function, do not
  replace it.
- No config hot-reload ([ADR-0003](../adr/0003-yaml-configuration.md)): merchant
  onboarding stays a PR-then-deploy, and a rolling deploy has a real window
  where old and new pods disagree about the client list.
- **Open — Keycloak/ZITADEL comparison, parked.** [ADR-0009](../adr/0009-dashboard-oidc-provider.md)
  records vsms's own recommendation to compare Authkestra against
  Keycloak/ZITADEL "before milestone 1, not after." Still hasn't happened.
- **Open — access-token TTL and the revocation mitigation.** ADR-0009: "the
  mitigation this decision implies is short access-token TTLs... and/or a
  server-side deny-list. Which of these vpay will actually build is not
  decided." **Still open.** A constant now exists —
  `vpay_api::op::ACCESS_TOKEN_TTL_SECS = 900` — but it is a default this
  code picked, not an answer: no ADR states it, it is not configurable, and
  no deny-list exists. The disabled-clients kill switch acts on _issuance_
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
- ~~**Open — the `disabled_clients` + YAML dual-authority runbook.**~~
  **Written 2026-09-03 (Step 6, block C.)**
  [runbooks/rotate-rail-credentials.md](../runbooks/rotate-rail-credentials.md)
  §5 documents the check ADR-0010 requires: YAML `merchant_clients` for
  identity, `disabled_clients` for subtraction, the order to ask the two
  questions in, the `INSERT`/`DELETE` that revoke and un-revoke, and the fact
  that the switch acts on _issuance_ only, so an already-issued token stays
  valid for its remaining 900 s. It also says to re-check the table after a
  database restore, because a restore silently un-revokes. **The runbook has
  never been followed against a deployment**, and `disable_client` /
  `enable_client` are still called by no shipping code — an operator flips
  the row by hand. The half the switch cannot cover, revoking a token already
  issued, is the next item and is still open.
- **Open — the revocation-endpoint gap on `authkestra-op` itself.**
  ADR-0009: a stolen access token cannot be revoked mid-lifetime through the
  OP. Whether vpay builds a deny-list or accepts this as bounded by TTL is
  the same open call as the TTL question above.
- `authkestra-op` has no `/token` rate limiting, deliberately left to
  Kubernetes ingress (ADR-0009 Consequences) — not this phase's problem to
  solve, but worth confirming ingress config actually does it before relying
  on the assumption. **`/v1/oauth/token` is now publicly reachable and
  unauthenticated by necessity (the credential is the request body), so this
  moved from theoretical to live on 2026-09-02.** ~~Nothing in this
  repository rate-limits it or verifies that anything else does.~~
  **Half-corrected 2026-09-03 (Step 6, block B):** the chart renders a
  separate `Ingress` for `/v1/oauth/token` carrying a tighter
  `nginx.ingress.kubernetes.io/limit-rps` than the one on `/v1`
  (ingress-nginx applies the limit per Ingress object, so one object cannot
  carry two), a `rate-limit-ordering` template guard refuses values where the
  token limit is the looser of the two, and `just helm-check` greps the
  rendered YAML for the annotation. **That is a check on rendered YAML and
  nothing more** — no ingress controller has ever honoured, or been asked to
  honour, either limit, and nginx enforces `limit-rps` per controller
  replica, so the effective global limit is approximately
  `limit-rps × replicas`. Still open in the sense that matters.
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
