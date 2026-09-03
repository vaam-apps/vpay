<!-- Implementation design for one step of the production-readiness plan. A point-in-time working document: once the step lands, docs/status.md and the flow docs are the record and this file is history. -->

# Step 7 — cleanup rework: implementation-ready design

User instruction (verbatim, 2026-09-03): "At the end, do a cleanup rework: clean error mapping using
hierarchy, thiserror and anyhow; keep the trait-based approach for adapters, and add clean error
surfaces; keep and generalize the serde rename_all => snake_case; start enforcing code best practices
(SOLID, DRY mainly); introduce repository pattern + interface pattern and hide repositories' real
implementation; doctests to ensure no lie in the docs, and externalized docs to .md files, and finally
reduce comments in-files. Goal is to have good and readable code with associated clean documentations."

Decisions taken by the orchestrator under the user's delegation (do not reopen): (1)
`ProviderError::{Transport, Malformed}` carry a real `#[source]` (the correct model; ~200-line diff
accepted); (2) `&dyn Repositories` in both vpay-api and vpay-worker; (3) transactions via a
`UnitOfWork` closure — verify the confirm-path `drop(tx)` recovery site compiles under it first;
(4) comment budget is a warn-only `verify-docs` report, not a gate; (5) `cargo doc` without
`--document-private-items`; (6) the worker keeps linking vpay-api for `ResourceConfig`; (7) Step 7 runs
AFTER Step 6, as the user asked ("at the end"); Phase A (repositories) is sequential and lands first,
then four parallel lanes; wire behaviour must not change (conformance, integration and SDK suites are
the guard — if any of them needs editing, escalate).

## What the measurements say before the design

Measured on `claude/step4-worker` (all counts from grep/wc, production code = lines before the first `#[cfg(test)]`).

**Six things that are not what the ticket implies.**

**S1 — there is exactly one compiled doctest in the workspace, and CI cannot run it.** 26 fence lines = 13 blocks; 5 are ```` ```text ````, one ```` ```no_run ```` (`/home/selast/dev/vpay/.claude/worktrees/vpay-production-readiness-56b122/sdks/rust/src/lib.rs:20`), one ```` ```ignore ```` (`…/backends/crates/vpay-api/src/resource_auth.rs:732`). The only real doctest is `…/backends/crates/vpay-core/src/money.rs:142-151`. `just test-rust` is `cargo nextest run --workspace` (justfile:52) and `.github/workflows/ci.yml:81` is the same — **nextest does not run doctests**, so that one has never run in CI. "Doctests so the docs cannot lie" is a from-zero build, and `cargo test --doc` is a *second* test runner in `just ci`, not a flag on the existing one.

**S2 — "generalize `rename_all = "snake_case"`" is wire-breaking if applied literally.** 12 sites have it, 52 serde derives lack it. But `…/backends/crates/vpay-adapter-mtn-momo/src/wire.rs` models *MTN's* camelCase (`externalId`, `partyIdType`, `partyId`, `financialTransactionId` — per-field `#[serde(rename)]` at :39, :48, :50, :93). Adding `rename_all` to `RequestToPay`/`StatusResponse`/`CallbackBody` is at best a no-op masked by field renames and at worst breaks two adapters and 26 conformance cases. 16 of the 52 are rail-wire or foreign-protocol (`authkestra`, OAuth) types. The convention applies to **vpay's own wire and config**, not to the adapters' `wire.rs`.

**S3 — the repository-trait ask collides with a documented, deliberate rule.** `…/backends/crates/vpay-worker/Cargo.toml:22-27`: "**No `sqlx` under `[dependencies]`, and that is a rule and not an oversight.**" Yet 7 sites call `pool.begin()` and pass `&mut PgConnection` into `*_in_tx`: `…/vpay-api/src/v1/payment_intents.rs:812, 913, 1048`; `…/vpay-worker/src/handlers.rs:400, 473, 693`; `…/vpay-worker/src/run_loop.rs:457`. `…/handlers.rs:942` records the constraint verbatim: a helper "would have to name `sqlx::PgConnection` … and this crate deliberately does not depend on `sqlx`". Any trait that returns `sqlx::Transaction` re-opens that rule.

**S4 — three of the ticket's cleanup axes are already clean.** `anyhow` is already binaries-only: `verify-errors` enforces it (`.xtask/src/main.rs:672-681`) and status.md:39 cites `12 error type(s), all classified; anyhow confined to binaries`; the only library `anyhow` is `vpay-db`'s `[dev-dependencies]` (`Cargo.toml:71`, under the `[dev-dependencies]` header at :64). Only **4 `#[allow]`/`#[expect]` in production code** (`run_loop.rs:510`, `handlers.rs:291`, `op/clients.rs:88`, `:314` — the last two are `#[allow(deprecated)]`). And both composites **already delegate all five `Classify` methods** (`…/vpay-api/src/error.rs:522-703`, `…/vpay-worker/src/error.rs:119-205`). Item 1's "smells to remove" list is much shorter than the ticket assumes.

**S5 — the real error debt is source-chain loss in the adapters, not the composites.** `ProviderError::Transport(String)` and `Malformed(String)` (`…/vpay-provider/src/lib.rs:102-129`) flatten every `reqwest::Error`/`HttpBodyError` through `format!` at ~40 sites. `…/vpay-adapter-orange-money/src/lib.rs:666-680` hand-walks `Error::source()` into a `String` *because* the variant cannot hold a source — that function's doc comment says "with its source chain" while producing a `String`.

**S6 — Steps 4 and 5 are unmerged and own the crates this step refactors.** `claude/step5-webhooks` vs `claude/step4-worker` is +3097 lines across `vpay-db` (+1300), `vpay-worker` (+1400), `vpay-api/src/model.rs` (+229). Cleanup on `vpay-db`/`vpay-worker` before both merge is a guaranteed conflict.

**Volume baseline:** 12,349 doc-comment lines / 15,216 production code lines = **81.2%**. Worst: `op/mod.rs` 267%, `resource_auth.rs` 184%, `db/jobs.rs` 177%, `db/payment_intents.rs` 152%. Longest blocks: `vpay-api/src/lib.rs:560` (122 lines), `jwks_cache.rs:1` (117), `resource_auth.rs:1` (98), `op/token.rs:1` (87), `form.rs:1` (85), `provider/http.rs:1` (83). 10 production functions ≥80 lines; `vpay-server/src/main.rs:167 fn run` (303), `payment_intents.rs:519 fn confirm_once` (244), `vpay-worker-bin/src/main.rs:169 fn run` (213).

---

## 1. Error hierarchy

Target shape is **unchanged from ADR-0011** — leaves, `ApiError`/`JobError`, `vpay_core::error` façade. Concrete work:

- **`ProviderError::Transport`/`Malformed` gain a source.** `Transport { context: String, #[source] source: Option<Box<dyn Error + Send + Sync>> }` is the honest shape; the cheaper variant is keeping `String` and adding `ProviderError::transport(what, &err)` as the *one* constructor that walks the chain (generalising `orange-money/src/lib.rs:666`) so MTN stops losing it. **Default: the constructor, not the variant change** — see D1.
- **Delete `ApiError::Internal(String)`? No.** It is the documented "invariant this layer guarantees was violated" arm (`error.rs:319-326`) with one production constructor (`internal_serialization`, :373). Leave it; add a doctest pinning that its `public_message()` is the generic sentence.
- **Not-smells, do not "fix":** `Rejected{code: FailureCode, message}` — the ticket's `raw` does not exist and `failure_raw` is written from the adapter mapping, not the error; `Unsupported`'s severity override; `DbError::UniqueViolation → Conflict`. Each has a test and a documented reason in `docs/flows/errors.md`.
- **`verify-errors` extension (worth it, cheap):** for every `#[from] Leaf` variant on a type that has an `impl Classify`, require a `Self::Variant(e) => e.<method>()` arm in **all five** methods. Today's code passes; the check makes a future `#[from]` that silently falls into a `_ =>` wildcard fail the build. Implement beside `has_classify_impl` (`.xtask/src/main.rs:1066`), text-scanning as the rest of the file does.

## 2. Adapter error surface

Keep `ProviderAdapter` and `#[async_trait]` unchanged (ADR-0002; `…/vpay-provider/src/lib.rs:334`). Additions:

- `impl ProviderError { pub fn retryable(&self) -> bool }` — **only if a duplicate exists**. Grepped: there is none. The worker reads `Classify::retry` exclusively. **Do not add it**; adding a second retry oracle is exactly what ADR-0011 forbids. Say so in the module doc.
- **One `CachedToken`.** `…/vpay-adapter-mtn-momo/src/token.rs:213` and `…/vpay-adapter-orange-money/src/token.rs:128` are the same three fields (`value`, `expires_at: Instant`, `fingerprint: [u8;32]`), the same redacting `Debug`, and the same `usable(fingerprint) -> Option<&str>`. They differ only in margin handling (MTN subtracts in `expiry()`, Orange in `new()`) and in `usable` taking `now` vs reading `Instant::now()`. Move to `vpay_provider::token::CachedToken` with `new(value, minted_at, lifetime, margin, fingerprint)` and `usable(now, fingerprint)` — clock injected, which is what makes MTN's existing tests keep working. Also move `fingerprint()` (`orange/token.rs:113`, length-prefixed SHA-256) — MTN's at `:164` is the same idea. **~150 lines removed, 2 crates touched.**
- **One body reader.** `mtn/src/lib.rs:252 read_body` and `orange/src/lib.rs:311 bounded` are the same wrapper over `vpay_provider::http::bounded_body`, differing only in the `"mtn_momo: "`/`"orange_money: "` prefix. Replace with `vpay_provider::http::read_rail_body(response, rail: &'static str)`.
- **Deliberately per-rail, document as such:** `mapping.rs` in both crates (the `FailureCode` tables), `wire.rs` (the rail's own casing — S2), token URL derivation, `Capabilities`.

## 3. serde

Convention: **`#[serde(rename_all = "snake_case")]` on every type that models vpay's own wire or config.** Scope in, missing today: `vpay-api/src/model.rs:77, 111, 128, 194`; `vpay-api/src/v1/payment_intents.rs:115, 330, 454`; `vpay-api/src/form.rs:786, 797`; `vpay-config/src/config.rs:88, 336, 357`; `vpay-config/src/lib.rs:28, 39`; `vpay-config/src/oauth.rs:112, 242`; `vpay-core/src/money.rs:61`; `vpay-provider/src/lib.rs:27`; `vpay-worker/src/jobs.rs:93, 163`; `sdks/rust/src/model.rs:81, 109, 119, 177, 205, 214, 272, 283, 292`. Each is a no-op today (fields are already snake_case) — that is the point: the attribute makes it stay one when someone adds `payTo`.

Scope **out**, and comment saying why: both `wire.rs` files, both `token.rs` `TokenResponse`s, `resource_auth.rs:222 RawClaims` (JWT claim names), `sdks/rust/src/client.rs:170-194` (RFC 6749 + Stripe envelope), `vpay-core/src/money.rs:16` (`UPPERCASE`, currency codes).

Enforcement: **xtask scan, not a macro.** New `cargo xtask verify-serde` reusing `strip_cfg_test_items`/`searchable` (`.xtask/src/main.rs:819, 718`): every `#[derive(…Serialize|Deserialize…)]` in `backends/crates` outside an allowlisted `wire.rs`/`token.rs`/foreign-protocol file must carry `rename_all`. A derive helper macro would be a new proc-macro crate in the money path for a naming convention — not worth it.

## 4. Repository + interface pattern

`vpay-db` today: 49 `pub` fns, 10 `pub mod`s, 35 call sites in `vpay-api`, 64 in `vpay-worker`, 10 in `vpay-server`, 5 in `vpay-worker-bin`.

**Traits, `dyn`, in `vpay-db`, one per aggregate**, `#[async_trait]`: `PaymentIntents`, `Charges`, `Idempotency`, `ProviderRequests`, `Jobs`, `Events`, `WebhookDeliveries`, `Settlement`, `SigningKeys`, `DisabledClients`, `ConfigReconcile`. One umbrella `trait Repositories: PaymentIntents + Charges + …` and `pub struct PgRepositories(PgPool)` implementing all of them; modules become `pub(crate)`. `PgPool` stops being re-exported from `lib.rs:103`.

**`dyn`, not generics.** `ProviderAdapter` is already `Box<dyn` + `#[async_trait]` (`vpay-provider/src/lib.rs:319-335` documents the boxed-future cost as accepted). Generics would put `<R: Repositories>` on every axum handler, every `AppState`, and every `JobHandler` signature — a large diff for one heap allocation per query on a path that already awaits Postgres.

**Transactions: `UnitOfWork` closure, no `Box<dyn Tx>`.**

```rust
#[async_trait]
pub trait UnitOfWork: Send + Sync {
    async fn transaction<'a>(&'a self, f: TxFn<'a>) -> Result<(), DbError>;
}
// where the closure receives &mut dyn TxRepositories — the subset with *_in_tx today
```

This is the only shape that keeps `vpay-worker`'s no-`sqlx` rule (S3) *and* keeps the commit point at the caller, which `payment_intents.rs:1037-1042` says is load-bearing (the charge and its job must commit before the network call). `begin() -> Box<dyn Tx>` leaks lifetime management to callers and makes "forgot to commit" expressible. ADR-0006 holds unchanged: `PgRepositories` is the only implementation, tests construct it against real Postgres, no fake is written.

**Call-site churn:** `vpay-api` 35 (of which 3 transactional), `vpay-worker` 64 (4 transactional), `vpay-server` 10, `vpay-worker-bin` 5. `AppState`/`RouterDeps` (`vpay-api/src/lib.rs:682`) and the worker handler signature (`&PgPool` → `&dyn Repositories`) change once each. `SqlClientAssertionStore` (`vpay-db/src/client_assertion.rs:121`) already implements a foreign trait over the pool and is the precedent.

## 5. SOLID/DRY, ranked by payoff

1. **Binary boot duplication** — `exit_code_for` (`vpay-server/src/main.rs:152`, `vpay-worker-bin/src/main.rs:87`), `install_crypto_provider` (:651 / :423), `init_tracing` (:661 / :433). ~120 duplicated lines. **Put them in `vpay_config::boot`, not a new `vpay-app` crate** — they depend only on `LogFormat`/`CommonArgs`, which `vpay-config` already owns (`cli.rs:125`). `adapters()` (`vpay-worker-bin/src/main.rs:150`) stays **duplicated and per-binary**: ADR-0006 makes the set of linked rails a property of the binary, and `verify-no-mocks` walks the graph from each binary root (`.xtask/src/main.rs:95`).
2. **`fn run` split** — 303 and 213 lines. Split into named `async fn` steps mirroring the boot doc (`load_config`, `open_pool`, `migrate`, `ensure_signing_key`, `build_router`, `serve`), each `.context()`ed as today.
3. **`PostRequest` + idempotency plumbing** out of `payment_intents.rs` (`:1476-1740`, ~264 lines) into `vpay-api/src/v1/post_request.rs`. `payment_intents.rs` is 2338 lines / 941 production code lines — the largest file in the workspace.
4. **`confirm_once`** (244 lines, `:519`) → validate / insert-charge / submit / persist, each already a named helper below it.
5. **`form.rs`** (`parse_form` + `VpayForm`/`VpayQuery`) → `form/parse.rs` + `form/extract.rs`; the 85-line module header moves to `docs/design/form-decoder.md`.
6. **`ResourceConfig` vs `RailConfig` vs `Config`** — leave the split (`v1/mod.rs:219, 297`; Step 4 D1 explicitly decided the worker links `vpay-api` for it, and Step 5 adds `endpoints_by_merchant_id` to it). Document the three responsibilities in `docs/design/configuration-projection.md`; do **not** move it mid-flight.
7. **Router assembly** — already one `pub fn router` (`lib.rs:682`). No work beyond moving its 122-line preamble.
8. **`#[allow]` removal** — 4 sites, 2 legitimately `deprecated`. Effectively no work (S4).

## 6. Docs externalisation + doctests

**Layout:** `docs/design/<topic>.md` for: `request-ids.md`, `jwks-cache.md`, `idempotency-lifecycle.md`, `confirm-ordering.md`, `rail-token-cache.md`, `verify-scanners.md`, `boot-sequence.md`, `worker-loop.md`, `form-decoder.md`, `error-envelope.md`, `configuration-projection.md`. Each is the destination for one of the ≥59-line blocks listed in S6. Add `docs/design/README.md` explaining the tier (ADR = decision, flow = process, design = "why this code looks like this").

**Rule for what stays in code:** one paragraph of what/why + a `docs/design/…` link; `# Errors` and `# Panics` sections stay (they are rustdoc contract); every "measured", "review finding 2026-…", and history paragraph moves out. **Target: 81.2% → ≤40%**, i.e. ~6,000 doc lines removed. No file over 100%.

**Doctest candidates** (all pure, no I/O, all currently prose-only): `vpay_core::ids::{is_well_formed, payment_intent_id, charge_id, refund_id, event_id}` (`ids.rs:99-167`); `Money::{new, checked_add, checked_sub, to_provider_string, to_provider_minor}` and `Currency::{from_code, exponent}` (`money.rs:27-170`); `state::{next_status, status_after_confirm, is_live, is_terminal, from_wire, as_wire_str}` (`state.rs:50-227`); `settlement::settle` (`settlement.rs:108`); `FailureCode::{payer_actionable, merchant_actionable, as_str}` (`failure.rs:32-49`); `vpay_worker::poll_delay` (`lib.rs:37`); `recovery_step` (`recovery.rs:151`); Step 5's `delivery_delay` and `signature_header`; `vpay_api::form::parse_form` (`form.rs:174`); `Category`'s five derivations. **Target: 1 → ≥35 doctests.**

**Honesty rule:** no `no_run`, no `ignore`, no `#` hidden setup lines that hide the assertion. Convert `resource_auth.rs:732`'s ```` ```ignore ```` to a real one or to ```` ```text ````. Any exception carries `// doctest-exempt: <reason>` and `verify-docs` counts them.

**`just ci`:** add `test-doc: cargo test --doc --workspace` between `test-rust` and `verify-ignored`; mirror in `.github/workflows/ci.yml` after line 81. **`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`: yes, without `--document-private-items`** — broken intra-doc links are the exact failure mode "docs must not lie" targets, and private-item docs would add hundreds of warnings for internal helpers on day one. Recipe `docs-check` at justfile:456 is currently a stub printing "not implemented yet" — this is its implementation, plus the link check it already promises.

**Comment budget: warn-only, no gate.** `cargo xtask verify-docs` reports per-file doc/code ratio and the top 10 offenders, exits 0 always, and is *not* in `verify`. A hard ratio gate rewards deleting the `# Errors` sections that ADR-0011 depends on.

## 7. Ordering and work split

**Sequencing is not negotiable at the top:** Steps 4 and 5 merge to `master` first (S6). Then, one lane at a time on the shared crates, full suite green between each:

**Phase A (blocking, sequential):** repositories in `vpay-db` (traits + `PgRepositories` + `UnitOfWork`), then the 114 call sites in one commit per consuming crate. Errors and serde must come *after*, because a repository refactor that lands on top of a changed `ProviderError` shape produces conflicts nobody can review.

**Phase B (four parallel lanes, disjoint crates, after A is on master):**
- **Lane 1 — `vpay-core` + `vpay-ledger`:** doctests (~18), doc externalisation, `rename_all` on `Money`.
- **Lane 2 — `vpay-provider` + both adapters:** `CachedToken`, `read_rail_body`, `ProviderError::transport` constructor, `wire.rs` serde exemption comments, `docs/design/rail-token-cache.md`.
- **Lane 3 — `vpay-api` + `vpay-config`:** `PostRequest` extraction, `confirm_once` split, `form/` module split, `rename_all`, `vpay_config::boot`, four `docs/design/` pages.
- **Lane 4 — `.xtask` + `justfile` + CI + `docs/`:** `verify-serde`, `verify-docs`, the `verify-errors` delegation check, `docs-check` implementation, `test-doc` in `just ci` and CI, `docs/design/README.md`.

`vpay-worker` and both binaries are touched by Phase A and by Lane 3's `boot` move only — assign them to Lane 3 to keep lanes disjoint.

**Review lenses (two per lane, parallel, distinct):** (a) *behaviour preservation* — re-run the full suite plus `vpay-tests-conformance` and `just demo`; the decisive test is "revert one refactored call site and confirm a named test FAILS"; (b) *docs honesty* — every claim moved to `docs/design/` is either doctested or marked unproven; no doctest uses `no_run`/`ignore`; no `docs/status.md` row upgraded.

**Definition of done:** `just ci` green including the new `test-doc`; doc-line ratio reported before/after per crate; doctest count reported (1 → N); `just verify-ignored` unchanged at `0 ignored`; total test count **≥** the pre-step figure; zero diff in `backends/tests/conformance`, `backends/tests/integration` assertions and `sdks/*` expectations.

## 8. Docs/status

New rows: **Repository traits (`vpay_db::Repositories`, `UnitOfWork`)** — ✅ only with the note that `PgRepositories` is the sole implementation and ADR-0006 is untouched; **Doctests** — with the measured `cargo test --doc --workspace` count; **`verify-serde` / `verify-docs`** under the self-verification section; **`docs/design/`** in the documentation row. Update the header paragraph (status.md:6-29) to say `just verify` is now five scanners, and add the doctest count to the measured-tests paragraph (status.md:52-63). Do **not** change any 🟡/⛔ — nothing here builds a feature.

**This step must not change public wire behaviour.** The guards that would catch a violation: `backends/tests/conformance` (26 cases over both rails), `backends/tests/integration` (52 cases incl. `confirm_rails`, `payment_intents`), `sdks/rust/tests/{resources,errors,op_conformance,token_exchange}.rs`, and `sdks/nodejs`. If any of those needs editing, the change is out of scope for a cleanup step — escalate rather than edit the test.

---

## Decisions needed from a human

1. **`ProviderError::Transport`: keep `String` and add one chain-walking constructor, or change the variant to carry `#[source]`?** *Default: keep `String`, add `ProviderError::transport(rail, what, &err)`.* Gained: ~40 call sites become one line each, MTN stops losing the chain that Orange preserves, and the ~30 `matches!(err, ProviderError::Transport(_))` assertions across both adapters and the conformance suite keep compiling. Lost: `Error::source()` on a `ProviderError` still returns `None`, so a future `tracing` integration cannot walk it structurally — it only has the flattened text. The `#[source]` version is the correct model and roughly a 200-line diff across the two adapters plus every test that pattern-matches the variant.

2. **Repository traits: `&dyn Repositories` everywhere, or keep `&PgPool` in `vpay-worker` and abstract only `vpay-api`?** *Default: `&dyn` in both.* Gained: one seam, `sqlx` disappears from `vpay-api`'s `[dependencies]` (`vpay-api/Cargo.toml:127`) and from the worker's surface entirely, and the no-`sqlx` rule at `vpay-worker/Cargo.toml:22` becomes structural rather than a comment. Lost: 114 call sites change in a step whose whole promise is zero behaviour change, and `#[async_trait]` boxes a future per query. Abstracting only `vpay-api` halves the diff but leaves two spellings of "how you reach Postgres" in the workspace, which is the thing this step exists to remove.

3. **Transaction API: `UnitOfWork` closure, or `begin() -> Box<dyn Tx>`?** *Default: the closure.* Gained: "forgot to commit" is not expressible, the caller still chooses the commit point (`payment_intents.rs:1037` requires this), and no `sqlx` type escapes `vpay-db`. Lost: the closure cannot borrow across an `.await` boundary as freely as a held handle, so the confirm path's `drop(tx); …get_for_intent(pool, …)` recovery (`payment_intents.rs:1066-1071`) has to be restructured — verify that specific site compiles under the closure form **before** committing to it, because it is the one place a lost race is handled by abandoning a transaction mid-function.

4. **Comment budget: warn-only report, or a hard gate in `just verify`?** *Default: warn-only.* Gained: the user asked for readable, not policed; a gate would put pressure on `# Errors` and `# Panics` sections that ADR-0011 and rustdoc both depend on. Lost: the 81.2% ratio can silently climb back — nothing stops the next step from adding a 120-line module header. If a gate is wanted, gate on *per-file ratio for new files only* rather than a workspace number.

5. **`cargo doc` with `--document-private-items`?** *Default: no.* Gained: `RUSTDOCFLAGS="-D warnings"` goes green on the first try and catches every broken intra-doc link, which is the honest-docs failure mode. Lost: private helpers' doc comments are never checked, so a broken link inside `fn insert_charge`'s comment survives. Turning it on later is one flag once the public surface is clean.

6. **Does `vpay-worker` keep linking `vpay-api` for `ResourceConfig`?** *Default: yes — unchanged.* This was decided in Step 4 (plan §5, D1) and Step 5 extends `ResourceConfig` with `endpoints_by_merchant_id`. Gained: no churn on two in-flight steps. Lost: a "cleanup" step ships with the worker still depending on a crate named `api` whose router it never mounts, which is the most visible SOLID complaint anyone will make about the result. Reopening it is a `vpay-config` move of ~150 lines and should be its own step, not folded in here.

7. **Do the four lanes run before or after Step 6 (deployment)?** *Default: after Steps 4 and 5 merge, before Step 6.* Gained: Step 6's deployment work reads the cleaned-up boot sequence and one `vpay_config::boot`. Lost: Step 6 slips by the length of Phase A. Running cleanup *after* Step 6 means the deployment step hardens the duplicated `init_tracing`/`exit_code_for` pair into container images and runbooks first.