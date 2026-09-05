# exp12b (opus arm) — sabotage review of `claude/exp12b-sqlx09-opus`

Date: 2026-09-05. Reviewer working in the same worktree, on the same branch,
base `d086084`, implementation `a8d6fb6..e2deea4`. Host: the authoring machine,
rootless Docker, `CARGO_BUILD_JOBS=4`, Node 22.23.2 via nvm, other agents
running container tests concurrently (33 containers up at the start of this
review). Every figure below was produced here.

The implementer's account is [`opus-b.md`](opus-b.md). This file records what
that account got right, what it got wrong, and what was changed as a result.

## 0. The verdict

**Safe as delivered: no — but narrowly, and not in the way that word usually
means.** The change itself is sound: the fail-closed stores are correct and
genuinely unreachable, the sqlx bump is clean, and `just ci` was green end to
end as delivered (1277/1277, 0 skipped, 0 ignored, 41 binaries, 90 doctests,
deny green). Nothing was papered over and nothing was overclaimed about the
*feature*.

What was not safe was the **new safety net**. The injection audit that the
whole sqlx 0.9 story rests on — the one thing standing behind 36
`AssertSqlSafe` call sites — had a hole big enough for a live SQL injection to
walk through, and the mutations it was proven against all missed it because
they all spelled the injection the same way. That is the finding this review
exists for; everything else is documentation.

Four commits of remediation, then the gate re-run in full.

## 1. Findings

| # | Severity | Finding |
|---|---|---|
| F1 | **correctness (gate)** | `vpay_db::sql_audit` was blind to positional `{}` captures. `format!("SELECT {COLUMNS} FROM charges WHERE payment_intent_id = '{}'", payment_intent_id)` in `charges::get_for_intent` — a live injection through `AssertSqlSafe` — passed all five tests. Fixed; the mutation now fails the gate. |
| F2 | misleading-claim | Four places said a store error "renders as `server_error`/500" and that a 400 therefore proves the stores are unreachable. `op::token::token_error_status` maps everything but `invalid_client` to **400**, and `only_invalid_client_answers_401` asserts that for `server_error` by name. The test was always right; the reasoning printed around it was not. |
| F3 | correctness (gate) | Nothing gated "one sqlx major", the entire point of the change. Restoring `sqlx-postgres` on `authkestra-op` resolved 0.8.6 beside 0.9.0 with `cargo deny check` still printing "bans ok" and `just ci` green. `deny.toml` now denies `sqlx`/`sqlx-core` `< 0.9`. |
| F4 | correctness (coverage) | Deleting `authkestra_op_smoke.rs` left `authkestra.oauth_dpop_jti` and migration 0013's three added columns named by **no test at all**. Re-covered in `postgres_smoke.rs`. The store-versus-schema *agreement* is genuinely gone and is now recorded as having no owner. |
| F5 | misleading-claim (nit) | "the list above is ten cases now, not seven" — the list is seven bullets, the file is ten cases, and the introducing paragraph still said seven. |
| F6 | nit | The bump's `cargo tree -d` cost was under-reported: sqlx 0.9 brings the RustCrypto 0.11 generation in beside the 0.10 one, so six crates are newly duplicated (14 duplicate warnings now, 8 before) and two SHA-2 implementations compile into `vpay-server`. Recorded in `docs/status.md` with F3. |

Nothing was found in the categories that would have been worst: **no `#[ignore]`
was added** (`verify-ignored` reads 0, expected 0), **no `#[allow]` was added**
anywhere in the diff, no test asserts nothing, no gate was weakened, no mock or
fake reaches a shipping binary (`verify-no-mocks` green), and the counts in
`justfile`/`docs/status.md` moved in the same commits as the code that moved
them.

### F1, in full, because it is the one that mattered

`sqlx::AssertSqlSafe`'s contract is "the caller audited this string". The
branch's best idea was to make that audit a test rather than a comment, and the
test was written carefully — a balanced-paren scanner, controls on the scanners
themselves, an exact site count, a checked allowlist, and three recorded
mutations. All three mutations interpolate **by name**:
`{payment_intent_id}`. `interpolations()` silently discarded a capture with no
name, so the *positional* spelling of the same injection was invisible:

```
$ # charges::get_for_intent, mutated to interpolate the caller's value as `{}`
$ cargo nextest run -p vpay-db -E 'test(/sql_audit/)'
    Summary [0.021s] 5 tests run: 5 passed, 86 skipped
```

Five green tests over a statement reading
`WHERE payment_intent_id = '<caller string>'`. The named spelling was caught;
this one was not, and it is the spelling a rushed edit is *more* likely to
reach for.

`interpolations` now reports an unnamed capture as `POSITIONAL_CAPTURE`, which
is in neither the constant set nor the allowlist and so is a violation on
sight. Same mutation, after the fix:

```
        FAIL [0.006s] vpay-db sql_audit::tests::every_interpolation_into_a_statement_is_a_crate_constant
    charges.rs: a statement uses a positional `{}` capture, whose value comes
    from the argument list and cannot be checked here. …
```

`a_positional_capture_is_reported_and_is_neither_a_constant_nor_allowed` pins
the scanner over synthetic text, including that `{{}}` is still an escape that
yields nothing — the one property the fix could plausibly have broken.

There is a **second, narrower gap left open and stated rather than fixed**: the
audit is scoped to `format!`. A statement assembled by `String::push_str` or
`+` and bound to `sql` would satisfy `every_assert_sql_safe_wraps_the_variable_the_audit_covers`
and be invisible to the constant check. No such statement exists in `vpay-db`
today (all 36 are `format!`), and closing it properly needs a different
technique than source scanning. Recorded here rather than papered over.

## 2. The mutation table

| # | Mutation | Expected | Observed | Verdict |
|---|---|---|---|---|
| M1 | `charges::get_for_intent` interpolates `payment_intent_id` as a positional `{}` | `sql_audit` fails | **5 passed** — silent | **hole (F1)**; after the fix, FAIL with the file and the reason |
| M2 | `RefusingAuthorizationCodeStore::consume_code` returns `Ok(None)` | store unit test fails, grant integration test still passes | `every_method_of_every_slot_refuses` **FAILED**; `the_three_grants_vpay_does_not_serve_are_refused_before_any_store` **PASSED** (11.9 s, real container) | as designed — both halves of the net exist |
| M3 | one `AssertSqlSafe` reverted to `&sql` (`jobs::claim`) | compile error | `error[E0277]: dynamic SQL strings should be audited for possible injections … the trait SqlSafeStr is not implemented for &String` | as designed |
| M4 | `sqlx-postgres` restored on `authkestra-op` | two sqlx majors; is anything red? | `cargo tree -d`: `sqlx v0.8.6` + `sqlx v0.9.0`, `sqlx-postgres` at both. `cargo deny check bans`: **"bans ok"**. Nothing red. | **no gate (F3)**; after the fix, `error[banned]: crate 'sqlx = 0.8.6' is explicitly banned`, `bans FAILED`, exit 2 |
| M5 | `postgres_smoke`'s new column check renamed to `jkt_not_a_column` | fails against real Postgres | FAILED, naming the column | new check is live, not tautological |

## 3. What was verified rather than taken on trust

* **authkestra-op 0.7.1 ships no `No*Store` for the three traits.** Confirmed
  from the extracted crate in the local registry: `lib.rs` re-exports
  `NoClientAssertionStore` and `NoDpopReplayStore` and nothing else of that
  shape; the only `impl AuthorizationCodeStore/RefreshTokenStore/DeviceCodeStore
  for` outside blanket impls and test doubles are `SqlxOpStore`'s. Hand-written
  types were the right call, not duplication.
* **All three grant handlers check `client.allows_grant_type` as their first
  statement** (`token.rs:828`, `1370`, `627`), and every store `Err` arm in all
  three renders `server_error` without inspecting the variant — so
  `OpError::GrantTypeNotPermitted` versus `OpError::Storage` is indeed an
  operator-facing choice only, exactly as claimed.
* **`handle_client_credentials` takes no `op_store`**, so the one served grant
  cannot be routed to a store by any configuration. `handle_token`'s dispatch
  has five arms; the token-exchange one is refused by
  `config.token_exchange_enabled == false` before any store call, and the
  custom-grant arm by `allows_grant_type` before any store call.
* **vpay mounts only `handle_token`** — no `authorize`, `userinfo`,
  `device_authorization` or enrolment handler is reachable, so no other path
  into the three stores exists.
* **The 36 `AssertSqlSafe` sites, re-derived independently** with a balanced
  scanner written for this review: 36 sites, captures are exactly
  `{COLUMNS}`/`{OPEN}`/`{LIVE_CHARGE_STATES}`/`{SETTLEABLE_STATUSES}`/
  `{CLAIM_RETURNING}`/`{PREVIOUS_STATE}`/`{columns}`/`{direction}` and nothing
  else. No positional capture exists today — F1 is about what the gate would
  let through tomorrow, not about a live injection on this branch.
* **`Cargo.lock` is sane**: 477 → 469 packages, and every entry that moved is
  sqlx, its transitive tree, or the RustCrypto/`whoami`/`flume`/`hashlink`
  crates that tree pulls. No unrelated bump.
* **`cargo tree -i aws-lc-rs`** — "did not match any packages".
* **Decisive negative, re-measured**: a scratch crate outside the workspace on
  `cratestack-sqlx = "0.11.1"` **and** `vpay-db` by path resolves one sqlx
  major (`sqlx 0.9.0`, `sqlx-core 0.9.0`, `sqlx-postgres 0.9.0`) and
  `cargo check`s in 26.19 s. Scratch crate deleted.

## 4. Classification of `UnservedGrantError`, since the brief asked

`Category::NotImplemented` is right, and the question is slightly academic:
the error is **never rendered to a caller**. It is logged at `error!` and the
store returns `OpError::GrantTypeNotPermitted`, which every grant handler turns
into `server_error` — vpay's own error envelope is never involved.

If it *were* rendered through `vpay_core`'s path it would be **501
`not_implemented`**, `api_error`, `Retry::Never`, `Severity::Error`. That is the
right shape for "a capability this deployment has never offered was asked for":
a 4xx would say the caller made a fixable mistake, and the caller's actual
mistake (asking for a grant it is not registered for) is already refused
upstream with `unauthorized_client`. Reaching this error means an invariant of
the OP assembly broke, and `Severity::Error` is what wakes someone. `Internal`
(500, `Severity::Page`) was the other defensible choice and the implementer's
reason for declining it is sound.

## 5. The gate, re-run in full after the fixes

`just ci`, end to end, on this machine, after all four remediation commits:

```
fmt-check                      cargo fmt --all -- --check          (silent)
clippy                         --workspace --all-targets -D warnings (clean)
verify                         ok — the ten gates
test-rust    Starting 1278 tests across 41 binaries
             Summary [741.013s] 1278 tests run: 1278 passed, 0 skipped
test-doc                       90 passed, 0 failed, 1 ignored
verify-ignored                 0 ignored (expected 0), 41 test binaries
                               (expected 41), 1278 total (minimum 1080)
lint-web / test-web            8 packages, all passing
deny                           advisories ok, bans ok, licenses ok, sources ok
JUST_CI_EXIT=0
```

The one ignored doctest is `sdks/rust`'s README block and is pre-existing.
1277 → 1278 is F1's regression test, in the same binary, so `expected_suites`
stays 41.

Named tests, run again on their own after the fixes —
`merchant_token_flow`, `postgres_smoke`, `worker_kill9`, `worker_recovery`,
every `idempot*`, `one_charge_per_intent`, every `claim` case,
`refusing_stores` and `sql_audit`:

```
    Summary [176.467s] 105 tests run: 105 passed, 1173 skipped
```

Image: `--target server` on a review-owned buildx builder (`vpay-exp12b-opus-review`,
removed afterwards; the shared default builder was not touched),
`docker run --rm … --version` prints `vpay-server 0.1.0`, **16 MB** — the same
number the implementer reported, confirmed independently.

## 6. Reserved for the maintainer

Unchanged from the implementer's list, plus one:

* **Whether `[bans] multiple-versions` should stop being `warn`.** This review
  turned one specific duplication (`sqlx`) into a hard deny and left the
  general setting alone, because the fourteen remaining duplicates are
  upstream's to converge and gating this repo on other people's release
  schedules is a policy decision, not a review's.
