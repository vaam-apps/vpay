# Verification log — 2026-09-13, ADR-0018's cross-tenant admin role (review pass)

Last verified 2026-09-13 on branch `claude/dash-admin-role-review`, an
adversarial review of `claude/dash-admin-role-adr`
(`7a48c5a11ee8b6611cbfe997b89548bcc17a5cf0`). The implementing pass stopped
before committing and **before its `just ci` ever finished**, so its gate
evidence was treated as absent rather than green. Every number below was
measured in this pass; each command's exit code is read from a file rather
than from a harness banner.

The environment: `rust-toolchain.toml`'s pinned `1.98.0` (`rustc 1.98.0
(88d9e12ae 2026-08-18)`), Node from `.nvmrc` (`v22.23.2`, not the host's
`v24.20.0`), rootless Docker at `unix:///run/user/1000/docker.sock`. The
container-backed suites **ran** — executed and ignored counts are given
separately below, because a suite that skips on an absent Docker still
prints `ok`.

## The regression the implementing pass left, and how it was found

Migration `0043` adds `staff_members.is_admin BOOLEAN NOT NULL` and then
**drops the `DEFAULT`**, following migration `0035`'s rule for every column
on this table. That rule is right and this pass did not touch it. What it
costs is a thing no compiler can see: `vpay_db::Staff::create` is not the
only writer of `staff_members`. `postgres_smoke.rs` seeds that table with
**five hand-written `INSERT`s** of its own — they exist because their
subject is a database constraint, and a repository call that refused the row
first would prove nothing about it — and none of the five named the new
column.

Measured before the fix, with `-E 'test(staff) or test(cascade) or
test(session_token) or test(plain_pkce)' --no-fail-fast`:

```
Summary [ 15.276s] 4 tests run: 0 passed, 4 failed, 35 skipped
  FAIL a_half_enrolled_staff_member_is_refused_by_the_database
  FAIL a_session_token_without_its_expiry_is_refused_by_the_database
  FAIL an_authorization_code_with_a_plain_pkce_method_is_refused_by_the_database
  FAIL signing_out_cascades_onto_a_code_in_flight
```

with, in every one of them:

```
null value in column "is_admin" of relation "staff_members" violates not-null constraint
```

The first of those is the one worth reading twice. It _expects_ a rejection
and asserts which constraint produced it, so the new `NOT NULL` made it
report `staff_members_totp_is_paired` as **missing** when that CHECK was
fine:

```
assertion `left == right` failed: the rejection must come from the pair CHECK specifically
  left: None
 right: Some("staff_members_totp_is_paired")
```

Fixed by naming `is_admin, … false` in all five `INSERT`s — **not** by
giving the column a `DEFAULT`, which would have weakened the migration's own
decision to satisfy a test. After the fix: `4 tests run: 4 passed, 35
skipped`. `postgres_smoke.rs`'s module doc now records that this file is a
second writer of `staff_members` and that the next migration adding a column
there should `grep "INTO staff_members"` first.

## The second regression, which a narrower run would have missed

Scoping the first `postgres_smoke` run to the staff-shaped tests was itself a
mistake, and the full `just ci` is what caught it:
`schema_migrates_cleanly_on_an_empty_database` pins the number of applied
migrations, and migration `0043` makes it forty-three:

```
assertion `left == right` failed: all forty-two migration files ... should be recorded as applied
  left: 43
 right: 42
```

Bumped to 43, with the `0043` clause appended to the assertion's own running
description of every migration, as that message has carried for every
migration before it. The whole suite then runs clean: `39 tests run, 39
passed, 0 skipped`. The lesson is recorded rather than quietly fixed — a
filtered nextest run is not evidence about a file, only about the filter.

## The drift measurement the commit pointed at but did not carry

`0043`'s own DRIFT paragraph tells a reader to see `postgres_smoke.rs`'s
`EXPECTED_DRIFT_CHANGES` "for the measurement this migration's commit
carries" — and the commit did not touch that file at all, so the pointer led
nowhere. `docs/status/cratestack/drift.md` asserted the measurement in
prose.

It was taken. `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount`
against a freshly migrated `postgres:16-alpine`, exit 0:

```
drift detected in 24 table(s)/view(s) (179 change(s) total)
19 column(s) have a Postgres type cratestack could not confidently map …
test result: ok. 1 passed; 0 failed; 0 ignored; 38 filtered out
```

**All three constants held** — `EXPECTED_DRIFT_CHANGES` 179,
`EXPECTED_DRIFTED_RELATIONS` 24, `EXPECTED_UNMAPPABLE_COLUMNS` 19. The
implementing pass's prediction from shape (a `BOOLEAN` with no CHECK, no
index and no `@default` costs nothing, as `password_change_required` already
proved on this table) was **correct**. The three constants now carry the
"Still … after migration 0043" notes this repository writes for every other
migration, so the migration's pointer resolves.

One environment caveat, stated rather than hidden: the `cratestack` CLI on
this host is **0.11.1** and the repository pins **0.12.0**, so
`just check-schema` and the drift test both warn and both ran against the
0.11.1 grammar. `is_admin Boolean` is nevertheless type-checked under
0.12.0 by a different route that is not optional — `vpay-db`'s private
`mod schema` compiles `schemas/vpay.cstack` through `cratestack-macros`
`0.12.0`, and `cargo build --workspace --tests` exits 0.

## The six decisive checks, by mutation

Baseline first: `dashboard_read_surface` is **21 tests run, 21 passed, 0
skipped** (98.1s — the containers ran). Each mutation below was applied,
the suite re-run, and the mutation reverted; `grep -r "LANE-B-REV MUTATION"`
over `backends/` returns nothing on the committed tree.

**M1 — delete the `is_admin` check** (`if staff.is_admin` → `if true`), so
a non-admin's `?merchant_id=` is honoured. `19 passed, 2 failed`:

- `a_non_admin_cannot_move_the_tenant_with_the_query_parameter` — RED, and
  the failure body is the breach itself: the non-admin's list came back
  holding `pi_dash_nonadmin_b`, the other tenant's row.
- `the_admin_flag_defaults_to_non_admin` — RED, same way.

So the boundary is genuinely tested: deleting the one check that enforces it
reddens the suite.

**M2 — delete the write refusal** (`required_scope`'s `_ => None` →
`Some(scope)`), so a non-`GET` reaches the route table. `19 passed, 2
failed`:

- `an_admin_still_cannot_write` — RED (`403` expected, `405` observed).
- `a_write_method_is_refused_by_the_boundary_not_by_the_route_table` — RED.

The `403`-not-`405` distinction `dash::required_scope`'s doc comment argues
survives, **and it is proven for an admin specifically**, not inferred from
the non-admin case.

**M3 — make "not yours" distinguishable from "does not exist"** (on a miss,
re-read the id unscoped and answer `403` if it exists anywhere). `18 passed,
3 failed`:

- `another_merchants_intent_is_indistinguishable_from_one_that_never_existed`
  — RED (the pre-existing non-admin property).
- `a_non_admin_cannot_move_the_tenant_with_the_query_parameter` — RED (its
  uniform-404 half, under the override parameter).
- `an_admin_reads_a_merchant_other_than_its_own` — RED.

The uniform `404` is guarded for both roles, not asserted.

## `staff add`, and what a row without `--admin` actually holds

`cargo nextest run -p vpay-server --test cli -E 'test(staff_add)'`, exit 0:
**5 tests run, 5 passed (1 slow), 33 skipped**, including
`staff_add_admin_flag_defaults_to_false_and_admin_sets_it`, which runs the
real binary twice against a live Postgres and reads both rows back through
`vpay_db::Staff::find_by_email` — not off stdout, where `is_admin` correctly
does not appear.

That, together with the four repaired `postgres_smoke` cases, is also the
answer to "does `create` name the column": every one of those inserts now
succeeds against a table whose `is_admin` is `NOT NULL` with no `DEFAULT`,
which it could not do if `CreateStaffMemberInput` were not carrying the
field.

## The seam

`vpay_api::dash::DashboardTenancy` is a real, publicly exported two-variant
enum (`Bound`/`ChosenByAdmin`) with `merchant_id()` and `is_cross_tenant()`,
inserted into the request extensions — not an inline `bool` at one call
site. It is what the plan's Lane C asks for.

One gap this pass closed: **nothing ran its `FromRequestParts` impl.** No
`/dash/v1` handler reads a `DashboardTenancy` yet (Lane C is what will), so
the extractor — including its fail-closed arm — was code the suite never
executed, and Lane C would have been its first caller.
`the_seam_is_extractable_and_fails_closed_when_it_is_absent` now asserts
both directions: absent → `ApiError::Internal`, never a defaulted tenant;
present → the value a handler reads back.

`is_cross_tenant()` still has **no production caller**. The cross-tenant
audit line ADR-0018 § "blast radius" requires is emitted, but from inside
`ChosenByAdmin`'s own branch in `require_dashboard_token` rather than
through the predicate. That is accurate to what the ADR claims and is left
as-is; it is named here so Lane C's author knows the predicate is a contract
with one unit test behind it and no caller.

## Gates

`just verify` — **exit 0**, read from
`lane-b-rev-verify-exit.txt`: the twelve gates pass, `verify-docs` advisory
report printed. `just ci` — see `lane-b-rev-exit.txt` and
`lane-b-rev-ci.txt` in the branch's run, and the summary in this pass's
report.

Two `just ci` attempts before the reported one failed for reasons that were
**not** this branch's code, and both are recorded rather than quietly
retried:

- `fmt-check-web` exit 254, `Command "prettier" not found` — this worktree
  had no `node_modules`. Fixed by `pnpm install --frozen-lockfile` under
  `.nvmrc`'s `v22.23.2`, exit 0.
- `test-rust` exit 100, one **real** failure —
  `schema_migrates_cleanly_on_an_empty_database`, the pinned migration count,
  above. That one is this branch's code and is fixed here.
- `test-rust` exit 100, one infrastructure failure —
  `vpay-db config_reconcile::tests::a_provider_reads_through_cratestack_exactly_as_it_does_through_sqlx`,
  `failed to create a container: Timeout error` after 120s. A rootless-Docker
  container-creation timeout under contention from another agent's build on
  the same host (15 containers sat stuck in `Created`), not a code failure,
  and the same test passes on a re-run. The earlier drift run hit the
  identical timeout once and passed on retry.

## What this pass did NOT do

- **It did not change ADR-0018's decisions**, only appended measured
  evidence to `docs/status/cratestack/drift.md`. Every decision in the ADR
  stands as the implementing pass wrote it.
- **It did not edit migration `0043`.** A first draft of this review added
  the "five hand-written inserts" warning to the migration's own comment and
  then reverted it: `backends/migrations/README.md`'s one rule is that a
  migration file is never edited, and `just migrations-manifest` refuses to
  rewrite a manifest line by design. The warning lives in
  `postgres_smoke.rs`'s module doc instead. Whether an _unmerged_ migration
  may still be edited before it ships is a maintainer's call, not this
  pass's.
- **It wrote no new `/dash/v1` route** and consumed the seam from no
  handler. Lane C is where that belongs.
- **It did not close `is_cross_tenant()`'s missing caller**, above.
- **It did not re-run `just docs-check-citations`** (needs the network) or
  `just helm-check` (same), neither of which is in `just ci`.
