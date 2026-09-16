# sqlx 0.8 → 0.9, and the three OP stores that pinned 0.8

_Archived from [docs/status.md](../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../` because the file moved one directory down._

_One link in it also changed target, in the same pass that split
`docs/reference/vpay-db.md`: the section it pointed at, "Dynamic SQL strings and
sqlx 0.9", moved to `vpay-db/dynamic-sql.md`, so the link names that page instead of
an anchor that no longer exists. The sentence around it is untouched._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](README.md) is the index of them._

### sqlx 0.8 -> 0.9 (2026-09-05)

**Landed.** `[workspace.dependencies] sqlx` is `0.9` and `Cargo.lock` resolves
**exactly one sqlx major**:

```
$ cargo metadata | (every package whose name starts with "sqlx")
sqlx 0.9.0  sqlx-core 0.9.0  sqlx-macros 0.9.0  sqlx-macros-core 0.9.0
sqlx-mysql 0.9.0  sqlx-postgres 0.9.0  sqlx-sqlite 0.9.0
```

(`sqlx-mysql`/`sqlx-sqlite` appear because `cargo metadata` enumerates
optional dependencies whether or not a feature activates them; nothing enables
them — `default-features = false` plus eight named features — and `cargo tree`
does not show them.) `cargo tree -d` lists **no sqlx version duplicate**; the
two `sqlx-core v0.9.0 (*)` lines in its output are the same version twice, the
same shape it prints for `base64 v0.22.1` and `log v0.4.33` on this workspace.

**And it is a gate now, not a sentence** (added 2026-09-05 by the review of
this branch). `deny.toml`'s `[bans]` denies `sqlx < 0.9` and `sqlx-core
< 0.9` outright. Until that entry existed, nothing in `just ci` held the
one-major property: `multiple-versions` is `warn`, so restoring
`authkestra-op`'s `sqlx-postgres` feature — one word in one manifest —
resolved 0.8.6 and 0.9.0 side by side with `cargo deny check` still printing
"bans ok" and every other recipe green. Measured, by doing exactly that:
`cargo tree -d` then listed `sqlx v0.8.6` and `sqlx v0.9.0`, `sqlx-postgres`
at both majors, and the whole gate stayed green. With the ban in place the
same mutation is `error[banned]: crate 'sqlx = 0.8.6' is explicitly banned`
and `cargo deny check bans` exits 2. Both bounds move together when the
workspace goes to 0.10, in the same commit as the pin.

**A cost the bump does carry, recorded rather than left for someone to find
in a `cargo deny` log:** sqlx 0.9 brings the RustCrypto 0.11 generation
(`sha2` 0.11.0, `digest` 0.11.3, `hmac` 0.13.0, `hkdf` 0.13.0,
`crypto-common` 0.2.2, `block-buffer` 0.12.1) alongside the 0.10 generation
the rest of the graph still uses, so `cargo deny check bans` warns about six
duplicate crates it did not warn about before — fourteen in total now, eight
before. Two independent SHA-2/HMAC implementations compile into
`vpay-server`. `multiple-versions` stays `warn`, deliberately: the duplication
is upstream's to resolve as `jsonwebtoken`/`rustls`/`sqlx` converge, and
turning it into an error would gate this repo on other people's release
schedules. Net package count went **down**, 477 -> 469.

**Why at all.** `cratestack-sqlx 0.11.1` pins `sqlx-core =0.9.0` and
`sqlx-postgres =0.9.0`. Under the old pin, a crate depending on both
CrateStack and `vpay-db` resolved two majors — two `sqlx::Transaction` types
that cannot share a transaction. See the "decisive negative" below.

**The feature list is copied across unchanged**, checked against both
releases' own manifests rather than assumed: all eight exist in 0.9.0 with the
same meaning, `tls-rustls-ring` still expands to `tls-rustls-ring-webpki`
(vendored Mozilla roots — non-negotiable for a `FROM scratch` image, ADR-0004),
and the four features 0.9 deleted (sqlx#3821: the runtime+TLS combinations)
were none of vpay's. `webpki-roots` moves 0.26.11 -> 1.0.9 and is still
`CDLA-Permissive-2.0`, which `deny.toml` already allows with its own written
reason, so that file needed no edit.

**One API change reached this workspace, and it is the only one.** sqlx#3723
made `query`/`query_as`/`query_scalar` take `impl SqlSafeStr`, implemented for
`&'static str` and for `AssertSqlSafe` and nothing else, so a `format!`-built
`String` no longer compiles as a statement. `cargo check --workspace
--all-targets` produced **36 errors, all of that one kind, all in `vpay-db`**,
and zero warnings. Everything the bump might have been expected to break did
not: `Executor`/`Acquire` bounds, `Transaction<'static, Postgres>` in
`PendingTransaction`, `PgRow`/`FromRow`, `sqlx::migrate!`, `sqlx::Error`
variants, `PgPoolOptions::acquire_timeout`/`connect_lazy`, `classify_write`.
There are no `query!` macros in this workspace and no `.sqlx/` directory, so
`cargo sqlx prepare` is **not applicable** rather than skipped.

**Two more sites that a lib-only check would not have found**, both in
`vpay-tests-integration` and both new to this pass (the earlier measurement in
`docs/plans/exp12-notes/opus.md` could not build that crate at all):
`postgres_smoke.rs`'s `SELECT COUNT(*) FROM {table}` and its
`insert_signing_key({expires_at_clause})`, each now wrapped with its own audit
comment — a table name and a SQL expression, neither of which can be a bind
parameter, both interpolating literals written in that file. And
`payment_intents.rs`'s `count(pool, sql, bind)` helper, whose `sql: &str`
became `sql: &'static str`: every caller passes a literal, so tightening the
parameter keeps the _compiler_ doing the checking instead of moving it into a
comment.

**The audit is a test, not a comment.** `AssertSqlSafe`'s contract is that the
caller audited the string; a contract discharged by prose is discharged by
whoever last read the prose. All 36 statements interpolate a `const … : &str`
declared in `vpay-db` — the five per-module `COLUMNS`, `OPEN`,
`LIVE_CHARGE_STATES`, `SETTLEABLE_STATUSES`, `CLAIM_RETURNING`,
`PREVIOUS_STATE` — or `direction`, which is
`if backwards { "ASC" } else { "DESC" }`. **No caller-supplied value reaches a
statement string anywhere in the crate**; every id, cursor, limit and status is
already a bind parameter. `vpay_db::sql_audit` (test-only, 5 tests) reads the
crate's own sources and enforces exactly that, and it was **proven to fire by
three mutations**, each reverted: interpolating `{payment_intent_id}` into
`charges::get_for_intent`; redefining `direction` as something other than the
two-literal `if`; and wrapping a fresh `format!` in `AssertSqlSafe` instead of
the audited `sql` variable. The third is the one that matters — without it the
audit could be bypassed by not using the variable the audit looks at. Full
reasoning: [docs/reference/vpay-db.md § dynamic SQL strings and
sqlx 0.9](../reference/vpay-db/dynamic-sql.md#dynamic-sql-strings-and-sqlx-09).
`QueryBuilder` was considered and rejected there.

**The MSRV floor moved, 1.88 -> 1.94.** `rust-version` in the root
`Cargo.toml` is derived from `cargo metadata`'s `rust_version` across the whole
resolved graph, and sqlx 0.9.0 declares `1.94.0` where 0.8.6 declared none;
the seven `sqlx-*` packages are now the sole maximum. Re-derived by
measurement over all 469 packages, 112 of which declare nothing. The toolchain
pin (`1.98.0`) is unaffected and is what this workspace is actually compiled
with; the floor has never been verified by compiling under 1.94.

**Supply chain.** `cargo deny check`: **advisories ok, bans ok, licenses ok,
sources ok**. `cargo tree -i` finds no `aws-lc-rs`, `aws-lc-sys`,
`openssl-sys` or `native-tls`. `deny.toml` was not edited. The bans warnings
grew by the RustCrypto 0.10/0.11 split sqlx 0.9 pulls in (`digest`,
`sha2`, `hmac`, `hkdf`, `block-buffer`, `crypto-common`, `cpufeatures`) —
warnings, because `multiple-versions = "warn"`, not errors. The two
`license-not-encountered` warnings (`CC0-1.0`, `MPL-2.0`) are unchanged:
measured at two before the bump and two after, by running
`cargo deny check licenses` against the pre-bump lockfile.

**The `sqlx-core` TLS claim was re-read at 0.9.0**, because that is exactly the
kind of statement about a dependency's internals a major bump invalidates. It
still holds: `handshake` selects `rustls::crypto::ring::default_provider()`
under `_tls-rustls-ring-webpki` and hands it to `builder_with_provider`, never
calling `CryptoProvider::get_default()` — so `vpay-db` still does not need to
install a process-wide provider.

**The decisive negative, re-measured on this branch.** A scratch crate
_outside_ the workspace depending on `cratestack-sqlx = "0.11.1"` **and** on
`vpay-db` by path:

- with the workspace pin at **0.8**: `sqlx 0.8.6`, `sqlx-core 0.8.6` **and**
  `sqlx-core 0.9.0`, `sqlx-postgres 0.8.6` **and** `sqlx-postgres 0.9.0` — two
  majors, i.e. two incompatible `Transaction` types;
- with the pin at **0.9**: one major, and `cargo check` finishes
  (`Checking cratestack-sqlx v0.11.1 … Checking vpay-db … Finished`).

The scratch crate was deleted afterwards and, ~~on that branch, **no
CrateStack crate was added to this workspace**~~ — **superseded 2026-09-06:
twelve of them were.** See "The first CrateStack read" below. This bump is
what made that possible, and it is also why `sqlx` is now pinned `=0.9.0`
rather than `"0.9"`: `run_in_tx` accepts vpay's `Transaction` only while both
halves resolve the same `sqlx-core`, and a caret pin would let 0.9.1 break
that with a trait error nobody would read as a version problem.

~~**Reserved for the maintainer.** Whether to adopt `cratestack-sqlx` at all
now that it resolves~~ — **answered 2026-09-06: adopted, for one read.**
Asking authkestra upstream to move `authkestra-store-sqlx` to sqlx 0.9 (see
the section below) is still open.

**The gate, on the whole branch.** `just ci` **exit 0**, end to end, on this
machine on 2026-09-05:

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1277 tests across 41 binaries
     Summary [1029.968s] 1277 tests run: 1277 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1277 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
```

1277 is 1272 (the section below) plus `vpay_db::sql_audit`'s five.

**Re-measured 2026-09-05 after the sabotage review of this branch, in full:**

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1278 tests across 41 binaries
     Summary [741.013s] 1278 tests run: 1278 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1278 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
JUST_CI_EXIT=0
```

`just test-doc`: **90 passed, 0 failed, 1 ignored**, unchanged. Web: vitest
across 8 packages, all passing. Image: `docker buildx build -f
backends/Dockerfile --target server .` on a review-owned builder (removed
after), `--version` prints `vpay-server 0.1.0`, **16 MB** — the same figure
the implementer measured. **1278, not 1277:**
The extra case is `vpay_db::sql_audit`'s
`a_positional_capture_is_reported_and_is_neither_a_constant_nor_allowed`, and
the reason it exists is a finding rather than an addition: as first written,
the injection audit below could be walked straight past by spelling the
interpolation positionally. `format!("SELECT {COLUMNS} FROM charges WHERE
payment_intent_id = '{}'", payment_intent_id)` — a live SQL injection through
`AssertSqlSafe`, in the one crate that wraps it 36 times — passed all five
tests, because the scanner dropped a capture with no name and only the
`{payment_intent_id}` spelling was ever mutated. `interpolations` now reports
an unnamed capture as a violation on sight, and the mutation that was silent
now fails `every_interpolation_into_a_statement_is_a_crate_constant`. See
`docs/reference/vpay-db.md` § dynamic SQL strings and sqlx 0.9.
`just test-doc`: **90 passed, 0 failed, 1 ignored** (the ignored one is
`sdks/rust`'s README block, pre-existing). Web: vitest across 8 packages, all
passing. **An earlier attempt at this same gate failed** and is recorded
rather than dropped: `vpay-db::postgres
an_abandoned_transaction_survives_a_rollback_it_cannot_send` hit
`failed to create a container: Timeout error` after 120 s, with 35 containers
up and a load average of 28 on this shared host — a bare `docker run
postgres:16-alpine` took 19.5 s to create at that moment. Nothing else ran
(nextest cancels on the first failure), so that run is evidence about the
host, not about the code; `cargo nextest run -p vpay-db` afterwards is
**91 tests run: 91 passed, 0 skipped**, that case included.

**The image builds and runs.**
`docker buildx build -f backends/Dockerfile --target server .` on a private
builder (`vpay-exp12b-opus`, removed afterwards; the shared default builder
was never touched or pruned): **exit 0**, `docker run --rm … --version` prints
`vpay-server 0.1.0`, image **16 MB** (`FROM scratch`, musl static, ADR-0004).

**Rebased onto `8d907f9` on 2026-09-05, and the gate re-run there.** The three
transcripts above were measured on `d086084`; [PR
#44](https://github.com/vaam-apps/vpay/pull/44) landed on `master` in between
and added the `schemas/vpay.cstack` drift case to
`backends/tests/integration/tests/postgres_smoke.rs` ("The measured drift"
under CrateStack, below). One conflict, in `justfile` and in exactly one
place: both branches appended to the comment above `expected_ignored`, #44
recording 1271/42 and this branch 42 → 41. **Both blocks were kept** — each
names the base it was measured on — and the constant is 41, because #44's case
joined a test binary that already existed while this branch deleted one.
`docs/status.md` and `postgres_smoke.rs` merged without conflict and were
checked line by line rather than trusted: #44's `migrated_postgres_with_url`
and its whole drift test are intact, and so are this branch's three
`AssertSqlSafe` wrappings, its `authkestra.oauth_dpop_jti` row and its
migration-0013 column assertions.

On the rebased tree, `just ci` end to end:

```
verify: ok — the ten gates above passed; the verify-docs report is advisory
    Starting 1279 tests across 41 binaries
     Summary [ 676.578s] 1279 tests run: 1279 passed, 0 skipped
verify-ignored: 0 ignored (expected 0), 41 test binaries (expected 41), 1279 total (minimum 1080)
advisories ok, bans ok, licenses ok, sources ok
JUST_CI_EXIT=0
```

Run **twice** on this tree, before and after these paragraphs were written —
the second run is the one on the commit this branch pushes, and reported the
same three numbers. The `postgres_smoke` case #44 added is in there by name
(`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`, PASS),
which is worth checking rather than inferring from the total: it shells out to
the `cratestack` CLI and _fails_ rather than skipping when that binary is
absent, so a green run is evidence the tool ran.

1279 is `8d907f9`'s 1271 minus `authkestra_op_smoke.rs`'s 3, plus
`merchant_token_flow` case (i), `op::refusing_stores`' 4 and
`vpay_db::sql_audit`'s 6. `just test-doc`: **90 passed, 0 failed, 1 ignored**,
unchanged. Web: vitest across 8 packages, all passing. Both mutations were
re-run on the rebased tree and both still fail closed — re-enabling
`sqlx-postgres` on `authkestra-op` reds `cargo deny check bans`, and a
positional `{}` interpolation of a caller's value in `vpay-db` reds
`vpay_db::sql_audit`. Image: `docker buildx build -f backends/Dockerfile
--target server .` on a private builder (`vpay-exp12b-land`, removed
afterwards), `--version` prints `vpay-server 0.1.0`, image **16 MB**.

---

### The three OP stores that pinned sqlx 0.8 (2026-09-05)

**What changed.** `vpay_api::op::MerchantOp::new` filled three of
`CompositeOpStore`'s slots — `AuthorizationCodeStore`, `RefreshTokenStore`,
`DeviceCodeStore` — with
`authkestra_op::sqlx_store::SqlxOpStore<sqlx::Postgres>` over
`Repositories::op_store_pool()`. They now hold
`vpay_api::op::refusing_stores`' `RefusingAuthorizationCodeStore`,
`RefusingRefreshTokenStore` and `RefusingDeviceCodeStore`: twelve async
methods, no SQL, every one returning `Err(OpError::GrantTypeNotPermitted)`
after logging an `UnservedGrantError` that names the grant and the method.
`authkestra-op`'s `sqlx-postgres` feature came off both manifests that
enabled it (`vpay-api`, `vpay-tests-integration`), and `sqlx` moved back to
`[dev-dependencies]` in `vpay-api` — nothing under that crate's `src/`
outside a `#[cfg(test)]` module names an `sqlx` type any more.

**Why.** `SqlxOpStore` is behind a feature that pins `sqlx ^0.8`, and it was
the **only** reverse dependency of that major left in the workspace
(`cargo tree -i sqlx@0.8.6` named `authkestra-op` and nothing else). Three
slots that no request can reach were holding every crate here back from sqlx
0.9 — which is what `cratestack-sqlx 0.11.1` requires, and therefore what
stands between this repository and using CrateStack in a vpay transaction at
all. Bumping authkestra does not help: `authkestra-op` 0.8.1 (published
2026-09-05) **deletes** `src/sqlx_store.rs` and moves it to
`authkestra-store-sqlx`, which is itself still `sqlx ^0.8`. The measurement
behind all of this is `docs/plans/exp12-notes/opus.md` on branch
`claude/exp12-sqlx09-opus`; this pass is its sequel.

**Why refusing rather than implementing.** The alternative was for vpay to
own the OP's code/refresh/device storage on its own pool: roughly four
hundred lines of ported SQL, three of whose methods (`consume_code`,
`consume_token`, `consume_device_code`) must be atomic compare-and-swap or
they _are_ an authorization-code replay vulnerability — written for grants
this deployment refuses at the door. Storage nothing reads is not safer than
no storage. Fail-closed is also the pattern `authkestra-op` applies to its
own optional seams (`NoClientAssertionStore`, `NoDpopReplayStore`), and
0.7.1 ships **no** such type for these three traits (checked against the
extracted crate source, not a changelog: `NoClientAssertionStore` and
`NoDpopReplayStore` are the only two, and the only other implementations of
the three traits are `SqlxOpStore`, `redis_store`, a `KvStore` blanket impl
and the crate's own test doubles).

**This is not a stub reachable from a shipping binary** (AGENTS.md rule 1).
The distinction is `Ok(None)` versus `Err`. An "always empty" store answers
`Ok(None)`, every `authkestra_op` handler renders that as `invalid_grant` —
"your code was wrong" — and the day another grant is mounted it becomes a
silent lie. These answer `Err` from all twelve methods, so the first request
on a newly mounted grant fails loudly with a message naming the grant.
`verify-no-mocks` is unchanged and green.

**Proof that they are unreachable, measured rather than argued.**
`backends/tests/integration/tests/merchant_token_flow.rs` case (i),
`the_three_grants_vpay_does_not_serve_are_refused_before_any_store`, POSTs
`authorization_code`, `refresh_token` and
`urn:ietf:params:oauth:grant-type:device_code` to a real token endpoint on a
real socket with a real freshly minted `private_key_jwt` assertion and every
field the grant's own handler would need, and asserts all three come back
**`unauthorized_client` / HTTP 400**, and specifically not `server_error`.
Every authkestra grant handler renders a store error as `server_error`, so
"the stores are unreachable" and "a merchant is never told `server_error`
here" are the same assertion. **Corrected 2026-09-05 by the review of this
branch:** this paragraph used to read "never 500 — a store error would be a
500". That is wrong, and wrong in the direction that matters, because it made
the _status_ look like the proof. `op::token::token_error_status` maps
everything except `invalid_client` to **400**, `server_error` included, and
`only_invalid_client_answers_401` asserts exactly that — so this endpoint has
no 500 to emit and a store error would have arrived as a 400 as well. The
`error` code is the whole distinction; the test always asserted it, and the
prose around it was describing a different system. **The refusal shape was
measured on the unmodified tree first** (with `SqlxOpStore` still wired) and
is byte-for-byte the same afterwards; it is `unauthorized_client`, not the
`unsupported_grant_type` one might expect, because each handler's _first_
statement is `client.allows_grant_type(..)` and a merchant registration can
only ever declare `client_credentials`
(`vpay_config::ConfigError::DisallowedMerchantGrant`). The one grant `/v1`
does serve provably cannot reach a store either:
`handle_client_credentials` is not passed the `op_store` at all
(`authkestra-op-0.7.1/src/handlers/token.rs`).

**An amendment to ADR-0010 and ADR-0009, recorded here because an ADR is
immutable.** ADR-0010 describes the merchant OP's store as the composite of
a YAML client store and authkestra's SQL stores; ADR-0009 §"acceptance"
names `authkestra_op_smoke.rs` as the evidence that migrations `0006`/`0013`
match `SqlxOpStore`'s hardcoded DDL. Neither _decision_ is reversed — clients
still come from YAML, `client_credentials` is still the only grant, and the
`authkestra.*` tables still exist exactly as transcribed. What changed is
that vpay no longer constructs the store those two documents assume, so the
three grant slots are fail-closed and that acceptance test is gone. Both
ADRs stand as written; this paragraph is the amendment.

**What was given up, plainly.** `authkestra_op_smoke.rs` (3 tests) was
deleted: it drove `SqlxOpStore`'s own hand-built SQL against
`0006`/`0013` — `find_client` decoding `token_endpoint_auth_method`/`jwks`,
`store_token`/`get_token` round-tripping `jkt`,
`check_and_record_dpop_jti` refusing a replay — and it could not compile
without the feature. It proved a property of a type this system no longer
uses. `postgres_smoke.rs` still asserts the four `authkestra.*` tables exist
and that `oauth_codes.client_id`'s foreign key fires — **and, added by the
review of this branch on 2026-09-05, `authkestra.oauth_dpop_jti` and the
three columns 0013 adds (`oauth_refresh_tokens.jkt`,
`oauth_clients.token_endpoint_auth_method`, `oauth_clients.jwks`).** Those
five names were the part of the deleted file that nothing else covered: after
the deletion no test in the repository mentioned any of them, so a later
migration could have dropped or renamed one with the whole gate staying
green. What is genuinely gone with the store, and is _not_ replaced, is the
store-versus-schema agreement — that `SqlxOpStore`'s hand-built
`INSERT … ON CONFLICT`, `SELECT … jkt` and `find_client` JSONB decoding match
this DDL. That claim has no owner now and cannot get one while nothing
constructs the store. The four `authkestra.*` tables are now **unread and
unwritten by any vpay code path**; dropping them needs a new migration and is
left to the maintainer.
The header comments in `0006` and `0013` still cite the deleted file and were
**deliberately left wrong** — `sqlx::migrate!` checksums each migration's
whole file content, so editing an applied one turns the next boot into a
version mismatch.

**Reserved for the maintainer.** Asking authkestra upstream to move
`authkestra-store-sqlx` to sqlx 0.9 is an external issue or PR for the
maintainer to file, not for an agent. It would let a future pass put a real
SQL-backed store back in these slots if `/v1` ever mounts one of the three
grants.

**Counts on this change.** `just verify-ignored`: **1272 total, 41 test
binaries, 0 ignored** — 1270/42 on `d086084`, minus `authkestra_op_smoke`'s
3, plus `merchant_token_flow` case (i) and `op::refusing_stores`' 4 units.
`expected_suites` moved 42 → 41 in this commit, with the reasoning in the
justfile; `min_tests` stays 1080. `just test-doc`: **90 passed, 1 ignored**
(86 + the four examples on `refusing_stores`; the ignored one is
`sdks/rust`'s README block, pre-existing).
