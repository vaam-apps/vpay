# Roadmap

**Snapshot date: 2026-08-11, last refreshed 2026-09-04 (the Step 9 addendum at the foot of this page).** A point-in-time read
of the repository, for someone opening it cold or coming back after a break,
organized as the sequence of phases from scaffold to a deployable gateway. It
goes stale the moment code moves and nobody is obliged to update it on the
same schedule as a commit. **On any disagreement,
[`docs/status.md`](status.md) and the ADRs in [`docs/adr/`](adr/) win,
always** — they are the machine-checked and immutable sources of truth
respectively. This page does not restate
`docs/status.md`'s per-feature rows; if you're here to check whether some
specific thing works, go there instead.

**vpay still cannot take a payment — with real money.** *That sentence used to
continue "no HTTP call to any rail has ever been made by this code", which
stopped being true on 2026-09-03: both adapters call rails, and since Step 4 the
worker drives a confirmed intent all the way to `succeeded`. **Every one of
those calls went to a WireMock host — no real rail has ever been called, and no
money has ever moved.***

*Refreshed 2026-09-03 (evening), and stated precisely in both directions.*
**What is proven**, by tests that fail if it breaks, on a developer machine and
in CI: a merchant authenticates on `/v1` with `private_key_jwt`, creates a
`PaymentIntent`, confirms it, the worker polls the charge and drives the intent
to `succeeded` with nobody touching it, and a signed `payment_intent.succeeded`
webhook is delivered and verified — by both shipping SDKs and by the official
`stripe` package's own `constructEvent`. A payer's own browser can drive the
push half of that through `@vaam-apps/vpay-stripe-js` without a merchant credential.
**What is not proven, and is what stands between this and taking money**: every
rail in every one of those runs is a `wiremock/wiremock` host, **no real rail
has ever been called**; every webhook receiver is one too, **no merchant
endpoint has ever been POSTed to**; and **nothing has ever run on a cluster**.
Everything below explains what stands between here and that no longer being
true.

| # | Phase | Status, with the evidence that backs it |
|---|---|---|
| 1 | Foundations | ✅ Complete — seven commits through `932d8a4`; the **28** migrations in `backends/migrations/` apply cleanly and their constraints are proven to fire (`postgres_smoke.rs`; 26 when this row was written, 27 with Step 8's callback index and 28 with Step 9's `checkout_sessions`) |
| 2 | Authentication — merchant (`/v1`) | ✅ Delivered 2026-09-02 (Step 1, PR #15) — the OP is mounted at `/v1/oauth` and `AuthenticatedMerchant` gates the whole `/v1` nest; 7 `merchant_token_flow` tests. *`docs/status.md`'s own "Merchant auth" row is still 🟡 and names its own trigger — "when the CI `rust` job runs them green" — which has since happened on `master` (run `33792230584`) without that row being re-measured* |
| 2b | Authentication — dashboard login (`/dash/v1`) | ⛔ Not started, unchanged — no `/login`, no `/authorize`, no `SessionStore`, no `/dash/v1` route of any kind; split out of Phase 2 on 2026-09-02 |
| 3 | Payment API (`/v1`) | ✅ Delivered 2026-09-02→03 (Steps 2–3, PRs #16–#17) — create / retrieve / list / cancel / confirm, form-encoded, idempotent and merchant-scoped, with `confirm` moving the intent to `processing` or `requires_action`. **Against WireMock rails**, which is why the matching `docs/status.md` rows are 🟡 |
| 4 | The rails | → **split 2026-09-03.** **4a** ✅ delivered (Step 3, PR #17) — both adapters pass the one shared conformance suite, 26 tests, 0 `#[ignore]`s, every one of them against a `wiremock/wiremock` container. **4b** (push-rail recovery) delivered inside Phase 5 (Step 4, PR #18). The two headings below are current; this row is the pre-split one |
| 5 | The worker | 🟡 In progress — the job loop, the poll ladder, recovery and settlement landed 2026-09-03 (Step 4, PR #18) against WireMock rails, and a confirmed intent reaches `succeeded` unattended. **Updated 2026-09-04 (Step 8):** ~~the callback route (`POST /provider/{code}/callback`) … did not~~ — **it exists now** (lane C), and ~~the "crash tests" kill no process~~ — **`worker_kill9.rs` `SIGKILL`s the shipping worker and the shipping server** (lane D), so two of the three kill points are caused rather than written. Lane G additionally fixed a `500` on confirm that this step's demo found. **Still 🟡:** prompt expiry is unbuilt, kill point 1 is still written rather than caused, Orange is not in the kill test, no rail has ever called the callback route, and every rail here is a WireMock host |
| 5b | Stripe SDK compatibility on `/v1` | ✅ Delivered 2026-09-03 (PR #20) — the real `stripe@22.6.1` package driven out of process against the compose stack: `sdks/stripe-compat`, **25 cases, 0 skipped**, run by CI's `e2e (compose)` job; [flows/stripe-sdk-compat.md](flows/stripe-sdk-compat.md). *`docs/status.md`'s row is 🟡 for the standing limit — the rail and the receiver are both WireMock hosts* |
| 5c | Stripe.js-compatible browser checkout | ✅ Delivered 2026-09-03 (PR #22), **push rails only** — `/v1/browser` plus `@vaam-apps/vpay-stripe-js`, no merchant credential in the browser, proven by `checkout.cy.ts` against the compose stack. ~~**The redirect return trip is a named gap**: there is no bounce endpoint, so an Orange checkout must not be shipped on this package~~ — **retired 2026-09-04 by Phase 5d**: the rail is told a per-charge return URL and vpay serves the page that receives the payer ([flows/browser-checkout.md](flows/browser-checkout.md), [flows/hosted-checkout.md](flows/hosted-checkout.md)) |
| 5d | Hosted and embedded checkout | ✅ Delivered 2026-09-04 (Step 9), **against WireMock rails** — a `checkout.session` object (`cs_…`, migration `0028`) with a hosted mode (vpay mints a `url`) and an embedded mode (the merchant frames vpay's page), `frontends/apps/checkout` in French and English, the redirect return trip that closes 5c's named gap, `initEmbeddedCheckout` in `@vaam-apps/vpay-stripe-js` and `checkout.sessions` in both merchant SDKs, a fourth image and a Helm workload, and `examples/shop` — a merchant site a human can buy from. Proven in a real browser end to end by `shop-hosted.cy.ts` and `shop-embedded.cy.ts`. [flows/hosted-checkout.md](flows/hosted-checkout.md), [runbooks/checkout.md](runbooks/checkout.md). *`docs/status.md`'s rows are 🟡 where this is ✅, and for reasons this row must not hide: no browser has been observed enforcing vpay's `frame-ancestors` (Cypress strips it), no pod has ever run the page, and the rails are stubs* |
| 6 | Webhooks | 🟡 In progress — the outbox drain, signing and delivery landed 2026-09-03 (Step 5, PR #19) against a WireMock receiver. **Updated 2026-09-04 (Step 8, lane B):** ~~there is no SSRF protection of any kind~~ — **a runtime egress guard exists**, `vpay_worker::ssrf`, which resolves each endpoint's host once, refuses every non-public address in both families and pins the connection to what it classified; boot-time `validate_host` is still only a stub-host guard and never was an address check. **Still 🟡:** no merchant endpoint has ever been POSTed to, the guard has never refused a real one, delivery is unordered, replaying an exhausted delivery is a hand-written transaction, and the pin cost the shared connection pool |
| 7 | Operability — including Step 6's groundwork (PR #21) | 🟡 In progress — **the repo is here now**, on Step 7 (see below). No longer "blocked by environment": CI runs the compose stack, both Cypress specs and the stripe-node conformance suite; the Helm chart is linted, rendered and kubeconform-checked by CI's `deploy` job; `release.yml` has built and signed images on every push to `master` since Step 6; ~~`just demo` runs seven steps~~ — **as of 2026-09-04 (Step 8, lane A) `just demo` is four steps whose fourth is six payments across both rails**, with `demo-up`/`demo-walk`/`demo-status`/`demo-down` split out, two stacks able to coexist on one machine, and [runbooks/demo.md](runbooks/demo.md) as the procedure with real pasted output. **Not done: no cluster, no Prometheus scrape, no runbook walked against a real fault, no real rail behind any of it — and the demo has not yet been run on the merged Step 8 gate branch** |

**Where the repo is now, 2026-09-03 (evening).** Steps 5b, 6 and 5c landed on
`master` in that order (PRs #20, #21, #22), followed by a frontend dependency
audit and a CI timeout fix (PRs #23, #24). The work in flight is **Step 7, the
cleanup rework** ([plans/2026-09-03-step7-cleanup-rework.md](plans/2026-09-03-step7-cleanup-rework.md)),
on a branch and unmerged: repository traits hiding the `sqlx` implementations
behind an interface, a real `#[source]` on the adapter transport/malformed
errors so a rail failure keeps its cause, doctests so the documentation cannot
lie, and flow prose moved out of long module comments into `.md` files. **It
moves no capability.** Nothing in the table above changes when it lands, and the
conformance, integration and SDK suites are the guard that says so.

**On the markers in that table.** ✅ means "delivered, with a test that would
fail if it broke" — it does not mean "safe with money". Every ✅ above is proven
against WireMock rails and a WireMock receiver, which is why most of the
matching per-feature rows in [`docs/status.md`](status.md) are 🟡. Where this
page and that one disagree, that one wins.

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
  and constraint-tested against a real database. *This is Phase 1's scope,
  not the repository total: there are **20** migrations as of 2026-09-03
  (`0013` with the authkestra upgrade, `0014`–`0018` with Step 2, of which
  `0017` and `0018` are schema only, and `0019`–`0020` with Step 3). See the
  Phase 3 addenda.* **Corrected 2026-09-03 (evening): that "20" was true when
  it was written and is now wrong — `ls backends/migrations | wc -l` answers
  26.** `0021`–`0024` came with the worker and the webhook outbox (Steps 4 and
  5), `0025` with the Stripe-SDK retry advice (Step 5b) and `0026` with
  `client_secret` (Step 5c). Phase 1's own scope is still the first twelve.
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
(`a_missing_config_is_exit_78_naming_the_problem` and siblings in
each binary's `tests/cli.rs`); `/healthz` returns 200 or 503 based on a real
`SELECT 1`, not a static string; all migrations apply cleanly and
idempotently with their constraints proven to fire in
`backends/tests/integration/tests/postgres_smoke.rs` — 12 when this phase
closed, **26** as of 2026-09-03 (evening; this line said 20 until then —
`schema_migrates_cleanly_on_an_empty_database` applies whatever is in
`backends/migrations/`, so the count is not pinned here and a stale number in
this document never failed a build).

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
   the file, and a rollback to a retired `kid` is refused. **The "no runbook
   describes the sequence" half of this is fixed as of 2026-09-03 (Step 6,
   block C):** [runbooks/rotate-signing-key.md](runbooks/rotate-signing-key.md)
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
  (`vpay_db::client_assertion_store`, `INSERT … ON CONFLICT DO NOTHING`,
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
  correct "is this client allowed" answer needs checking both —
  [runbooks/rotate-rail-credentials.md](runbooks/rotate-rail-credentials.md)
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
- ~~**Open — the `disabled_clients` + YAML dual-authority runbook.**~~
  **Written 2026-09-03 (Step 6, block C.)**
  [runbooks/rotate-rail-credentials.md](runbooks/rotate-rail-credentials.md)
  §5 documents the check ADR-0010 requires: YAML `merchant_clients` for
  identity, `disabled_clients` for subtraction, the order to ask the two
  questions in, the `INSERT`/`DELETE` that revoke and un-revoke, and the fact
  that the switch acts on *issuance* only, so an already-issued token stays
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

**Status.** In progress — see the 2026-09-03 addendum at the end of this
phase. *This line said "Not started" until then.* The object model and state
machine (`vpay-core::state`) are implemented and tested, and four
`/v1/payment_intents` paths now route HTTP requests through them; `confirm`
reaches the rail adapter and stops at its `NotImplemented`.

*Addendum, 2026-09-03 (evening): two later deliverables sit **on top of** this
surface without changing its contract — the official Stripe SDK path (Step 5b,
PR #20, [flows/stripe-sdk-compat.md](flows/stripe-sdk-compat.md)) and the
unauthenticated browser surface a payer's own page calls (Step 5c, PR #22,
`/v1/browser`, [flows/browser-checkout.md](flows/browser-checkout.md)).
Migrations `0025` and `0026` came with them; the repository total is now 26,
not the 20 the Step 3 addendum below records.*

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
  prevent. *Landed for `confirm` on 2026-09-03; the recovery half did not —
  see the addendum.*

### Status addendum — 2026-09-03 (Step 2, branch `claude/step2-payment-intents`)

**Done, with the test that would fail if it broke.** Everything below ran
against a real `postgres:16-alpine` on the authoring machine on 2026-09-03
(74 container-backed tests, 0 failures); ~~it has not run in CI.~~
**Retired 2026-09-05: it runs in CI.** CI run `33929374663` (2026-09-04,
`master`, head `33d6c25`) ran `cargo nextest run --workspace` on
`ubuntu-latest` to 1159 tests run, 1159 passed, **0 skipped** — 163 of them
`vpay-tests-integration` and 86 `vpay-db`, the container-backed crates.

- **`POST /v1/payment_intents`** — form-encoded, validated, merchant-scoped,
  idempotent, writing a real row
  (`create_then_retrieve_round_trips_through_the_sdk`).
- **`GET /v1/payment_intents/{id}`**, with another merchant's id answering a
  byte-identical 404 (`merchant_b_cannot_read_merchant_as_intent`).
- **`GET /v1/payment_intents`** — keyset pagination over a new `seq` column
  (`list_pages_forward_and_backward_with_cursors`,
  `a_list_refuses_two_cursors_and_a_malformed_one`).
- **`POST …/cancel`** — a compare-and-swap that also refuses while a charge
  is live (`cancel_is_legal_only_from_requires_payment_method`,
  `a_confirmed_intent_cannot_be_canceled`).
- **Idempotency on every `POST`**, required rather than optional: replay,
  mismatch, in-flight, release-on-`5xx`, reclaim-expired and sweep, each with
  a named test (see `docs/status.md`'s Idempotency row).
- **Request-auth middleware (D3)** validating once, resolving the tenant, and
  checking `payments:write` / `payments:read`
  (`a_client_registered_for_no_scopes_is_forbidden_while_a_scoped_one_is_not`).
- **Boot step 4** — `vpay_db::ConfigReconcile::reconcile`, one
  advisory-locked transaction in both binaries, with a YAML rail that has no
  linked adapter exiting `78`
  (`a_provider_code_with_no_linked_adapter_is_exit_78`).
- **Five migrations**, `0014`–`0018`. *This document's Phase 1 scope line
  says "All 12 Postgres migrations (`0001`–`0012`)"; that remains an accurate
  description of **Phase 1's** scope, and is not the repository total. The
  repository now has **18** (`0001`–`0018`): `0013` landed with the
  authkestra upgrade, `0014`–`0016` are Step 2's working schema, and `0017`
  (`refunds`) and `0018` (`events`) are **schema only — no code reads or
  writes either table**.*

**The three "definition of done" items above are all still unmet, and none of
them can be met before Step 3 / Phase 4:**

- *"create → confirm → a terminal state over real HTTP"* — **unmet.**
  `confirm` reaches `adapter.submit(..)` and receives
  `ProviderError::NotImplemented`, which is a real `501`. No intent has ever
  reached `processing`, `requires_action` or `succeeded`. The terminal state
  in this criterion requires a rail.
- *"`one_charge_per_intent` proven at the API level"* — **met in the half
  that does not need a rail**, and stated exactly: a second confirm produces
  no second charge (`a_second_confirm_cannot_produce_a_second_charge`, with
  `a_second_charge_for_one_intent_is_refused_as_a_named_unique_violation`
  under it). What is not proven is the same property across a *successful*
  submission, because there has never been one.
- *"a replayed idempotency key returns the same object without a second
  row"* — **met**
  (`a_replayed_idempotency_key_returns_the_same_object_and_no_second_row`).

**Also not done in this phase, and not hidden by the above:** `next_action`
is never populated and a redirect `return_url` is validated and then dropped
(no column); there is no recovery pass reading the `submitting` charges and
status-less `provider_requests` rows that `confirm` deliberately leaves
behind; `/v1/refunds`, `/v1/events` and `/v1/balance` are unrouted; the
worker sweeps nothing; and the Node SDK has still never spoken to a running
vpay.

### Addendum to the addendum — 2026-09-03 (Step 3, branch `claude/step3-rails`)

**Three of the items just above are closed by Phase 4a, and one criterion
moves from "unmet" to "half-met".**

- **`confirm` now moves the intent.** `processing` on a push rail,
  `requires_action` with a `next_action.redirect_to_url` on a redirect rail,
  `409 charge_declined` on a decline, `502` when the rail is unreachable —
  seven integration tests in `backends/tests/integration/tests/confirm_rails.rs`
  against real Postgres **and** WireMock containers.
- **`next_action` is populated**, and only ever from the committed charge
  row (`redirect_confirm_commits_the_rails_material_before_it_answers`).
- **`return_url` has a column** (`charges.return_url`, migration `0019`, with
  length and scheme CHECKs) and is committed before the rail is called.
- *"create → confirm → a terminal state over real HTTP"* — **still unmet, and
  the reason changed.** The HTTP is real but the rail is a stub, and no
  terminal state is reached by anything: `succeeded` requires a poll, and
  nothing polls. That is Phase 4b/Phase 5.
- *"`one_charge_per_intent` proven at the API level"* — the half that needed
  a successful submission is now proven too: a confirm that succeeds still
  cannot produce a second charge, and a retry after a *lost* submit is
  refused with "poll, do not create a new PaymentIntent".

**Migration count, corrected:** the repository now has **20**
(`0001`–`0020`). `0019` adds `charges.return_url`; `0020` adds only a column
comment documenting the `provider_requests.status_code = 0` sentinel
(*answered, but the port carries no HTTP status*), changing no data and no
constraint.

---

## Phase 4a — The rail adapters

**Done 2026-09-03** (branch `claude/step3-rails`; Step 3 of the
production-readiness plan). *This phase was "Phase 4 — The rails" until that
day, when it was split: the push-rail recovery table it used to contain moved
to Phase 4b/Phase 5, per Step 3's decision 5. The split is recorded rather
than silently applied because the old phase's Definition of Done was met by
the adapters alone, and anyone reading it afterwards would have believed
recovery had landed.*

**Goal.** Seven of the eight `ProviderError::NotImplemented` tokens replaced
with real HTTP calls, passing the shared conformance suite. *(Eight, in the
original wording. `mtn_momo::refund` stays — MTN refunds are the
Disbursements product, with a subscription key and token scope no deployment
holds — and `orange_money::refund` left the list without being built,
because Orange documents no refund API and the adapter now inherits the
port's permanent `Unsupported` default. See `docs/status.md`.)*

**Status.** Done, against WireMock. Capabilities ✅; `submit`,
`query_status` and `parse_callback` ✅ on both rails against a real
`wiremock/wiremock` container; **the real sandboxes ⛔ — never called**.

**What landed.**
- The port became `#[async_trait]`, `ProviderConfig` gained per-rail
  timeouts, and the vendored-roots HTTP client moved into
  `vpay_provider::http` (redirects refused, proxies ignored, bodies capped
  at 256 KiB).
- `mtn_momo::{submit, query_status, parse_callback}` with a fingerprinted
  token cache and the failure table transcribed from its flow doc.
- `orange_money::{submit, query_status, parse_callback}`, returning
  `pay_token` + `notif_token` + `payment_url` together so a caller cannot
  hold a URL without the material to query it; `refund` is the port's
  `Unsupported`.
- Config gained `providers[].callback_url` / `currency`, `REQUIRED_RAIL_KEYS`
  ([ADR-0012](adr/0012-rail-configuration-requirements-in-config.md)) and
  `ProviderHost::to_provider_config`; a livemode-secret rule that had made
  livemode unbootable was fixed.
- `POST …/confirm` moves the intent: `processing` / `requires_action` /
  `409 charge_declined` / `502`, with the redirect's `next_action` built
  only from the committed charge row.
- `verify-no-mocks` became a `cargo metadata` reachability walk;
  `verify-status` became two-directional and comment-aware.

**Definition of done — met, and stated exactly.** The shared conformance
suite (`backends/tests/conformance/tests/adapter_conformance.rs`) passes
parameterised over both adapters with **no `#[ignore]`s left**: 26 tests, 26
passed, 0 skipped, measured 2026-09-03; `just verify-ignored` pins
`expected_ignored := "0"`.

**What remains before this phase can be called done against the world.**
- **Neither rail's real sandbox has ever been called.** Every assertion is
  against a stub whose mappings were written from `docs/flows/adapter-*.md`,
  so a document that is wrong about the rail would still pass.
- The 401 → re-mint → retry path is unproven on both rails.
- ~~No callback route exists:~~ **the callback route exists (Step 8, lane C)**
  but nothing verifies Orange's `notif_token`, and MTN's callbacks are
  unsigned — so a callback is a hint on both rails and always will be. No rail
  has ever called the route: every body it has parsed was transcribed from
  `docs/flows/adapter-*.md` by this repository's own tests.
- `mtn_momo::refund` is still a `NotImplemented` token, and `POST /v1/refunds`
  is unrouted.
- Orange's duplicate-submit idempotency is an assumption about the rail.

**Unblocks.** A meaningful Phase 3 `confirm` (delivered); Phase 4b/Phase 5
(there is now something to poll); Phase 6 (something to sign a webhook
about).

---

## Phase 4b — Push-rail recovery *(moved into Phase 5, and delivered there)*

**Done 2026-09-03 (Step 4), as part of Phase 5.** This was the half of the old
Phase 4 that Step 3 deliberately did not ship. Its scope below is now
implemented by `vpay_worker::recovery::recovery_step` and proven by
`backends/tests/integration/tests/worker_recovery.rs` — with one item
outstanding and named at the end of this section. The heading stays because
Phase 4a's Definition of Done never covered recovery, and deleting the split
would make that look retroactively fine.

**Scope.**
- The push-rail recovery table
  ([`docs/flows/crash-safety.md`](flows/crash-safety.md)): disambiguating a
  `submitting` charge via `provider_requests` (no row → resubmit; row with
  `status_code IS NULL` → poll, 3 consecutive `NotFound` over ≥60 s before
  treating the request as never received).
- Crash tests that kill the process at each of the three documented points
  and assert no double charge.

**What Step 4 delivered against that scope.** The `submitting` charges and
status-less `provider_requests` rows a lost submit leaves behind are now
*read*: no row → resubmit under the same reference; row with
`status_code IS NULL` → poll, and 3 consecutive `NotFound` over ≥60 s before
treating the request as never received; row with a status → advance the
bookkeeping. A redirect charge stuck in `submitting` is failed instead, keyed
on `Capabilities::flow`. Each has a test named in
[`docs/flows/crash-safety.md`](flows/crash-safety.md).

**What is still outstanding from this phase's scope:** ~~the crash tests do not
kill a process.~~ **Corrected 2026-09-04 (Step 8, lane D):** two of the three
kill points are now proven by a real `SIGKILL` to a real shipping process
(`worker_kill9.rs`); **kill point 1 still writes the state rather than causing
it**, which proves the recovery table but not that moment's behaviour under a
signal. That distinction is stated in
[`crash-safety.md`](flows/crash-safety.md) rather than smoothed over.

*The redirect-rail half of the old scope — "`ref_extra` must commit before
`redirect_to_url` is ever emitted" — **did** land in Phase 4a: the commit and
the `next_action` are one transaction and a re-read, proven by
`redirect_confirm_commits_the_rails_material_before_it_answers`.*

**Risks carried by this phase.** Unchanged: rail testing depends on WireMock
hosts, and on this machine that meant a rootless Docker daemon that could
not start containers. It now can, and the conformance and integration suites
run against real containers locally — but no CI run has exercised them on
this branch.

---

## Phase 5 — The worker

**Goal.** Charges in `processing` reach a terminal state without operator
intervention, and a crash at any of three documented points resolves
without double-charging.

**Status. 🟡 In progress — the loop landed 2026-09-03 (Step 4).**
`vpay-worker-bin` no longer logs a heartbeat saying the loop is not
implemented: it boots, reconciles configuration under the same advisory lock
`vpay-server` uses, reaps stranded job leases, seeds its its singleton jobs (two then; four since Step 5 — `sweep:expired`, `scan:live`, `fanout:events`, `scan:deliveries`) and
runs `vpay_worker::run_loop` — N claim/settle tasks over a `jobs` table
(migration `0021`, `FOR UPDATE SKIP LOCKED`, leases guarded on `locked_by`,
dead letters parked at `run_at = 'infinity'`), a 60-second gauge line, and a
bounded drain on SIGTERM. A confirmed payment reaches `succeeded` without
anyone touching it, and `just demo`'s sixth step is exactly that, end to end
through the containerised stack.

**Every settlement observed so far came from a WireMock rail.** No real rail
has been called, and the loop has never run anywhere but a developer machine
and CI.

**Scope, and what became of each item.**
- ✅ The job loop consuming charges needing action — `vpay_worker::run_loop`,
  driven by the same function the integration suite runs (there is no
  `#[cfg(test)]` variant and no injected clock).
- ✅ The poll ladder wired to that loop, indexed by `jobs.attempts - 1`.
- ✅ The reconciler: the settlement table (`vpay_core::settlement::settle`),
  the 24-hour `unresolved` escalation with its hourly re-poll and alert, and
  the one transaction that moves charge + intent + `events` row together.
  (Still not [RFC-0001](rfc/0001-settlement-and-payouts.md)'s settlement/payouts
  scope, which stays parked pending a licensing conversation.)
- 🟡 The three crash-test injection points from
  [`docs/flows/crash-safety.md`](flows/crash-safety.md) — all three are
  exercised, by *writing the state each one leaves* and running the real
  handlers against it. ~~**No process is killed.**~~ **Corrected 2026-09-04
  (Step 8, lane D): two of the three are now killed for real.**
  `backends/tests/integration/tests/worker_kill9.rs` `SIGKILL`s the shipping
  `vpay-worker-bin` mid-status-query and the shipping `vpay-server`
  mid-`requesttopay`, asserts the exit was *signalled with 9*, and asserts the
  charge settles exactly once with one submit in the rail's journal. **Kill
  point 1 is still written rather than caused** — there is no network call to
  interrupt before the reference is minted — and Orange is not exercised at
  all.
- ⛔ Absorbed from Phase 4b but **not** delivered: nothing else. Not in this
  phase's original scope and still unbuilt: `prompt_ttl_seconds` /
  `prompt_expired_at` / `payment_intent.processing` — named in
  [`docs/flows/reconciler.md`](flows/reconciler.md)'s Status. *(The callback
  route was on this list until 2026-09-04; Step 8's lane C built it.)*

**Definition of done — met in substance, with one honest gap.** The recovery
table resolves every injection point without a double charge, asserted by a
single distinct `provider_reference_id` across every `provider_requests` row
for the charge. ~~What is *not* met is the literal wording: these are not
kill-the-process tests~~ — **narrowed 2026-09-04 (Step 8): two of the three
now are.** The literal wording is unmet for **kill point 1 only**, and for the
reason above rather than for want of effort. Calling the remaining case a
kill-the-process test would still be the overstatement this repository exists
to avoid.

**Unblocks.** Reliable terminal states for Phase 6 to notify on.

**Risks carried by this phase.**
- ~~`--shutdown-grace-seconds` does nothing on `vpay-worker-bin`.~~ Closed:
  the flag now bounds a real drain — tasks stop claiming, in-flight jobs
  finish, and on timeout the remaining tasks are aborted, every lease this
  worker holds is handed back and the process exits non-zero
  (`a_drain_that_runs_out_of_grace_releases_every_lease_it_still_holds`).
- **Open: a rail answer that contradicts a settled charge is detected and
  logged, and neither call site is covered by a test.** The classifier is
  table-tested; the wiring is not. See `docs/status.md`.
- **Open: no real rail.** Everything above is proven against WireMock hosts,
  so what is proven is that vpay executes its own documents correctly.

---

## Phase 5d — Hosted and embedded checkout *(delivered 2026-09-04, Step 9)*

**Goal, in the maintainer's own words (2026-09-04):** *"We need a hosted page
for driving payments on the web: one in-iframe version, one fully hosted page.
We need that before prod."*

**Where this phase started.** Phase 5c had shipped `/v1/browser` and
`@vaam-apps/vpay-stripe-js` — a merchant could build its own payer page —
but vpay served no HTML at all, `tower-http` was built without `fs`, there was no
`frame-ancestors` or `X-Frame-Options` anywhere, no `success_url`/`cancel_url`
on any object, no per-charge return URL (so a redirect rail sent every payer to
a `POST`-only callback path that answers an empty `405`), and no i18n.
`@vaam-apps/vpay-stripe-js`'s README listed "Checkout (hosted or embedded)" under "Not
compatible, ever".

**What landed** ([plans/2026-09-04-step9-hosted-checkout.md](plans/2026-09-04-step9-hosted-checkout.md),
twelve lanes):

- **A `checkout.session` object** (`cs_…`, migration `0028`) a merchant creates
  from its server against an intent it already has. `ui_mode: hosted` answers a
  `url` to redirect the payer to; `ui_mode: embedded` answers a `client_secret`
  the merchant hands to `@vaam-apps/vpay-stripe-js`. Two payer credentials, not one: the
  session secret rides in the hosted URL's **fragment**, and a separate
  `return_token` rides in the return page's query string, because a fragment
  does not survive a rail's redirect.
- **The page** — `frontends/apps/checkout`, a Next 15 App Router app vpay
  serves, French and English, with the amount in integer minor units, a rail
  selector, an MSISDN form for MTN, the redirect for Orange, the outcome
  screen, and the forward to the merchant's URL with `{CHECKOUT_SESSION_ID}`
  substituted.
- **The return trip**, which closes Phase 5c's named gap
  ([flows/browser-checkout.md](flows/browser-checkout.md)'s D4): the provider
  port carries a per-charge `return_url`, Orange sends it, and vpay has a page
  to receive the payer.
- **`frame-ancestors` from configuration** — a per-merchant `checkout_origins`
  list, empty by default, resolved server-side by the page's middleware before
  any script runs, plus the page's own origin check against the same list.
- **`initEmbeddedCheckout`** in `@vaam-apps/vpay-stripe-js` and `checkout.sessions` in
  both merchant SDKs, in one PR per [ADR-0015](adr/0015-sdk-parity.md).
- **A fourth image** (`ghcr.io/vaam-apps/vpay-checkout`), a Helm workload
  behind `checkout.enabled`, and an eight-service demo stack.
- **`examples/shop`** — a Next.js merchant site with a seeded XAF catalogue,
  tRPC and ZenStack over Prisma, whose orders turn `paid` only from vpay's
  signed webhook and never from the return trip.

**What proves it.** `shop-hosted.cy.ts` and `shop-embedded.cy.ts` drive a real
browser through the shop to vpay's page and back on both rails, hosted and
embedded — and are proven not to pass with `vpay-worker` stopped. `just
test-e2e` from nothing is green in the `vpay-ci` VM: 11 tests over four specs,
0 failing, 0 skipped.

**Risks carried by this phase.**
- **No real rail, as everywhere else.** Every payment a browser has completed
  through vpay's page settled against a `wiremock/wiremock` host.
- **`frame-ancestors` is proven *sent*, never proven *enforced*.** Cypress
  strips `Content-Security-Policy` from every document it proxies, so the
  header is asserted with `cy.request` out of the runner's Node process. What a
  browser *was* seen enforcing is the page's own origin check refusing an
  unregistered framer.
- **A second unauthenticated surface with the same ingress requirement.**
  Phase 5c's D5 said rate limiting belongs at the ingress and nothing here
  enforces or checks it; Step 9 added the checkout app and three more
  `/v1/browser` reads under the same unmet requirement.
- **No pod has ever run the page**, and its path-prefix Ingress shape has been
  run by nobody.
- **The demo shop's `PrismaShopStore` has no automated coverage of its own** —
  verified by hand against a real Postgres, and exercised by the Cypress specs
  without being asserted on.
- **`checkout_not_configured` answers `500`, not `503`.** A truthful `503`
  needs an ADR-level change to [ADR-0011](adr/0011-error-modelling.md)'s
  category table, and that is left to the maintainer.

---

## Phase 6 — Webhooks

**~~Candidate, not decided (2026-09-02):~~ Decided 2026-09-03 — keep the
in-repo outbox; `cratestack-outbox` is not adopted.** The question this
paragraph reserved was whether to take `cratestack-outbox` (which implements
exactly this shape — `OutboxClient::persist_in_tx` inside the caller's
transaction, `drain` in insertion order, an axum drain/gc handler pair, and no
schema macro) or to write the ~200-line outbox by hand. **The answer is the
hand-written one**, in migrations `0021`/`0022`: the `jobs` table already
provides `FOR UPDATE SKIP LOCKED` claims, leases guarded on `locked_by`, a
reaper, dead-letter parking and a dedupe key, and `vpay_db::TxRepositories::insert_in_tx`
already writes the outbox row in the same transaction as the charge. Adopting
`cratestack-outbox` would duplicate all of that — two drain paths, two lease
mechanisms, two sets of metrics for one queue — and add a dependency in the
money path that has to be justified through `deny.toml`. What is given up is
real: vpay now owns this code forever, and the drain is one more thing to
maintain rather than to upgrade.

**This decision was taken by an agent under the user's standing delegation
("take all decisions yourself"), not by the maintainer who reserved it.** It is
reversible — the drain is `vpay_worker::webhooks::handle_fan_out`, one function
behind one job kind — and the maintainer may reopen it. ADR-0006's "no
in-process fake receiver" rule applied either way and still does.

**Goal.** Merchants receive signed webhook notifications for intent/charge
state changes, delivered at-least-once via a durable outbox.

**Status. 🟡 Delivered 2026-09-03 (Step 5), against a WireMock receiver.** Both
transactions of [`docs/flows/webhooks.md`](flows/webhooks.md)'s outbox run: the
settlement transaction writes the `events` row (Step 4), and the `fan_out_events`
singleton (`fanout:events`, seeded at every worker boot) turns it into one
`webhook_deliveries` row and one `deliver_webhook` job per configured endpoint,
in one transaction per event. `deliver_webhook` renders the event through the
same `vpay_api::model::EventObject` that `GET /v1/events` serves, signs those
exact bytes with HMAC-SHA256 over `"{t}.{body}"`, and POSTs them under
`Vpay-Signature` and `Stripe-Signature`. ~~`just demo`'s seventh step ends with
a signed `payment_intent.succeeded` read back out of a WireMock receiver's own
request journal and verified with the shipping SDK.~~ **Corrected 2026-09-04
(Step 8, lane A): the walkthrough is four steps, and step 4 reads a signed
webhook back out of the WireMock receiver's own request journal — one per
outcome, six in all — verifying each with the shipping SDK and asserting its
`type` against what that outcome must produce.**

**No merchant endpoint has ever been POSTed to.** Every delivery observed so
far went to a WireMock host on a compose network, on a developer machine and in
CI. That is why this phase is 🟡 and not ✅, and it is the same limit Phases 4
and 5 carry about rails.

> **Addendum, 2026-09-03 (evening) — two follow-ons landed on top of this
> phase, and neither changes anything above.** Step 5b (PR #20) proved a
> delivered webhook verifies through the **official `stripe` package**'s
> `webhooks.constructEvent`, against bytes read back out of the receiver's own
> request journal rather than bytes vpay believes it sent, and refuses both a
> body with one byte flipped and the right body under the wrong secret
> ([flows/stripe-sdk-compat.md](flows/stripe-sdk-compat.md)). Step 5c (PR #22)
> added the browser surface a payer's page polls while a delivery is in flight
> ([flows/browser-checkout.md](flows/browser-checkout.md)). Both are rows in
> the table at the top of this page rather than scope here. **The receiver is
> still a WireMock host, delivery is still unordered, and there is still no
> SSRF protection of any kind.** *(**Corrected 2026-09-04, Step 8 lane B:** the
> last clause is retired — `vpay_worker::ssrf` guards every delivery. The other
> two stand.)*

**Scope, and what became of each item.**
- ✅ An outbox row written in the same transaction as the state change it
  reports — `vpay_db::TxRepositories::insert_in_tx`, inside
  `vpay_db::Settlement::apply_succeeded`/`apply_failed` (Step 4).
- ✅ A signing scheme matching Stripe's — `t=…,v1=…` over `"{t}.{body}"`,
  emitted under both header names, verified by the two shipping SDKs against
  bytes this server actually sent.
- ✅ A delivery worker with a retry policy — `vpay_worker::delivery_delay`
  (10 s, 30 s, 2 m, 10 m, 1 h, 6 h, 24 h), then `state = 'exhausted'` with an
  `alert = true` log line.
- ✅ A backstop behind the queue — `scan:deliveries` (migration `0023`), every
  10 minutes over up to 500 rows, re-enqueueing a `deliver_webhook` job for any
  `pending` delivery nothing is driving.
- 🟡 Absorbed and only half-built: **the operator side.** There is no replay
  endpoint and no CLI, and the backstop cannot resurrect an `exhausted`
  delivery — re-sending one is a hand-written transaction, now written down in
  [`docs/runbooks/webhook-delivery-failures.md`](runbooks/webhook-delivery-failures.md).
- ⛔ Not in scope and still unbuilt: five of the seven documented event types
  (`payment_intent.created`, `.processing`, `.canceled` and the two refund
  types), and the `?type=` filter on `GET /v1/events`.

**Definition of done — met.** `worker_e2e.rs` proves a settlement commits its
`events` row in the same transaction as the charge and that the same loop drains
it; `backends/tests/integration/tests/webhooks.rs` proves signature verification
against a real HMAC — twice, once through the Rust SDK and once through the Node
SDK in a subprocess — driven against a WireMock receiver started by
`vpay_testkit`, never an in-process fake
([ADR-0006](adr/0006-no-mocks-in-main-processes.md)). The Node case **fails
rather than skips** when `node` is absent; CI sets `VPAY_REQUIRE_NODE=1`.

**Unblocks.** Nothing further downstream on the payments path — this is the
last functional phase before the MVP claim in `docs/status.md` can move.

**Risks carried by this phase.**
- ~~**No SSRF protection at all, and `validate_host` is not any.**~~
  **Closed 2026-09-04 (Step 8, lane B), and what replaces it is narrower.**
  Endpoint URLs are still checked at boot only, and that check is still a
  scheme rule plus four host substrings that never looks at an address — it
  never could, because an address is not a property of a configuration file.
  What changed is that the **address is now checked at delivery**:
  `vpay_worker::ssrf` resolves the host once, classifies every address the
  lookup answered with, refuses the delivery permanently if any of them is
  non-public, and pins the client to exactly those addresses with
  `reqwest::ClientBuilder::resolve_to_addrs` — which is the third answer the
  Step 5 plan's decision 4 did not consider, and it needs no custom connector.
  **Three residuals remain and are risks this phase still carries:** the guard
  is on webhook delivery only; a NAT64 receiver (`64:ff9b::/96`) is refused
  fail-closed; and the pin cost the shared connection pool, one handshake per
  delivery, unmeasured under load. And **no deployment has ever refused a real
  merchant's endpoint.**
- **Delivery is unordered.** Concurrent claims and the retry ladder can reorder
  two of one merchant's events; merchants must dedupe on `event.id` and reason
  from `event.created` and the object's own `status`, never from arrival order.
  Nothing in the design provides ordering and nothing is planned to.
- **A webhook secret is a forgery key.** Whoever holds one can sign a
  `payment_intent.succeeded` a merchant's handler will believe. Rotation exists
  (two secrets, two `v1=` values) but needs a deploy, and there is no
  revocation short of one.
- **The ladder's later rungs have never elapsed.** 1 h, 6 h and 24 h are
  asserted as values by a unit test; no deployment has waited them out.
- Unchanged: the standing no-mocks constraint above.

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

> **Addendum, 2026-09-03 (evening) — the environment blocker is lifted, and
> what is left of this phase is a cluster nobody has rather than a registry
> nobody could reach.** The 2026-09-02 correction above stands and is not
> deleted; the paragraph before this one describes the *original* authoring
> machine and is still true of it. What Step 6 (PR #21) and the CI runs since
> then changed:
>
> - **The compose stack runs in CI, on every pull request and every push.**
>   `ci`'s `e2e (compose)` job builds both images, brings the stack up against
>   WireMock rails, and runs **both Cypress specs** — `dashboard.cy.ts`'s three
>   tests and Step 5c's `checkout.cy.ts`, which drives a real browser through
>   confirm → processing → succeeded — plus the stripe-node conformance suite.
>   Green on `master` at run `33792230584`. The one spec this phase's Status
>   paragraph above calls "only the dashboard's scaffold notice" is no longer
>   the only spec.
> - **The Helm chart exists and CI checks it.** `deploy/helm/vpay`, fifteen
>   named template guards, `helm lint` / `helm template` / `kubeconform`
>   through `just helm-check`, run by CI's `deploy (helm chart)` job — the job
>   runs the recipe rather than a copy of it, so the two cannot drift. 20
>   resources validated, 0 invalid; every guard fires by name.
> - **The release pipeline has run, and signed what it built.** `release.yml`
>   builds three images across two native architectures, merges each into a
>   manifest list and signs it with keyless cosign. Four runs on `master`
>   since Step 6 landed — `33772512791`, `33784613048`, `33789060270`,
>   `33792230539` — nine jobs each, all green. **The `edge` images exist
>   because of those runs and for no other reason.** No `v*` tag has been
>   pushed, so the semver tag path is still unexercised; and the first push
>   creates each GHCR package **private**, so nobody outside CI can pull one
>   until a human flips the visibility by hand
>   ([runbooks/release.md](runbooks/release.md)).
> - **`just demo` runs seven steps** end to end against the containerised
>   stack, the seventh being a signed `payment_intent.succeeded` read back out
>   of a WireMock receiver's own request journal and verified with the
>   shipping SDK. *(**Corrected 2026-09-04, Step 8 lane A:** four steps now,
>   the fourth being six payments across both rails, each with its own signed
>   webhook.)*
> - **Both binaries have an observability listener** on a second port —
>   `/livez` and `/metrics`, twelve metric names with one seam each.
>
> **What is still not done is the whole of this phase's Definition of done.**
> **No cluster has ever run the chart** — not a real one, not kind — so
> nothing above is evidence about scheduling, admission, probe behaviour,
> `readOnlyRootFilesystem`, NetworkPolicy enforcement, PDB behaviour during a
> drain, or whether an ingress controller honours the `limit-rps` annotation
> CI greps for. **No Prometheus has ever scraped a vpay process**, so all five
> alert rules are unevaluated and every threshold is provisional rather than
> derived from traffic. **No runbook has been walked against a real fault** —
> eight are written, none has been followed against a deployment, because no
> deployment exists. No backup has ever been taken
> ([ADR-0013](adr/0013-database-backups-and-retention.md) is *proposed*). And
> under all of it, no real rail has ever been called. See
> [flows/deployment.md](flows/deployment.md) and `docs/status.md`'s
> Infrastructure rows for the per-artefact account.

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

**Third addendum, 2026-09-03 (evening).** Eight pull requests merged to
`master` in one day, and the table at the top of this page is rewritten against
them rather than patched around them: **#17** the rail adapters (Step 3),
**#18** the worker — job queue, poll ladder, recovery, settlement (Step 4),
**#19** signed webhook delivery with its retry ladder and the events API
(Step 5), **#20** compatibility with the official `stripe` package and the
`sdks/stripe-compat` conformance suite (Step 5b), **#21** the Helm chart, the
release pipeline, the observability listener, twelve metrics and four new
runbooks — eight in total (Step 6), **#22** the Stripe.js-compatible browser checkout — `/v1/browser`,
`@vaam-apps/vpay-stripe-js`, `examples/checkout-browser` (Step 5c), **#23** a frontend
dependency audit clearing 21 Dependabot alerts and adding `just audit-web` as a
gate, and **#24** a raised Cypress verify budget in the CI e2e job. **Step 7,
the cleanup rework, is on a branch and unmerged**; it moves no capability.

**The caveats none of those merges touched**, and which this page must not be
read as having retired: no HTTP call to a **real** rail has ever been made —
every payment in this repository's history settled against a
`wiremock/wiremock` host answering the way these documents say a rail answers;
**no merchant endpoint has ever been POSTed to** and ~~there is no SSRF
protection on webhook destinations~~ *(retired 2026-09-04 by Step 8's egress
guard — see the fourth addendum)*; **no cluster has run the Helm chart and no
Prometheus has scraped a vpay process**; the GHCR packages those release runs
created are **private** — nothing in this repository can publish them, and
making one pullable is a one-time change a human makes in the package's own
settings ([runbooks/release.md](runbooks/release.md)); there is still no
`/dash/v1` login; and **no runbook has been followed against a real fault**.
[`docs/status.md`](status.md) records each of those per feature, and it wins.

**Fourth addendum, 2026-09-04 (Step 8, the production gate).** One branch,
`claude/step8-production-gate`, six lanes plus a seventh the step's own reviews
produced, and the point of the step was to close every gap that does **not**
need a real rail credential:

- **Lane B — the runtime egress guard.** `vpay_worker::ssrf` on every webhook
  delivery: resolve once, classify every answered address in both families
  (mapped and compatible spellings included), refuse the delivery permanently
  if any is non-public, and pin the client to what was classified. This closed
  the repository's only ⛔ on a shipping path. Three residuals remain, named in
  Phase 6's risks.
- **Lane C — the rail callback route.** `POST /provider/{code}/callback`, plus
  migration `0027`'s index behind its unauthenticated lookup. It writes no
  charge or intent state: it pulls the charge's existing poll job forward.
  Orange's `notif_token` is still not compared against the stored one, and the
  route discards `CallbackRef::ref_extra` rather than trusting it.
- **Lane D — a real `SIGKILL`.** The shipping `vpay-worker-bin` killed
  mid-status-query and the shipping `vpay-server` killed mid-`requesttopay`,
  against real Postgres and WireMock. Kill point 1 is still written rather than
  caused, and Orange is not exercised.
- **Lane G — the confirm/worker race**, which was *not* in the plan. Lane A's
  demo produced a `500 api_error` on confirm in four of six runs: the worker
  claimed the poll job the confirm had just committed and applied the
  crash-recovery table to a charge whose process had not crashed. The fix is a
  minimum charge age in `recovery_step`. **A demo found a real defect in a
  payment path, which is the strongest argument this step makes for having a
  demo at all.**
- **Lane A — the demo.** Six payments, both rails, three outcomes each, every
  outcome steered at the stub by a field a merchant controls; split recipes;
  two stacks on one machine at the Compose layer; and
  [runbooks/demo.md](runbooks/demo.md) with real pasted output. `.e2e/` is
  still shared between stacks and that is a named gap.
- **Lane F — SDK parity**, added at the user's request outside the original
  plan: [ADR-0015](adr/0015-sdk-parity.md), [docs/sdks/parity.md](sdks/parity.md)
  and `cargo xtask verify-sdk-parity` in `just verify`. 267 proving tests named,
  24 dated gaps, **none of them closed by this step**.

- **Lane H — the correctness review's four confirmed findings**, 2026-09-04,
  after the six lanes above had merged. (1) The recovery window compared
  Postgres' `charges.created_at` against the **worker host's** clock, so a
  worker sixty seconds fast made lane G's guard a silent no-op on exactly the
  deployment whose fleet clocks had drifted; the age now comes from
  `Charges::get_by_id_as_of`, which selects `now()` beside the row, and
  `recovery_step` takes durations rather than instants so there is no parameter
  left for a caller to read off the wrong clock. (2) `RecoveryAction::Wait`
  rescheduled at `poll_delay(0)`, and every reschedule spends a rung, so a
  genuinely crashed charge burned six of them waiting the window out; `Wait`
  now carries `window - age`, clamped, and reschedules once, so the first real
  recovery rung is `poll_delay(1)` — twenty seconds. (3) The callback route's
  pull-forward matched any unleased future job, so an anonymous caller drove
  rail traffic at their own rate and the module doc said the opposite; it now
  refuses a job already due within `PULL_FORWARD_FLOOR` — ten seconds, the
  ladder's own fastest rung — and the cost is stated rather than implied: a
  callback arriving while the charge sits on that first rung no longer settles
  it early. (4) The egress classifier let `192.88.99.0/24`, `2001:1::/32`,
  `2001:2::/48` and `2001:20::/28` through as ordinary public addresses; all
  four are refused now. [plans/step8-notes/lane-h.md](plans/step8-notes/lane-h.md)
  is the record, including the two findings it deliberately did not fix.

**What Step 8 did not do, stated so it is a decision and not an omission.** No
real rail was called and the "do not deploy" banner is untouched. The dashboard
and `/dash/v1` are unbuilt, and the demo says why: there is no data source to
show. The Orange redirect return trip is still missing, so a redirect-rail
checkout must not ship on `@vaam-apps/vpay-stripe-js`. `mtn_momo::refund` is still the
one `NotImplemented` token. `just demo` has **not** been run on the merged gate
branch. And `charges.provider_reference_id` is still not `UNIQUE` — lane C
recommends it and deliberately left the decision to the maintainer.
**Lane H adds five more, each named rather than rounded off:** an SSRF-refused
webhook delivery is exhausted on its first attempt and there is **no replay
path** (finding 5); the callback route's two `202`s are distinguishable in
*time*, because the known-reference path runs a transaction the unknown one
does not (finding 7); **no rate limit** was added to that route, per charge or
per source, and the pull-forward floor is not one; `scan_live_charges` still
computes its ten-minute cutoff from the worker host's clock and compares it
against a column Postgres wrote — the same defect class as the recovery
window's, in its mildest direction; and **`2001::/23` as a whole is still
deliverable**, because refusing IANA's whole protocol-assignments block is a
wider call than the review asked for and is left to the maintainer.

**Issue #11 is answered item by item in the PR and is not auto-closed.** Two of
its seven items are incomplete (the `.e2e/` half of the concurrency item, and
the walkthrough's own history), and a closed issue that is not fixed is worse
than an open one.

**Fifth addendum, 2026-09-04 (Step 9, hosted checkout).** One branch,
`claude/step9-hosted-checkout`, twelve lanes, and it delivered the thing the
maintainer asked for in the sentence at the head of Phase 5d. What that phase
is and what it carries are written out there; this addendum records the three
things about the *step* that a phase description would flatten.

- **A defect the demo shop found, which three lanes had walked past.** No
  merchant server that reaches vpay by an internal URL could authenticate at
  all. Both SDKs signed the client assertion's `aud` with the token endpoint
  they were about to POST to and derived both from `baseUrl`, while vpay's OP
  derives its issuer solely from `deployment.public_base_url` — so
  `http://vpay-server:8080/v1/oauth/token` was compared against
  `http://localhost:8080/v1/oauth/token` and refused with a bare
  `invalid_client` / `InvalidAudience`, with the signature, the `client_id`,
  the `kid` and the lifetime all correct. It survived because lane 7 never
  spoke to a running vpay, lane 4 brought the shop up but never clicked through
  it, and every other consumer runs *on the host*, where the public issuer
  happens to be right. Lane 6 found it by putting a merchant's own server
  inside the compose network; lane 5b fixed it with a third setting
  (`assertionAudience` / `ClientBuilder::assertion_audience`), proven by the
  real pinned `authkestra_op` verifier refusing and then accepting the same
  client. **ADR-0010 does not change** — the SDKs had conflated two of its
  three strings.
- **`just test-e2e` could not have passed since Step 5c**, and nobody had
  noticed because CI's `e2e` job was the only place `checkout.cy.ts` ever ran.
  The recipe brought up a stack with no merchant anybody holds a key for. It is
  fixed, and this is the first item on `docs/status.md`'s MVP list to move to
  met since item 1.
- **Two reviews and a second round.** Correctness/money-and-secrets and
  conventions/blast-radius on the merged gate, then a second round after the
  first remediation, and every remediation reviewed. Lanes 1b, 3b, 5b and r2
  are what came out of them, and the reviews found things the lanes' own tests
  could not: a return-URL lookup that answered `None` for every intent, a
  browser read that never stopped issuing a credential, a page that refused to
  paint when a merchant had no display name, a staleness check that was a
  presence proxy, and five demo publications on `0.0.0.0`.

**What Step 9 did not do, stated so it is a decision and not an omission.** No
real rail was called and the "do not deploy" banner is untouched: a browser now
walks an entire checkout, and every rail in that walk is a `wiremock/wiremock`
container. **No browser has been observed enforcing vpay's `frame-ancestors`** —
Cypress strips the header, so it is asserted as sent, and the refusal a browser
was seen performing is the checkout app's own origin check. No pod has run the
page. There is still no rate limiting in front of either unauthenticated
surface. The demo shop's `PrismaShopStore` has no automated coverage of its
own. `checkout_not_configured` still answers `500` where `503` would be
truthful, and moving it is an ADR-level change **left to the maintainer** —
along with whether `checkout.public_base_url` should be a separate host or a
path under the API host in production, and whether a session may create its
PaymentIntent inline in a later step. The dashboard is untouched and `/dash/v1`
is still unbuilt.

