# exp54 review — the procedure held, its paging arithmetic and one report did not

Sabotage review of `claude/exp54-payments-procedure` at `695c220` (PR #117),
from a separate worktree on `claude/exp54-review`. Date 2026-09-11.

**The security surface held under every mutation it was attacked with.** The
tenancy predicate, the limit clamp and the status refusal are all real, all
observable through a test, and each test fails by the name the branch
predicted. What did not hold is arithmetic at the edges of the page envelope,
one claim about page size that the code contradicted inside its own comment,
a vocabulary with no test behind it, and a report that could not see the
`#[allow]` this branch added.

The branch's most important property is also the easiest one to lose: it
says, in four places and in plain words, that **nothing serves this
procedure**. That survived the review intact and is not softened anywhere
below. `docs/status.md` leads its row with "A tested library call with no
caller"; `docs/flows/dashboard.md` says `GET /dash/v1/payment_intents` "is
unchanged and is still the only payments list the dashboard can reach";
`schemas/vpay.cstack` says "do not read the presence of this declaration as a
shipped endpoint"; the module header says the tests are the only callers.
Checked independently: `vpay-db` takes `cratestack-pg` with
`features = ["postgres"]` and no axum feature, so there is not even a
generated router compiled, let alone mounted.

Nothing in this review weakens a test. Four tests were strengthened and one
was added.

---

## The mutations, run rather than described

Each was applied to the reviewed tree, run under
`cargo nextest run -p vpay-db -E 'test(/search_payment_intents/)'`, and
reverted. **Baseline first: 12 tests run, 12 passed, 0 ignored, 0 skipped** —
including the container test, which starts a real `postgres:16-alpine`
(1.36 s) rather than being skipped. A skipped container test would have made
every row below meaningless.

| #   | mutation                                                                          | result                                                                                                                                                                                                                                                              |
| --- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `WHERE merchant_id = $1` → `WHERE ($1::TEXT IS NOT NULL)`                         | **2 RED.** `against_postgres::the_page_is_the_tenants_own_rows_filtered_and_bounded` — `merchant_a`'s page came back `["pi_b_only", "pi_a_new", "pi_a_old"]`, the other tenant's payment first. Also `the_statement_selects_no_credential_and_no_jsonb_column`.     |
| 2   | `resolve_page(args.page)` → `(limit.unwrap_or(DEFAULT), offset.unwrap_or(0))`     | **1 RED.** The same container test, with Postgres' own `database: OFFSET must not be negative`. `a_hostile_limit_is_clamped_before_it_reaches_postgres` stays **green**, exactly as the branch predicted — it pins the helper's contract, not the body's use of it. |
| 3   | `validated_status`' vocabulary check → `if true`                                  | **1 RED.** `an_unknown_status_is_refused_rather_than_answered_with_an_empty_page`, on `Some("succeded")` — the typo answered as a filter instead of a refusal.                                                                                                      |
| 4   | this review's own `(offset == 0).then_some(0)` arm, deleted                       | **2 RED.** `the_envelope_reports_the_page_it_actually_has` (`left: None`, `right: Some(0)`) and the container test's `canceled` page.                                                                                                                               |
| 5   | this review's own `Some(page.limit.unwrap_or(DEFAULT_PAGE_LIMIT))` → `page.limit` | **1 RED.** `a_hostile_limit_is_clamped_before_it_reaches_postgres`, `limit=None offset=None`: `left: (100, 0)`, `right: (10, 0)` — the 10× divergence as a number.                                                                                                  |
| 6   | `allow_sites` reverted to its four bare needles                                   | **1 RED.** `a_lint_silenced_through_cfg_attr_is_counted_and_other_cfg_attrs_are_not`, `left: []`, `right: [1, 11]`.                                                                                                                                                 |
| 7   | `payer_limit_reached` deleted from `enum FailureCode` in `schemas/vpay.cstack`    | **1 RED.** `the_two_failure_code_vocabularies_are_one_vocabulary`: `payer_limit_reached: unknown enum variant … expected one of: insufficient_funds, payer_timeout, …` — the drift named, which is what nothing in the workspace would have noticed before.         |

**Rows 1–3 are the branch's own claimed mutations and all three fire**, on the
reviewed tree _after_ this review's changes — so nothing here was weakened to
make a mutation pass. **Rows 4–7 attack this review's own fixes**, so they are
not asserting themselves back to the implementation either. Row 7 in
particular is the mutation the new vocabulary test claims to be decisive
against, run rather than asserted.

### The tenancy predicate, stated plainly

**Deleting it makes a named test fail.** The tenancy tests are not asserting
the implementation back to itself: the container test seeds `merchant_a` with
two intents and `merchant_b` with one, and with the predicate removed
`merchant_b`'s row appears in `merchant_a`'s page. That is the leak observed
as rows, not as a proxy. The statement-text test fires as a second,
independent witness.

### The `#[allow]` report, measured both ways

The report is a report — it exits 0 whatever it finds — but the change to it
is a change to a gate-adjacent tool and carries that burden of proof:

| scanner                        | `#[allow]` / `#[expect]` sites                                                           |
| ------------------------------ | ---------------------------------------------------------------------------------------- |
| as shipped (four bare needles) | **6** — `vpay-db/src/schema.rs:49` absent                                                |
| after this review              | **7** — that line present, spelled `#[cfg_attr( not(test), allow( dead_code, reason = …` |

**No other previously-invisible allow surfaced.** The six entries are
byte-identical between the two runs; the only difference is the seventh. The
count moved because this branch added a silenced lint, not because the
scanner started over-reporting.

---

## Findings

### 1. `total_count` said "unknown" where it could say "none" — **fixed**

**Medium.** `page_of` read the window count off the first returned row and
answered `None` when there was no row. That is right for a page past the end
of the set, which is what the branch documented. It is wrong for an **empty
first page**, and that is the common case on this surface: an operator
filtering by a status they happen to have none of.

The statement asks for `limit + 1` rows starting at the first one, so at
`offset == 0` an empty result is not "the count fell off the page" — it is
proof the filtered set is empty. `Some(0)` is provable there, and `None` is
strictly less information. On a payments list the difference is between a
table that says "0 payments" and one that says nothing at all.

The branch's own comment made the opposite argument — `None` "rather than
`0`, which would be a lie about a set that has rows in it" — which is true
only past the end, and was applied to both cases.

Fixed in `page_of` with an `(offset == 0).then_some(0)` arm, pinned by a new
case in `the_envelope_reports_the_page_it_actually_has`, and by the container
test's `canceled` page, which now asserts `Some(0)` rather than `None`.
Deleting the arm fails the unit assertion and nothing else, which is the
shape a single-purpose test should have.

### 2. Offset paging's window shift was documented nowhere — **fixed (docs)**

**Medium, and the most important finding for anyone who wires this up.** The
branch establishes, correctly and at length, that `payment_intents_seq_key`
(migration `0014`) is a **UNIQUE** index and that `ORDER BY seq DESC` is
therefore a total order. Verified: it is `CREATE UNIQUE INDEX`, not a plain
one, so the brief's third question is answered — there is no tie-breaking
problem and no intra-statement duplication.

What the branch did not say anywhere — not in the module header, not in
`docs/status.md`, not in `docs/reference/vpay-db.md`, not in
`docs/flows/dashboard.md` — is that a total order does **not** make offset
paging stable. `OFFSET` is counted afresh on every call. A payment created
between page 1 and page 2 shifts the window by one: the last row of a page
reappears at the top of the next one, and a row is missed at the tail for
every insert behind the caller's back. `seq DESC` puts new rows at offset 0,
so this is worst on the busiest merchant.

That is not a defect in the body — it is what `Page<T>` costs, and the
numbered table it buys is the reason the procedure exists. But a money list
that can show the same payment twice and skip another is a fact an operator
has to be told, and the surrounding prose (a UNIQUE index, a `count(*) OVER
()` that "cannot disagree with the page") read as a stability claim it never
made. It is now stated in all four places, together with the reason `/v1` and
`/dash/v1` are cursor-paged and the instruction that a reconciliation, an
export or a sum reads the cursor list instead.

**The repository already holds the other half of this argument**, which is
what makes the silence worth a finding rather than a footnote:
`vpay-tests-integration::customers` has
`the_cursor_pages_both_ways_without_skipping_a_row_under_concurrent_inserts`.
Someone thought this property mattered enough to write a container test for
it on the cursor surface. The offset surface has no equivalent and **cannot**
have one — the property is false there — so the absence of such a test beside
`searchPaymentIntents` is not an oversight to fix with a test, it is a
difference to write down.

### 3. The default page size was 100 where `/dash/v1` gives 10 — **fixed**

**Medium.** `MAX_PAGE_LIMIT` is 100, a deliberate copy of
`vpay_api::v1::paging::MAX_LIMIT`, and its comment gives the reason: the two
surfaces are read by the same operator, so "a procedure that hands out ten
times the page the REST list does would make 'one page' mean two different
things on one screen."

`PageInput::resolve(max)` is `limit.unwrap_or(max).clamp(0, max)` — it is the
clamp **and the default**, fused. Passing only `MAX_PAGE_LIMIT` therefore
answered 100 rows to a caller who named no page size, where
`GET /dash/v1/payment_intents` answers 10 (`DEFAULT_LIMIT`, and
`docs/api/README.md` says "default 10, capped at 100"). The exact divergence
the comment ruled out, arriving through the default instead of the ceiling.

Fixed with a separate `DEFAULT_PAGE_LIMIT` (10) and a `resolve_page` helper
that supplies it and then hands `resolve` the clamp **untouched** — the
clamp is not reimplemented, so the mutation in the table above still fires
exactly as before. `a_hostile_limit_is_clamped_before_it_reaches_postgres`
now exercises `resolve_page` rather than `PageInput::resolve` directly, which
strictly widens it: it previously pinned a CrateStack function vpay does not
own and would have passed unchanged if vpay's own defaulting were wrong.

The ceiling and the default are still copies that no gate keeps in step with
`vpay-api`. That is unchanged and still recorded.

### 4. `types::FailureCode` had no vocabulary test — **fixed**

**Medium.** `the_two_intent_status_vocabularies_are_one_vocabulary` pins
`schemas/vpay.cstack`'s `enum IntentStatus` against `vpay_core::IntentStatus`.
Nothing pinned the `FailureCode` pair, although `SummaryRow::into_summary`
parses a stored `last_payment_error_code` with the generated enum in exactly
the same way.

The asymmetry is what makes it matter. An unknown `status` is a **caller's**
typo and answers `400`. An unknown `last_payment_error_code` is a **stored**
value and answers `CratestackError::Internal`. So a variant present in one
transcription and missing from the other does not produce a filter that
matches nothing — it turns every payment that failed for that reason into a
`500` on an operator's list. And `search_payment_intents.rs` is the only
consumer of `types::FailureCode` anywhere in the workspace (checked by
`git grep`), so the schema's copy had no other reader to disagree with it.

The two agree today — eleven variants each. Added
`the_two_failure_code_vocabularies_are_one_vocabulary`, mirroring the status
test, including its honestly-admitted limit: a variant added to
`schemas/vpay.cstack` alone still escapes both halves.

### 5. The "byte for byte" refusal was asserted against a literal — **fixed**

**Low, but it is the claim the tenancy design rests on.** The body refuses a
tenantless caller with `Forbidden("procedure policy denied this operation")`,
chosen to be indistinguishable from what the `@allow` check itself produces.
Verified at the source: `cratestack_policy::eval` builds it as
`format!("{construct} policy denied this operation")` with
`construct = "procedure"`, so the claim is true today.

It was not _tested_, though. Two separate assertions each compared a message
to the same hard-coded literal, in two different tests. The day a CrateStack
release rewords the policy denial, both keep passing and the
indistinguishability is silently gone — which is the only way this particular
property can break, since neither half is vpay's to change.

`the_tenancy_refusal_happens_before_the_statement_does` now asserts the two
`public_message()`s **equal to each other**, and keeps the literal assertion
as a second, separate line so a reword is seen rather than absorbed.

### 6. `verify-docs` could not see the `#[allow]` this branch added — **fixed**

**Low-medium, and a decision the brief asked for explicitly.** The branch
declares `mod search_payment_intents` under
`#[cfg_attr(not(test), allow(dead_code, reason = "…"))]`, and says plainly
that `just verify-docs`' `#[allow]` report did not move — it stayed at six —
because the scanner matches the literal text `#[allow(`.

That is an accurate self-report and the right instinct. But the conclusion —
document the gap — is the wrong half. `AGENTS.md` advertises that report as
the place every `#[allow]`/`#[expect]` in production code is listed. A lint
silenced in every shipping build, absent from the list of silenced lints, is
the one failure that list cannot afford; and `cfg_attr` is not an exotic
spelling, it is the _correct_ spelling whenever the deadness is conditional,
so the hole would widen every time someone did the right thing.

`allow_sites` in `.xtask/src/main.rs` now also counts
`#[cfg_attr(…, allow(…))]` / `#[cfg_attr(…, expect(…))]`, deciding on the
rejoined, comment- and literal-stripped attribute text so a `cfg_attr`
carrying `derive`, `serde` or anything else is still not counted. A new test,
`a_lint_silenced_through_cfg_attr_is_counted_and_other_cfg_attrs_are_not`,
fails with `sites.len() == 0` if the four bare needles are restored.

This is a **report**, not a gate — it cannot fail a build in either
direction — so the change cannot break CI, only make it tell the truth. It is
the only production `cfg_attr` allow in the repository; every other one is in
`tests/`, which the report does not scan. Measured both ways in the table
above: 6 before, 7 after, and the six are byte-identical, so nothing else had
been hiding behind the old spelling.

`backends/crates/vpay-db/src/schema.rs`' own comment recorded "it stayed at
six" as a measured fact. It was accurate when written and is now struck
through with the correction, rather than quietly deleted.

### 7. A stale claim about `.config/nextest.toml`, repeated into new code — **fixed**

**Low.** The new container-test module says "this crate is not covered by
`.config/nextest.toml`'s one-at-a-time filter, so every container test here
can be starting at the same moment as the others", citing
`tests/postgres.rs`' header.

`vpay-db` **is** covered, and has been since 2026-09-02: the
`postgres-containers` override's filter reads
`package(vpay-tests-integration) | package(vpay-tests-conformance) |
package(vpay-db) | package(vpay-server) | package(vpay-testkit)`, and that
file's own comment records the widening. `tests/postgres.rs` has carried the
stale half since the widening landed and the new module inherited it.

Corrected in both files, with the dated strike-through this repository uses.
The _conclusion_ — few, chunky tests — survives on a weaker but real reason:
under `max-threads = 1`, six tests cost six container starts in series.

---

## Upheld without change

- **The tenancy predicate is the body's, not the policy's**, and it reads
  `ctx.tenant_id()` rather than an argument. There is no `merchant_id`
  argument and there must not be one. `tenant_of` refuses an absent tenant
  _and_ an empty-string tenant, the second being the one that would otherwise
  compile into a well-formed predicate matching nothing. Independently
  confirmed that `CratestackContext::tenant_id` returns `None` for a
  non-string claim, so the refusal is fail-closed on a malformed context too.
- **A foreign tenant and a tenant that owns nothing get the same empty page.**
  That is the uniform refusal two prior reviews hardened elsewhere, and this
  new way into the same rows matches it.
- **No credential and no `jsonb` column in the projection.**
  `the_statement_selects_no_credential_and_no_jsonb_column` asserts over the
  statement text rather than the struct, which is the version that notices a
  column added to both.
- **The filter set is exactly `/dash/v1`'s three.** Not a fourth. `livemode`
  is deployment-wide (`config.deployment.livemode`, never per request) and
  the REST list does not filter on it either, so its absence is parity rather
  than a gap.
- **Every vendored-source citation in the branch's prose checks out** against
  the 0.12.0 crates on disk: `is_filterable_scalar` really does include
  declared enums (so "enums are not FindMany-filterable" really is false for
  this release, as the branch says); `Json` really does map to
  `::cratestack::Json<::cratestack::Value>`; `Value::from_plain_json` really
  does demote through `as_i64`/`as_f64`; `rpc.rs` really does encode with
  `serde_json::to_value`; `validate/procedure_handler_collisions.rs` exists.
  The one slip is a line number off by one, which is not worth a commit.
- **`sql_audit`'s `EXPECTED_ASSERT_SITES` correctly did not move.** Checked
  that `sources()` walks `src/` recursively, so the first SQL in a
  subdirectory is inside the audit rather than outside it.
- **The generated module stays private** and `verify-repositories`' rule is
  untouched.

---

## The final gate

`just ci` on the review's own head, run under CI's pins rather than the host's
defaults — Node **22.23.2** (`.nvmrc`; the host's own `node` is v24.20.0),
Rust **1.98.0** (`rust-toolchain.toml`), `pnpm install --frozen-lockfile`,
`DOCKER_HOST=unix:///run/user/1000/docker.sock`, `CARGO_BUILD_JOBS=6`.

**Exit 0**, read from a file the runner wrote rather than from a harness
banner.

| step                    | result                                                     |
| ----------------------- | ---------------------------------------------------------- |
| `fmt-check`             | ok (cargo fmt + prettier over the whole tree)              |
| `clippy`                | ok, `--workspace --all-targets -- -D warnings`             |
| `verify`                | ok — all twelve gates                                      |
| `test-rust`             | **1709 run, 1709 passed, 0 skipped** (2 slow), 1289 s      |
| `test-doc`              | **111 passed, 0 failed, 1 ignored** (the pre-existing one) |
| `verify-ignored`        | ok                                                         |
| `lint-web` / `test-web` | ok                                                         |
| `deny`                  | advisories ok, bans ok, licenses ok, sources ok            |

1709 is the branch's 1707 plus this review's two additions
(`the_two_failure_code_vocabularies_are_one_vocabulary` in `vpay-db`,
`a_lint_silenced_through_cfg_attr_is_counted_and_other_cfg_attrs_are_not` in
`xtask`). Both land in existing binaries, so `expected_suites` does not move
and `min_tests` is cleared with the same margin.

One container test went `SLOW [> 60s]` and then passed
(`config_reconcile::tests::a_provider_reads_through_cratestack_exactly_as_it_
does_through_sqlx`, under load) — the mode exp53's notes record, and distinct
from the instant `hyper Connect` failure that a missing `DOCKER_HOST` causes.
Both look identical in a one-line summary and have different fixes.

---

## What this review did **not** check

- **It did not exercise the procedure through any transport**, because none
  exists. What is proven is the body, the policy check and the witness,
  called the way a transport would call them. The branch says this; the
  review confirms it rather than extending it.
- **It did not attack the generated code** — the `Args` decoder, the
  `Authorized` witness's constructibility, or the RPC/axum dispatch paths.
  None is compiled into this workspace at the feature set `vpay-db` uses.
- **It did not re-derive the statement counts** in
  `docs/reference/vpay-db.md` § "What runs through it today". The branch's
  claim that a procedure moves none of them is sound in kind — a procedure is
  not a builder chain — and was accepted without recounting thirty-two
  statements.
- **It did not measure query plans.** That `ORDER BY seq DESC` under
  `WHERE merchant_id = $1` uses `payment_intents_merchant_seq_idx` is read off
  the index definition, not off an `EXPLAIN`.
- **It did not run `cargo xtask verify-citations`** (network + token), so the
  PR and issue numbers cited in the branch's docs are unverified here.
- **`just check-schema` ran against cratestack-cli 0.11.1, not the pinned
  0.12.0.** The recipe's version mismatch is a WARNING rather than a failure,
  so `just verify` went green on this branch having type-checked
  `schemas/vpay.cstack` under the older grammar. It passed, and the branch's
  own account of `procedure`/`type` syntax is 0.12.0-sourced and checks out
  against the vendored crates — but **the new declarations have not been
  parsed here by the version CI installs**. That is an environment gap on
  this host, not a defect in the branch, and CI will close it. Recorded
  because "verify passed locally" means slightly less than usual on this
  particular file.
- **It did not re-run the review under a second Docker configuration.** The
  first baseline attempt failed with `failed to create a container: client
error (Connect)` — testcontainers could not reach the daemon, because this
  host runs rootless Docker and the socket is
  `unix:///run/user/1000/docker.sock`, not the default. Every run reported
  above sets `DOCKER_HOST` to it. Worth recording because that failure looks
  exactly like a broken test and is not one; and because the alternative — a
  container test that silently does not run — would have made the tenancy
  evidence worthless.
- **It did not touch `docs/status.md`'s large `schemas/*.cstack` table row**,
  which still says of `PaymentIntent` that "nothing reads or writes through
  them". That remains true of the _model_ and is what the row is about, but a
  reader now has to know that to read it correctly. Left alone deliberately:
  the row is a single enormous table cell, the new
  "The first `procedure`" section says the same thing at length, and editing
  it is a formatting risk for no gain in truth. **Named here rather than
  fixed.**
