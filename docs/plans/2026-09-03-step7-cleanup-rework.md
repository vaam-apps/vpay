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

## Refresh, 2026-09-03 (after Steps 4, 5, 5b merged; 5c and 6 pending)

Decisions taken by the orchestrator under the user's delegation on the refresh's open questions (do not
reopen): (8) `UnitOfWork::transaction` returns `Result<TxOutcome<T>, DbError>` with
`TxOutcome::{Commit(T), Abandon(T)}` so `fan_out_one`'s lost-race abandon and the confirm path's
drop-and-reread are expressible without an error sentinel; (9) `vpay-api`'s `sqlx` dependency for the
OP's `SqlxOpStore` (ADR-0010) is exempt from the repository split — say so in `docs/status.md`; (10)
`browser/mod.rs` migrates to `&dyn Repositories` in the same commit as `v1/payment_intents.rs`; (11) a
fifth lane owns `vpay-db` + `vpay-worker` doc externalisation and the `handle_deliver` split (disjoint
from Lane 3); (12) `backends/tests/integration/tests/browser_checkout.rs`, `sdks/stripe-js`, the Cypress
`checkout.cy.ts`, and Step 6's `webhooks.rs`/`worker_e2e.rs` assertions join the wire-behaviour guard
list before Phase A starts; `expected_suites`/`min_tests` are re-measured after Steps 6 and 5c land and
before Phase A. Decision (1) at the top governs over the "Default: keep `String`" sentence in the
questions section below.

The refresh report follows verbatim.

# Step 7 cleanup rework — scoping refresh

Read in full: `AGENTS.md`, `CLAUDE.md`, `docs/plans/2026-09-03-step7-cleanup-rework.md`, ADRs 0002/0006/0007/0011, `.xtask/src/main.rs`, `justfile`, the current `vpay-db`/`vpay-worker`/`vpay-api`/`vpay-provider`/adapters/`vpay-core` sources, plus `git diff origin/master HEAD --stat` (and the touched files themselves) for `vpay-step6-deploy` and `vpay-step5c-stripejs`. Tree measured: `claude/step7-cleanup` = `master` @ `68e81ce` (Steps 0–5, 5b merged; **not** 6 or 5c). Everything below is either a direct measurement on that commit or explicitly marked as inferred from the two unmerged diffs.

## Decisions already taken (do not reopen) — one internal contradiction worth flagging

The plan's top decisions (1)–(7) are binding per the task framing. One thing a fresh reader will trip on: decision (1) at the top says `ProviderError::{Transport, Malformed}` **gain a real `#[source]`** ("the correct model; ~200-line diff accepted") — but §"Decisions needed from a human", item 1, still writes *"Default: keep `String`, add `ProviderError::transport(rail, what, &err)`"*. These disagree. The top note is dated after and states the delegation explicitly, so it governs, but whoever implements section 1 should not stop at the "Default:" sentence in item 1 — cite the top-of-file note (line 12-13), not the bottom recommendation.

## (a) Re-measured baselines on this tree

Method matches the design's own (grep/wc, "production code" = lines before a file's first `#[cfg(test)]`; comment-stripping matches `.xtask/src/main.rs`'s `searchable`/`strip_cfg_test_items` at `.xtask/src/main.rs:718`/`:819`).

**Doc-comment ratio, per crate** (`backends/crates/*` + `backends/apps/*`, doc = `///`/`//!` lines, code = everything else before `#[cfg(test)]`):

```
vpay-adapter-mtn-momo    doc=490  code=795  61.6%
vpay-adapter-orange-money doc=455 code=741  61.4%
vpay-api                 doc=3362 code=3526 95.3%
vpay-config              doc=1200 code=951  126.2%
vpay-core                doc=560  code=752  74.5%
vpay-db                  doc=2944 code=2171 135.6%
vpay-ledger              doc=10   code=110  9.1%
vpay-provider            doc=406  code=382  106.3%
vpay-server              doc=197  code=448  44.0%
vpay-testkit             doc=203  code=110  184.5%
vpay-worker              doc=1077 code=1055 102.1%
vpay-worker-bin          doc=100  code=294  34.0%
TOTAL                    doc=11004 code=11335 97.1%
```

The design's number (81.2%, measured on `step4-worker`) and this one (97.1%) aren't the same scope necessarily (I can't confirm what the original run included beyond `backends/`), but the direction is unambiguous either way: **`vpay-db` is now the single biggest doc-volume crate in the workspace at 135.6%**, driven almost entirely by files that did not exist at design time — `events.rs`, `webhook_deliveries.rs`, `jobs.rs` (rewritten), `settlement.rs` — all written in the same narrative-doc house style as the rest of the crate. This matters for (d) below: **no lane in the design's four-lane split owns doc externalisation for `vpay-db` or `vpay-worker`.** Lane 1 = `vpay-core`+`vpay-ledger`, Lane 2 = `vpay-provider`+adapters, Lane 3 = `vpay-api`+`vpay-config` (+worker/binaries' `boot` only), Lane 4 = tooling. The two crates carrying 36% of the workspace's doc lines (`vpay-db` 2944 + `vpay-worker` 1077 of 11004) have no assigned externalisation owner.

Worst single files by ratio: `vpay-db/src/lock_keys.rs` 985.7% (tiny, not meaningful), `vpay-config/src/oauth.rs` 261.2%, `vpay-provider/src/http.rs` 228.0%, `vpay-api/src/op/token.rs` 226.6%, `vpay-api/src/jwks_cache.rs` 216.0%, `vpay-api/src/op/mod.rs` 182.7% (down from the design's cited 267%, but still the largest absolute-line offender), `vpay-db/src/webhook_deliveries.rs` 164.3%, `vpay-db/src/jobs.rs` 152.6%, `vpay-api/src/resource_auth.rs` 152.2%, `vpay-db/src/charges.rs` 150.6%, `vpay-db/src/payment_intents.rs` 149.7%.

**Functions ≥80 lines in production code** (excludes `backends/tests/*` and anything after a file's `#[cfg(test)]`, verified with a brace-depth scan that strips string literals — a naive scan miscounts on `format!` bodies containing `{`):

```
303  backends/apps/vpay-server/src/main.rs:167       fn run
244  backends/crates/vpay-api/src/v1/payment_intents.rs:678  fn confirm_once
224  backends/apps/vpay-worker-bin/src/main.rs:172   fn run
157  backends/crates/vpay-worker/src/webhooks.rs:867 fn handle_deliver
117  backends/crates/vpay-config/src/config.rs:463   fn validate_all
105  backends/crates/vpay-db/src/config_reconcile.rs:123 fn reconcile
102  backends/crates/vpay-adapter-orange-money/src/lib.rs:133 fn mint_token
```
7 total, down from the design's 10 — but **`webhooks.rs:867 handle_deliver` (157 lines) is new** (Step 5, didn't exist at design time) and is not in the design's ranked SOLID/DRY list (§5). `confirm_once` is **still exactly 244 lines**, unchanged content, just moved from `:519` to `:678`. `vpay-worker-bin`'s `run` grew 213→224. Note Step 6's diff adds metrics-emission lines *inside* `handle_deliver` itself (see (c)) — it will be longer still by the time Step 7 starts.

**`#[allow]`/`#[expect]` in production code:** still **4**, but the composition changed. Production sites (before each file's `#[cfg(test)]` at `run_loop.rs:934`, `handlers.rs:1466`, `op/clients.rs:189`): `run_loop.rs:563` and `:744` (`#[expect(clippy::too_many_arguments)]`, new — didn't exist in design), `op/clients.rs:88` (`#[allow(deprecated)]`, the one the design cited), `handlers.rs:434` (`#[expect(clippy::too_many_arguments)]`, new). `op/clients.rs:319` and `error.rs:1634` are **test-only** (past their file's `#[cfg(test)]`), so they don't count — the design's "2 legitimately deprecated" is now "1 deprecated + 3 too-many-arguments", all with documented `reason =`. S4's conclusion ("nothing to fix here") still holds, but citing the design's specific line numbers would now be wrong.

**`anyhow` outside binaries:** still clean. Only `vpay-db/Cargo.toml:71` (`[dev-dependencies]`, confirmed under that header at `:64`); `vpay-core/src/error.rs` matches on `anyhow::` are doc-comment prose only (`:31,:343,:345`), no `anyhow` in `vpay-core/Cargo.toml`. **New and load-bearing for decision 2's stated rationale:** `vpay-api/Cargo.toml:127` now has `sqlx.workspace = true` under real `[dependencies]` (moved there for `SqlxOpStore<sqlx::Postgres>` naming, ADR-0010's OP work — comment at `:118-126` explains why), which is unrelated to the repository refactor. **This means decision 2's "Gained" clause — "sqlx disappears from vpay-api's `[dependencies]`" — is now false regardless of what Step 7 does.** `vpay-worker/Cargo.toml` still has `sqlx` under `[dev-dependencies]` only (`:105`), so the no-`sqlx`-in-worker half of the rule still holds structurally.

**`pool.begin()` / `&mut PgConnection` sites:** grew from the design's 7 to **14** `pool.begin()` call sites outside `vpay-db`: `vpay-api/src/v1/payment_intents.rs:971,1072,1207` (3), `vpay-worker/src/run_loop.rs:484` (1), `vpay-worker/src/handlers.rs:547,625,1038` (3), `vpay-worker/src/webhooks.rs:602,761` (2) — plus `vpay-db`'s own internal `pool.begin()` at `signing_keys.rs:159,214`, `config_reconcile.rs:128`, `settlement.rs:178,249` (self-contained, don't leak). Inside `vpay-db`, 15 `*_in_tx` functions take `&mut PgConnection`/`&mut Transaction`: `charges.rs:177,330,385`, `webhook_deliveries.rs:167,364`, `events.rs:146`, `signing_keys.rs:257,274,329`, `settlement.rs:295`, `payment_intents.rs:394,463,577,658`, `jobs.rs:161`. `vpay-worker`'s call-surface into `vpay_db::` grew far beyond the design's "64 call sites, 4 transactional" — `webhooks.rs` alone (1404 lines, entirely new since Step 5) is a new, heavy consumer across `jobs`, `webhook_deliveries`, `events`, `settlement`, `provider_requests`.

**`ProviderError::{Transport, Malformed}` flattening:** unchanged behaviour, confirmed at the exact cited lines. `vpay-adapter-orange-money/src/lib.rs:666-680`'s `transport()` still hand-walks `Error::source()` into a `String`; `vpay-adapter-mtn-momo/src/lib.rs:225` still does `ProviderError::Transport(format!("mtn_momo: {error}"))` with no chain-walk. ~50+ construction/match sites across both adapters (mix of production and test) still exist. S5 stands exactly as written, same line numbers.

**Doctests:** still exactly **1** real doctest, `vpay-core/src/money.rs:142-151`. `resource_auth.rs:732` is still ```` ```ignore ```` at the identical line. No `test-doc` recipe in `justfile`, no `cargo test --doc` anywhere in `.github/workflows/ci.yml`. S1 stands unchanged, byte-for-byte.

**`rename_all` vs serde derives:** 12 `rename_all` sites (matches the design exactly), but total `#[derive(...Serialize|Deserialize...)]` sites grew from the design's implied 64 to **69** — 5 new derives arrived with Steps 4/5 and none of them carry `rename_all`, none of them are documented as intentionally exempt either. Concretely new-and-uncovered: `vpay-api/src/v1/events.rs:75 ListParams` (`#[derive(Debug, Deserialize)]`, no `rename_all`) — this is vpay's own wire per the convention's own "scope in" rule, and it's missing. `vpay-worker/src/jobs.rs` has 4 derives, 1 `rename_all` (worth checking which 3 lack it — job payload types, arguably rail-adjacent but not foreign-wire, so in scope). Split by category: **vpay-own wire/config missing it** — `vpay-api/src/model.rs` (7 derives, 1 `rename_all`), `vpay-api/src/v1/payment_intents.rs` (4, 0), `vpay-api/src/v1/events.rs` (1, 0, new), `vpay-config/src/config.rs` (3, 0), `vpay-config/src/lib.rs` (2, 0), `vpay-api/src/form.rs` (2, 0). **Rail-wire/foreign, correctly out of scope** — `vpay-adapter-mtn-momo/src/wire.rs` (7, 0, by design), `vpay-adapter-orange-money/src/wire.rs` (6, 0, by design), `vpay-adapter-mtn-momo/src/token.rs` (2, 0), `sdks/rust/src/client.rs` (4, 0, RFC 6749/Stripe envelope), `vpay-api/src/resource_auth.rs` (1, 0, JWT claim names).

## (b) Repository-trait surface, transaction fit

`vpay-db` structure today already half-anticipates the design: newer modules (`charges`, `config_reconcile`, `events`, `idempotency`, `jobs`, `lock_keys`, `payment_intents`, `provider_requests`, `settlement`, `webhook_deliveries`) are `pub mod`; older ones (`client_assertion`, `disabled_clients`, `error`, `health`, `migrations`, `pool`, `signing_keys`) are private `mod` with functions flattened via `pub use` at `lib.rs:88-108`. `pub use sqlx::PgPool` is still at `lib.rs:110` (the design's target to remove it). `SqlClientAssertionStore` at `client_assertion.rs:69,122` is still the one place a foreign trait (`authkestra_op::client::ClientAssertionStore`) is implemented over the pool — still the precedent the design cites.

Call surface (approximate, by table, both qualified `vpay_db::x::` and unqualified `x::` after `use vpay_db::{…, x}`):
- **vpay-api**: `payment_intents` ~13, `idempotency` ~7, `charges` ~7, `provider_requests` ~4, `events` ~5, `jobs` ~3, `config_reconcile` 1. Total roughly 40, up from the design's 35 — and that's *before* Step 5c's `browser/mod.rs` (see (c)), which adds a second consumer of `payment_intents::get_by_id` and of `confirm_once`.
- **vpay-worker**: dominated by `jobs` (33 refs), `webhook_deliveries` (12), `events` (11), `settlement` (8), `provider_requests` (7), `charges` (3) — `webhooks.rs` alone, entirely new since the design was written, accounts for most of the growth. The design's "64 call sites, 4 transactional" figure is stale on both counts; the true call-site count is well over 100 references, and the transactional shapes are more varied (see below).

**`UnitOfWork` fit, checked against the actual sites:**

1. **The confirm-path `drop(tx)` recovery site** (design's specific caveat) is real and located at `payment_intents.rs:1206-1232`, inside `insert_charge` (not `confirm_once` directly — that function *calls* `insert_charge`). The pattern: begin tx → insert charge → on `UniqueViolation` (the race the unique index catches), `drop(tx)` at `:1225`, then re-read via the **plain pool** (`charges::get_for_intent(pool, …)` at `:1226`) and return a 409. This is buildable under a `UnitOfWork` closure — it doesn't need to hold `tx` across an `.await` after dropping it — but it requires **restructuring**, not a mechanical swap: the closure would need to return an `Outcome::Charged(row) | Outcome::Conflict` rather than doing the pool re-read inline inside the match arm, because the closure only gets `&mut dyn TxRepositories`, not `&dyn Repositories`/pool. The caller, which still holds the untransacted handle, does the re-read after `transaction()` returns. This is a real, non-trivial but bounded piece of Phase A work — budget for it explicitly rather than treating decision (3)'s caveat as a one-line "verify it compiles."

2. **A second, more serious shape the design's caveat didn't anticipate** (it postdates the design, being Step 5 work): `vpay-worker/src/webhooks.rs:595-649 fn fan_out_one`. This begins a tx, loops over a variable number of endpoints calling `webhook_deliveries::create_in_tx` + `jobs::enqueue_in_tx` per iteration, then does a compare-and-swap (`mark_fanned_out_in_tx`) and **branches to either `tx.commit()` (success) or `tx.rollback()` while still returning `Ok(())`** (a lost race — `:643-646`). That's a **third outcome** a boolean `Result<(), DbError>` (commit-on-Ok, rollback-on-Err) closure signature cannot express: "the closure completed successfully but chose not to commit." The design's sketched `UnitOfWork::transaction(f) -> Result<(), DbError>` needs generalising — e.g. the closure returns `Result<TxOutcome<T>, DbError>` with `TxOutcome::Commit(T) | TxOutcome::Abandon(T)` — before `fan_out_one` can be ported. This is new scope for decision (3), not covered by "verify the confirm-path site compiles."

3. Everything else that opens a transaction (`handlers.rs:547,625,1038`, `run_loop.rs:484`, `webhooks.rs:761`, all of `vpay-db`'s internal `settlement::apply_succeeded`/`apply_failed`) is a plain commit-only shape and fits the simple closure cleanly. `settlement.rs`'s two functions in particular are already self-contained (take `pool: &PgPool`, begin/commit internally, never leak `&mut PgConnection` to a caller) — they were already correctly excluded from S3's "leaks the pool boundary" citation and need no `UnitOfWork` involvement at all; they can become ordinary async trait methods.

## (c) Step 6 and Step 5c interaction (read as diffs, not on a merged tree)

**Step 6 (`vpay-step6-deploy`, diff vs `origin/master`):** `vpay_core::metrics` (new module, `backends/crates/vpay-core/src/metrics.rs`, 815 lines) and `vpay_provider::measured::Measured` (`backends/crates/vpay-provider/src/measured.rs`, 397 lines). `Measured` wraps `Box<dyn ProviderAdapter>` and calls `error.code()` — a `Classify` method — to derive the `error_kind` Prometheus label; it never pattern-matches `ProviderError::Transport`/`Malformed` directly. **No conflict with decision (1):** changing `Transport`/`Malformed` to carry `#[source]` doesn't touch `Measured`'s code at all, because it only consumes the `Classify` interface. `vpay-db/src/charges.rs` and `settlement.rs` gain a `pub(crate) fn record_transition` (`charges.rs:132`) called from inside the same functions the repository-trait plan would wrap (`insert`, `mark_submitted`, `apply_succeeded`, `apply_failed`) — this is an internal implementation detail that moves unchanged inside `impl Charges for PgRepositories { … }`/`impl Settlement for PgRepositories { … }` bodies; **no interaction with the trait split's shape.** Step 6 does add ~20 lines of metrics-emission code *inside* `webhooks.rs::handle_deliver` and `record_failure` (`webhooks.rs` diff `+23`), which will make `handle_deliver` (already 157 lines on this tree) longer still by the time Step 7 starts — reinforces that the SOLID/DRY split for that function needs to be scoped against the post-Step-6 file, not this one.

**Step 5c (`vpay-step5c-stripejs`, diff vs `3fe33c7`):** adds `backends/crates/vpay-api/src/browser/mod.rs` (689 lines) — a new, unauthenticated `/v1/browser` surface (publishable key + payment-intent `client_secret`) that **directly calls `vpay_db::payment_intents::get_by_id`** (`browser/mod.rs:337`) via `State<PgPool>` (`:320,:423,:475`), and **directly calls `v1::payment_intents::confirm_once`** (`:485`), reusing it as-is rather than duplicating it. This has two concrete consequences for Step 7: (1) `AppState`/`RouterDeps` and the `State<PgPool>` extractor pattern now have **two** route-module consumers to migrate to `&dyn Repositories`, not one, and `browser/mod.rs` was not part of the design's 35-call-site estimate; (2) `confirm_once`'s signature is now shared by two call sites (`v1::payment_intents::confirm` and `browser::confirm`) — any repository-shape change to it must keep both call sites compiling and both integration suites green, so **`backends/tests/integration/tests/browser_checkout.rs` (1060 lines, new) and the whole of `sdks/stripe-js` (82+ tests) plus `frontends/tests/e2e/cypress/e2e/checkout.cy.ts` must be added to the "wire behaviour must not change" guard list** — the design's guard list (§8, "backends/tests/conformance, backends/tests/integration, sdks/rust/tests/…") predates Step 5c and doesn't mention any of these. `browser_checkout.rs` is also a new file under `backends/tests/integration/tests/`, which is a new test binary — `expected_suites` in `justfile` will need bumping again beyond whatever Step 6 leaves it at.

## (d) Scope changes to Phase A / the four lanes, given ~+15k lines

- **Phase A is bigger than budgeted.** Its own estimate ("114 call sites") is now a floor, not a number — real call-site count is well over 150 once `webhooks.rs` and `browser/mod.rs` are counted, and it must also absorb the `UnitOfWork` outcome-type generalisation from (b)(2), which the design didn't know it needed.
- **No lane owns `vpay-db`/`vpay-worker` doc externalisation.** Per (a), these two crates now carry 36% of the workspace's doc-comment volume, concentrated in files that postdate the design (`events.rs`, `webhook_deliveries.rs`, `settlement.rs`, `webhooks.rs`, `signing.rs`). Recommend folding this into Lane 3 (which already touches `vpay-worker` for the `boot` move) rather than leaving it unassigned, or splitting a fifth lane — see decisions below.
- **Lane 3's `confirm_once` split and Lane 4's guard-suite list both need Step 5c's `browser_checkout.rs`/`sdks/stripe-js`/Cypress `checkout.cy.ts` added explicitly**, per (c). Silent omission here is exactly the kind of surprise this repo's own review discipline (AGENTS.md: "revert one refactored call site and confirm a named test FAILS") is meant to catch — but only if the test is in the list to begin with.
- **`webhooks.rs::handle_deliver` (157 lines, growing under Step 6) is not in the design's SOLID/DRY ranked list (§5) at all.** It belongs in whichever lane owns `vpay-worker` (currently Lane 3, "boot move only" — scope needs explicit widening, or it's silently skipped).
- The rest of the lane split (Lane 1 `vpay-core`+`vpay-ledger`, Lane 2 `vpay-provider`+adapters) still looks accurate and low-risk — `vpay-core`'s doctests-candidates list (`ids.rs`, `money.rs`, `state.rs`, `failure.rs`) is untouched by any of Steps 4-5b/6/5c.

## (e) Guards that must stay green, and what needs updating first

- `just verify` (`verify-no-mocks`, `verify-status`, `verify-errors`) — all still pass their described shape on this tree; `verify-errors`'s "12 error type(s), all classified" is **confirmed live** (I counted `impl Classify for`/`impl vpay_core::Classify for`/`impl crate::error::Classify for` outside test modules: exactly 12 production impls). No change needed for Step 7 to start, but the design's own proposed extension (require every `#[from]` leaf variant to have an explicit arm in all five `Classify` methods, beside `has_classify_impl` at `.xtask/src/main.rs:1066`) is still unimplemented — still worth doing, still cheap.
- `just verify-ignored` — `justfile:165-166` currently pins `expected_suites := "38"`, `min_tests := "870"` on this tree (Step 5b's numbers). **These will be wrong the moment Step 6 and Step 5c land** — Step 5c adds at least one new integration test binary (`browser_checkout.rs`) and both branches add tests to existing binaries. Whoever lands those two steps must re-run `just verify-ignored` and bump both numbers in the same commit (the recipe's own history comment at `justfile:158-176` documents this discipline; follow it) — and Step 7 should not start until that's done, since Step 7's own "definition of done" (§7, "total test count ≥ pre-step figure") depends on a correct pre-step baseline.
- **Wire-behaviour tripwires, updated for the current tree:** `backends/tests/conformance` (unchanged, 26 cases), `backends/tests/integration` (52+ cases; will include `browser_checkout.rs` after Step 5c), `sdks/rust/tests/{resources,errors,op_conformance,token_exchange}.rs` (unchanged), **add** `sdks/stripe-js` (Step 5c, ~82 TS tests: `client.test.ts`, `compat.test.ts`, `errors.test.ts`, `form.test.ts`, `polling.test.ts`, `redirect.test.ts`), **add** `frontends/tests/e2e/cypress/e2e/checkout.cy.ts` (Step 5c, Cypress against `compose.e2e.yml`), **add** the two Step 6-modified suites `backends/tests/integration/tests/webhooks.rs` and `worker_e2e.rs` (both gained substantial new assertions per the diff). The risk each lane carries is specific: Phase A's repository-trait migration risks silently changing the confirm-path 409 semantics (mitigated by (b)(1)'s explicit "restructure, don't reuse the inline drop-and-reread" note) and the fan-out's abandon-vs-commit semantics (mitigated by (b)(2)'s outcome-type note); Lane 2's `CachedToken`/`read_rail_body` unification risks changing the margin-handling difference between MTN and Orange (the design already flags this as the one place the two token caches genuinely differ — worth a dedicated assertion, not just "tests still pass," since a passing suite with a subtly wrong margin would look identical until a token expires in production).

## (f) Decisions still reserved for the maintainer vs already taken

Already taken and unaffected by this refresh: (2) `&dyn Repositories` in both — still the right default, though its stated *benefit* re: `vpay-api`'s `sqlx` dependency is now partially false (see below); (4) comment-budget warn-only; (5) `cargo doc` without `--document-private-items`; (6) worker keeps linking `vpay-api` for `ResourceConfig`; (7) Step 7 after Step 6 — confirmed still the right call given Step 6 modifies files (`webhooks.rs`, `run_loop.rs`) that Step 7 would otherwise refactor out from under it.

New items surfaced by this refresh that need a human call before implementation starts:

1. **Does the `UnitOfWork` trait need a third outcome (commit / abandon-as-Ok / error), or should `fan_out_one`'s abandon-on-lost-race be restructured to fit the binary Ok/Err shape instead?**
   *Default: add the third outcome* (`TxOutcome::Commit(T) | TxOutcome::Abandon(T)`, `transaction<T>(f) -> Result<TxOutcome<T>, DbError>`). Gained: `fan_out_one`'s actual semantics (a lost race is not a failure) are expressible without a sentinel error type; the confirm-path recovery (b.1) can use the same vocabulary. Lost: every other transactional call site (the simple commit-only majority) now has to unwrap `TxOutcome::Commit` even though it never abandons — a small ergonomic tax on the common case for the sake of the two uncommon ones. The alternative — encoding "abandon" as a distinguished `DbError` variant the caller matches on and swallows — keeps the trait binary but makes "not an error" travel through the `Err` channel, which is the exact anti-pattern ADR-0011 exists to prevent elsewhere in this codebase.

2. **Does `vpay-api`'s now-real `sqlx.workspace = true` dependency (`vpay-api/Cargo.toml:127`, for `SqlxOpStore<sqlx::Postgres>`/ADR-0010) get exempted from decision 2's original rationale, or does Step 7 also abstract the OP's own storage behind a trait to fully remove `sqlx` from `vpay-api`?**
   *Default: exempt it — document in Step 7's write-up that `vpay-api`'s `sqlx` dependency survives the repository refactor for a reason unrelated to `vpay-db`.* Gained: no scope creep into the OAuth/OP subsystem, which Step 7 was never asked to touch. Lost: the repository refactor's headline promise ("sqlx disappears from vpay-api's dependencies") becomes only half true, and a future reader of `vpay-api/Cargo.toml` will see `sqlx.workspace = true` right next to a crate that just spent a step removing it — worth one sentence in `docs/status.md` saying why, so it doesn't read as an oversight.

3. **Does `browser/mod.rs` (Step 5c) get folded into Phase A's `&dyn Repositories` migration in the same commit as `v1/payment_intents.rs`, or does it lag behind on `PgPool` for one extra commit?**
   *Default: same commit* — `confirm_once` and `payment_intents::get_by_id` are shared between the two modules, so migrating one without the other means one caller passes `&dyn Repositories` and the other still passes `&PgPool` to functions whose signature can only be one or the other. Gained: no transitional shim. Lost: Phase A's "one commit per consuming crate" framing (design §7) has to become "one commit per consuming crate, but `vpay-api`'s commit now spans two route modules that didn't co-evolve" — slightly bigger review surface for that single commit.

4. **Does the doc-externalisation work for `vpay-db` and `vpay-worker` (now 36% of workspace doc lines, concentrated in Step 4/5 files with no design-time precedent) get assigned to Lane 3, or does it become a fifth lane?**
   *Default: fold into Lane 3* (which already owns `vpay-worker`'s `boot` move, so it already has write access to that crate) rather than adding a fifth lane. Gained: keeps the "disjoint file ownership per lane" property the design insists on, at the cost of Lane 3 becoming the largest lane by a wide margin. Lost: Lane 3 was sized for `vpay-api`+`vpay-config`+`boot`; adding `vpay-db`'s and `vpay-worker`'s doc externalisation roughly doubles its diff, which may be worth a fifth lane instead if wall-clock parallelism matters more than lane count — that trade-off is the maintainer's to make, not a default I'd pick blind.

5. **Are `sdks/stripe-js`, `frontends/tests/e2e/cypress/e2e/checkout.cy.ts`, and `backends/tests/integration/tests/browser_checkout.rs` formally added to the "must not change wire behaviour" guard list (design §8) before Phase A starts, or is that left implicit?**
   *Default: add them explicitly, in the same commit that updates `docs/status.md` for Step 7's kickoff.* Gained: the review lens ("revert one call site, confirm a named test FAILS") has an actual named test to point at for the `browser` surface, which otherwise has zero mentions anywhere in the Step 7 design. Lost: nothing — this is pure downside-avoidance with no real cost, which is why I don't see a live tradeoff here, just an omission to fix.

**Files most load-bearing for this scoping, for reference:** `docs/plans/2026-09-03-step7-cleanup-rework.md`, `backends/crates/vpay-db/src/lib.rs:88-110`, `backends/crates/vpay-worker/Cargo.toml:22-30,100-105`, `backends/crates/vpay-api/Cargo.toml:118-127`, `backends/crates/vpay-api/src/v1/payment_intents.rs:678-921,1206-1232`, `backends/crates/vpay-worker/src/webhooks.rs:595-649,867-1023`, `backends/crates/vpay-adapter-orange-money/src/lib.rs:666-680`, `backends/crates/vpay-adapter-mtn-momo/src/lib.rs:225`, `backends/crates/vpay-core/src/money.rs:142-151`, `backends/crates/vpay-api/src/resource_auth.rs:732`, `.xtask/src/main.rs:611-681,718,819,1066`, `justfile:120-193`, and (unmerged, read as diffs) `vpay-step6-deploy`'s `backends/crates/vpay-provider/src/measured.rs`, `backends/crates/vpay-core/src/metrics.rs`, and `vpay-step5c-stripejs`'s `backends/crates/vpay-api/src/browser/mod.rs`.

**What I measured versus inferred:** everything under (a) and the `vpay-db`/`vpay-worker`/`vpay-api` call-site counts under (b) are direct measurements on `68e81ce`, reproducible with the grep/python commands used above. Step 6 and Step 5c's content (§(c), and the guard-list additions in (e)) is read from their diffs against their respective bases, not from a merged tree — I did not build or run either branch, and I did not run `cargo nextest`/`docker` anywhere (no container runtime invoked in this session), so the exact post-merge `verify-ignored` numbers in (e) are stated as "will need updating," not measured.

## Phase A escalation, 2026-09-03 — decisions (13) and (14)

Phase A stopped at the guard: removing `vpay-db`'s free functions breaks 56 fixture call sites across the
integration binaries, and giving `ProviderError::{Transport, Malformed}` a real `#[source]` breaks three
conformance assertions that pattern-match the single-field tuple variants. Decisions under delegation:

- **(13)** The integration test files (`backends/tests/integration/tests/**`, incl. `support/mod.rs`) MAY
  receive mechanical call-site changes so fixtures reach Postgres through `PgRepositories`/the trait
  objects instead of the removed free functions. Every `assert!`/`assert_eq!`/`matches!`/`panic!`/
  `expect(` line in those files must remain byte-identical; reviewers diff them. No free function is kept
  `pub` for fixtures' sake.
- **(14)** The three conformance assertions (`adapter_conformance.rs` ~:747, ~:947, ~:989) MAY be adapted
  to the new shape: `matches!(error, ProviderError::Transport { .. })`, the same for `Malformed`, and the
  body-cap message check becomes `error.to_string().contains(&MAX_RAIL_BODY_BYTES.to_string())` — the
  context travels in `Display`, the `reqwest`/body error in `source()`. No `Deref<Target = str>` on an
  error type. Nothing else in the conformance suite changes; the 26 cases stay 26.


## Phase A outcome, 2026-09-03 — what landed, where it deviated, and the new baseline

Phase A is merged on `claude/step7-cleanup`. This section is the record of how the
implementation differs from the design above; the design is *not* edited, so a reader
comparing the two can see the shape of every decision that was taken at the keyboard.

### Deviations from the design, and why each one was taken

1. **`UnitOfWork::transaction` is generic over the error type, not pinned to `DbError`.**
   Decision (13)'s sketch was `transaction<T>(f) -> Result<TxOutcome<T>, DbError>`. Three
   call sites raise their *own* layer's error from inside the unit of work: the confirm
   path's "the rail accepted a charge whose intent moved" invariant (`ApiError`) and two
   worker sites whose payload will not encode (`JobError`). Pinning the closure to
   `DbError` would have forced each of them either to smuggle the error out through the
   success channel or to relabel it as storage — the exact shape ADR-0011 exists to stop.
   The signature is `transaction<'a, T, E, F>(&self, f: F) -> Result<TxOutcome<T>, E>`
   with `E: From<DbError> + Send`, so the common case is still spelled `E = DbError`.
   Cost: every call site names its error type once (`Ok::<_, DbError>(…)`).

2. **`PendingTransaction` owns its `sqlx::Transaction<'static, Postgres>` rather than
   borrowing one.** Not an aesthetic choice: the closure signature
   `for<'t> FnOnce(&'t mut (dyn TxRepositories + 'a)) -> TxFuture<'t, _>` is only usable
   because the `'a` on the trait object gives the implied bound `'a: 't`, and that is
   what lets a closure borrow the caller's locals (`&NewCharge`, a `&str` merchant id)
   across an `.await`. With a borrowing `PgTransaction<'t>` the same signature forces
   every capture to be `'static`, which no call site in this workspace can satisfy. This
   is written on the type itself, because it looks like an easy simplification.

3. **`Migrations` is a trait on `Repositories`, which the design did not anticipate.**
   `run_migrations` is not a table family and had no home once `PgPool` stopped leaving
   the crate. Making it a fourteenth trait rather than leaving a free function taking a
   pool is what let `pub use sqlx::PgPool` go; the alternative (keep one `pub fn` taking
   a pool) would have kept the whole re-export alive for one caller.

4. **`vpay_db::connect_lazy` is a new public function the design did not ask for — and it
   now has a mechanical guard.** `connect` is deliberately eager, which makes "a handle
   whose queries fail" unobtainable; `vpay-api`'s own unit tests need exactly that to
   prove an unreachable database produces a refusal rather than an admission, and before
   the repository split they built the lazy pool themselves with `sqlx`. It is **not** a
   test double (the pool is the real `sqlx` one, every query really reaches Postgres),
   which is precisely why nothing in ADR-0006's dependency rules would ever object to a
   binary using it. `cargo xtask verify-no-mocks` now fails the build if `connect_lazy`
   appears in non-test code anywhere under `backends/apps`, and the function carries
   `#[doc(hidden)]` and a sentence naming that guard. Proven by putting the call into
   `vpay-server`'s `run` and watching the check name the file and the line's rule.

5. **`verify-errors`' delegation check exempts more than the ADR's "methods that do not
   match on `self`" wording implies, and the exemption is now written in terms of what it
   actually tests.** ADR-0011's amendment says "each `Classify` method whose body matches
   on `self` must name `Self::<Variant>` explicitly. Methods that do not match on `self`
   are exempt — there is no wildcard to hide in." The first implementation read that
   literally and searched for the string `match self`, which made the rule opt-out: the
   same ladder written `if let Self::Db(e) = self { … } else { … }`, `matches!(self, …)`
   or `match *self` skipped the method entirely, and the trailing `else` answered for an
   unnamed `#[from]` leaf exactly as a `_ =>` arm would. The check now recognises all five
   spellings (`.xtask/src/main.rs`'s `SELF_DISCRIMINATING_FORMS`). The ADR's *intent* is
   unchanged and it is not amended again; what changed is that the implementation now
   matches it. `a_from_variant_swallowed_by_an_if_let_ladder_is_reported` fails if the
   list is narrowed back.

   The banner's second number was also arithmetic rather than a count — it accumulated
   `from_variants().len()` once per *source file* in the crate and subtracted one per
   violation. It is now the number of `#[from]` variants whose declaring file also carries
   the `Classify` impl and whose every discriminating method names them.

### Re-baselined function-length table (§5's ranked list, measured on this tree)

Method as in §"(a) Re-measured baselines": production code only — `backends/crates/**` and
`backends/apps/**` `src/`, excluding each crate's own `tests/` directory and everything
after a file's first `#[cfg(test)]`; brace-depth scan with string literals and line
comments stripped, so a `format!` body containing `{` does not miscount. Threshold **≥ 80
lines**, signature line through closing brace inclusive.

```
371  backends/apps/vpay-server/src/main.rs:168                fn run
330  backends/apps/vpay-worker-bin/src/main.rs:181            fn run
243  backends/crates/vpay-api/src/v1/payment_intents.rs:800   fn confirm_once
210  backends/crates/vpay-worker/src/handlers.rs:226          fn poll_charge
173  backends/crates/vpay-worker/src/run_loop.rs:621          fn run_loop
167  backends/crates/vpay-worker/src/webhooks.rs:890          fn handle_deliver
138  backends/crates/vpay-config/src/config.rs:463            fn validate_all
106  backends/crates/vpay-worker/src/handlers.rs:530          fn resubmit_charge
105  backends/crates/vpay-db/src/config_reconcile.rs:132      fn reconcile
102  backends/crates/vpay-adapter-orange-money/src/lib.rs:133 fn mint_token
 89  backends/crates/vpay-api/src/v1/payment_intents.rs:1070  fn persist_submitted
```

11 functions, against the design's 10 and the refresh's 7 — the step has not yet reduced
any of them, and three that the earlier counts did not list are now on it:

- **`handlers.rs:226 poll_charge` (210)** and **`handlers.rs:530 resubmit_charge` (106)**
  are in no ranked list anywhere in this design. They are Step 4 code that the refresh's
  brace-depth scan missed. Whichever lane owns `vpay-worker` owns them.
- **`run_loop.rs:621 run_loop` (173)** likewise.
- Both binaries' `fn run` grew again (303 → 371, 224 → 330) with Step 6's observability
  listener. §5 item 1's boot-duplication move and item 2's `fn run` split are now the
  largest single win available and are still unstarted.
- `confirm_once` is 243 rather than 244 and moved `:678` → `:800`; the repository
  migration restructured its duplicate-charge recovery (`TxOutcome::Abandon` plus a
  re-read on the untransacted handle) without changing its length. §5 item 4 stands.
- `persist_submitted` (89) is new to the list for the same reason — it is where the
  second `Abandon` site lives.

### Notes for the remaining lanes

- **`jobs.last_error` now carries the source chain.** `run_loop.rs:378` renders the
  failure through `vpay_core::error::source_chain` before recording it, so a composite's
  own `Display` no longer hides the leaf (`run_loop.rs:376` says why). This is the shape
  the ADR-0011 amendment asks for, and it is done for this column only.
- **`webhook_deliveries.response_excerpt` still stores `Display` alone.** (The column is
  `response_excerpt`, CHECK <= 2000 — this note called it `last_error` when it was
  written, which is a different column on a different table; corrected here rather than
  left to mislead the lane that had to find it.) One write site,
  `vpay-worker/src/webhooks.rs:1045-1053`, which formats `no response: {error}` from a
  `reqwest::Error`. Bringing it in line with `jobs.last_error` is **lane 2/5 scope**, not
  Phase A's: it is a `vpay-provider`-adjacent rendering decision and the column's
  excerpt bound has to be re-checked against a longer string. **Done by lane 5**, which
  is where the `display_with_chain` rendering and the `bounded_excerpt` bound landed.
- **`ProviderAdapter`'s trait methods carry no `# Errors` sections.** `vpay-provider/src/
  lib.rs` has none at all today. Adding them is **lane 2/5 scope** (adapter error
  surface, §2), and it is the natural place to state which `ProviderError` variants each
  operation may raise now that `Transport`/`Malformed` carry a typed `#[source]`.
- **`TxOutcome::Abandon` does not surface a rollback failure.** It logs at `warn!` and
  returns `Ok(Abandon)`: `ROLLBACK` is best-effort by construction, so a failure changes
  nothing about the database and only about what the caller may report — and both
  abandoning call sites have an answer that must survive (the confirm path's `409`,
  `persist_submitted`'s `Internal` "a rail may hold a live payment" alert). Staged in
  `vpay-db/tests/postgres.rs` by terminating the backend that holds the open transaction,
  with the commit path as the control.

## Lanes outcome, 2026-09-03 — what landed, where it deviated, what did not land

All five lanes are merged on `claude/step7-cleanup`, on top of Phase A. As with the Phase A
outcome section above, the design and refresh are not edited — this is the record of how the
five lanes actually landed against them, plus the fifth lane the refresh's decision (11) added.

### Per lane

- **Lane 1 — `vpay-core` + `vpay-ledger`:** doctests and doc externalisation. The design's
  ~18-doctest candidate list landed as part of `vpay-core`'s 42 (money, ids, state, settlement,
  failure) plus `vpay-ledger`'s share of the workspace's 77.
- **Lane 2 — `vpay-provider` + both adapters:** `CachedToken` (the shared half of the
  MTN/Orange token cache), `read_rail_body`, and `# Errors` sections on `ProviderAdapter`'s
  trait methods and the two adapters' public functions, backed by
  `#![warn(clippy::missing_errors_doc)]` on `vpay-provider` — the one crate in the workspace
  that carries the lint the design proposed workspace-wide.
- **Lane 3 — `vpay-api` + `vpay-config`:** the `confirm_once` split (243 → under 80 lines, six
  steps in five named functions), `vpay-server`'s `fn run` split (371 → under 80), the shared
  boot sequence, and `rename_all` on the types that model vpay's own wire or config.
- **Lane 4 — `.xtask` + `justfile` + CI + `docs/`:** `cargo xtask verify-docs` (a report, never
  a gate — Step 7 decision (4)), `just test-doc` wired into `just ci` and into CI's `rust` job,
  and the CI plumbing both depend on.
- **Lane 5 — added by the refresh's decision (11), not in the original four-lane design:**
  doc externalisation for `vpay-db` and `vpay-worker` (the two crates the four-lane split left
  unowned, per refresh (d)); `display_with_chain` and the `bounded_excerpt` bound for
  `webhook_deliveries.response_excerpt`; and the `handle_deliver` / `poll_charge` / `run_loop` /
  worker-binary `fn run` splits recorded in the re-baselined function-length table above.

### Deviations from the design and refresh

- **`vpay_config::boot` vs `vpay-api`.** §5 item 1 and decision (4) of §7 both name
  `vpay_config::boot` as the destination for the shared boot sequence. It landed in `vpay-api`
  instead, as `vpay_api::boot` (`backends/crates/vpay-api/src/v1/boot.rs`, re-exported as
  `crate::boot`) — see that module's own doc comment for why: `vpay-api` is the only crate both
  binaries already link that also depends on `vpay-config` (the YAML), `vpay-provider` (the
  port) and `vpay-db` (the seed types), so putting it in `vpay-config` would have added a
  dependency edge `vpay-config` does not otherwise need.
- **`docs/reference/<crate>.md` vs `docs/design/<topic>.md`.** §6's layout plans eleven
  `docs/design/<topic>.md` pages, one per named topic (`request-ids.md`, `jwks-cache.md`, and
  so on). What shipped is `docs/reference/<crate>.md` — one page per crate rather than one page
  per topic, indexed by `docs/reference/README.md`. Six pages exist, covering eight of twelve
  crates (`rails.md` covers `vpay-provider` and both adapters together); `vpay-ledger`,
  `vpay-testkit` and both binaries have none.

### What was not done

- **`verify-serde`.** §7's Lane 4 scope names it explicitly; it was not built. The
  `rename_all` convention (22 types, up from the base's 2) is enforced by review, not by a
  scanner — the same status the design's own §6 flags for the comment budget.
- **The `docs-check` link checker.** §6 says implementing it is Lane 4's job, alongside the
  `verify-docs` it did build; `justfile`'s `docs-check` recipe still only runs
  `cargo xtask verify-status` and prints "link checking is not implemented yet." A relative
  `docs/reference/...` link resolved from its file's own directory is exactly what's missing
  here, and it is where this pass's own link-depth fixes (item 6) came from having to check it
  by hand.
- **`form.rs` / `PostRequest` splits.** §5 items 3 and 5 (`PostRequest` + idempotency
  plumbing out of `payment_intents.rs`, and `form.rs` into `form/parse.rs` + `form/extract.rs`)
  are unstarted; `payment_intents.rs` is still the largest file in the workspace.
- **§5.1's boot-helper move.** `exit_code_for`, `install_recorder`, `install_crypto_provider`
  and `init_tracing` are still duplicated verbatim between `vpay-server/src/main.rs` and
  `vpay-worker-bin/src/main.rs` — the ~120 duplicated lines §5 item 1 named are still there;
  only the boot *sequence* (adapters, config, seeds, migrate, reconcile) moved to a shared
  module, not these four smaller helpers.
- **The ≤40% doc-ratio target.** §6: "Target: 81.2% → ≤40%." Measured on the final tree,
  prose-to-code is **88.6%** (equivalently 88.5% by the same convention, after this pass's own
  doc trims) — not close. `cargo xtask verify-docs`'s own two-column convention (prose against
  code, examples counted separately) reads 100.6% on the same tree. This was Step 7's known,
  named miss (`docs/status.md`'s own header says so), not a new finding.
- **`missing_docs` is not a workspace lint**, and the design never proposed making it one.
  Measured directly on this tree with `RUSTFLAGS="-W missing_docs" cargo check --workspace
  --lib`: **62 public items undocumented** — 31 struct fields, 14 traits (13 of them the
  `vpay-db` repository trait surface: `Charges`, `ConfigReconcile`, `Events`, `Idempotency`,
  `Jobs`, `PaymentIntents`, `ProviderRequests`, `Settlement`, `WebhookDeliveries`,
  `ClientAssertions`, `DisabledClients`, `Health`, `Migrations`; the fourteenth is
  `vpay_provider::ProviderAdapter::capabilities`, reported as a method rather than the trait
  itself), 8 enum variants, 2 crates (the two `backends/tests` support crates, whose `lib.rs`
  is a one-line pointer to `tests/`), 2 structs, 2 constants, 1 method. Follow-up, not this
  pass's scope: the repository trait surface is the majority of it by construct, not by
  accident, and is the natural place to start if a future lane takes this on.

### Measured after-state

- `just test-doc` / `cargo test --doc --workspace`: **77 passed, 1 ignored, 0 failed** — the
  workspace's only ignored doctest is a ```` ```rust,ignore ```` fence in `sdks/rust/README.md`,
  pulled in via `#[doc = include_str!(...)]`, outside every lane's scope (`sdks/rust` is not
  scanned by `verify-docs` either).
- `just verify-ignored`: **`0 ignored (expected 0), 39 test binaries (expected 39), 999 total
  (minimum 950)`** (from `docs/status.md`'s own final-state bullet, re-verified as part of this
  pass's gate run).
- `cargo xtask verify-docs`, measured on this tree after this pass's own doc-comment trims:
  prose 13 027 / example 1 111 / code 12 938 across twelve crates — **100.6%** prose-to-code by
  its own convention; **88.5%** by the design's older convention (every doc line as prose,
  looser denominator). Six production functions of 80 lines or more (`validate_all` 138,
  `config_reconcile::reconcile` 105, `mint_token` 102, `poll_charge` 93, `persist_submitted` 89,
  the worker binary's `boot` 80); zero ```` ```ignore ```` fences in the two trees it scans;
  four `#[allow]`/`#[expect]`, unchanged from Phase A.
