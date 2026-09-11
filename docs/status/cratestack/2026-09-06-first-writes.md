# CrateStack — the first writes (2026-09-06)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### The first CrateStack writes (2026-09-06)

`DisabledClients::disable_client` is `.upsert(CreateDisabledClientInput).run(&system_context())`
and `DisabledClients::enable_client` is
`.delete_many().where_(client_id.eq(..)).run(&system_context())`. That is the
whole of one table's repository — read and both writes — through the generated
data layer, and still the only table in the workspace that is. `model
DisabledClient` gains `@@allow` arms for `create`, `update` and `delete`
alongside the `read` it already had; `cratestack_min_declarations` does not
move (13), the drift constants do not move (**85 changes over 16 relations,
18 unmappable columns**, re-run and passing), and there is **no migration**.

**The trait surface is unchanged and the `# Errors` contract is not.** Both
writes now return `DbError::Persistence` where they returned `DbError::Query`,
the same correction the read took hours earlier. A caller branching on the
classification sees nothing — `classify_cratestack` and `classify_write` are
asserted against each other in `persistence.rs` — and a caller matching the
_variant_ would have silently stopped matching, which is why both trait doc
comments say so in `# Errors`.

**What did NOT move:** every other table, every transaction (`UnitOfWork` is
untouched and no CrateStack `run_in_tx` is called anywhere), every enum
column, and the transport. `backends/migrations/*.sql` is still the
authoritative schema.

**Two deviations from the obvious shape, both measured rather than
preferred:**

1. **`enable_client` is `delete_many`, not `delete(pk)`.** `cratestack-sqlx`
   0.12.0's `query/write/delete_exec.rs` reports a `DELETE … RETURNING` that
   matched no row as `CratestackError::Forbidden("delete policy denied this
operation")`, and with no `@version` column on this model it has no way to
   tell that from a real policy refusal. `.delete()` would therefore turn
   every re-enable of an already-enabled client into a `Category::Internal`
   error, breaking this trait's documented "a no-op, not an error" contract
   and two existing tests. The cost of the alternative is recorded in the
   table below: `delete_many`'s policy lives in the `WHERE`, so a missing
   `delete` policy is silent.
2. **`disable_client` costs a second round trip.** `.upsert()` is
   transactional by construction — it probes the conflict target with `SELECT
… FOR UPDATE` and may run an `ON CONFLICT DO NOTHING` first — to tell a
   create from an update for an event/audit fan-out this model has neither of.
   Accepted: disabling a client is an operator action, not a hot path.
   `is_client_disabled` is on the token path and stayed a single
   `find_unique`.

**The rendered upsert is the statement it replaced, byte for byte** —
`INSERT INTO disabled_clients (client_id, reason) VALUES ($1, $2) ON CONFLICT
(client_id) DO UPDATE SET reason = EXCLUDED.reason` plus a `RETURNING` nobody
reads. `disabled_at` is in neither the insert list nor the `SET` list, because
`cratestack-macros` excludes every `@default(...)` column from both, which is
what keeps "a second disable leaves the original `disabled_at` untouched"
true. Two new unit tests in `disabled_clients.rs` assert that rendering
without a database; their value is as a guard on a **pinned external crate's**
output, since a schema edit that removed the default is caught earlier by the
compiler (`error[E0063]: missing field \`disabled_at\``, measured).

**Decisive tests — six mutations, all run against a real Postgres, none
merely designed.** The most useful result is that the three write actions do
not behave alike:

| Mutation                                                       | Runtime effect                                                                                   | What went red                                                                                                                                                |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| drop `@@allow("create", …)`                                    | `disable_client` **errors**: `Forbidden` → `PersistenceError::Denied` → `Category::Internal`     | `a_client_disabled_through_cratestack_is_visible_to_both_paths`, at the **first** disable                                                                    |
| drop `@@allow("update", …)`                                    | `disable_client` **errors on the conflict branch only** — the first disable of a client succeeds | the same test, at the **second** disable, plus `disabled_client_lookup_reflects_disable_and_enable`. A test that disabled once and stopped would have passed |
| drop `@@allow("delete", …)`                                    | `enable_client` removes nothing and returns **`Ok`** — silent                                    | the same test's enable assertion, which reads the row back through a plain `SELECT`; plus the lookup test                                                    |
| drop `@@allow("read", …)`                                      | kill switch silently OFF (unchanged from the read change)                                        | `a_disabled_client_reads_the_same_through_both_paths`, `CrateStack says false, sqlx says true` — **re-verified after its seed changed**                      |
| replace the `upsert` call with `Ok(())`                        | disable does nothing                                                                             | `find_client_reflects_the_disabled_clients_kill_switch` (integration): "a disabled client must stop resolving immediately"                                   |
| replace `delete_many` with an `update_many` that sets `reason` | enable updates instead of deleting                                                               | the enable assertion, on the row still being there                                                                                                           |

So: **`create` and `update` fail loudly, `read` and `delete` fail silently.**
`upsert_exec.rs` evaluates the create policy in Rust before it builds any SQL
and `upsert_resolve.rs` does the same for the conflict branch, so an empty
allow list is a real error there; the read and `delete_many` paths compile the
policy into a `WHERE` clause where an empty allow list renders `FALSE`. The
two silent holes fail in opposite directions, and only one of them is safe: a
missing `read` policy leaves every revoked client **admitted**, a missing
`delete` policy leaves a client **revoked**.
Neither is visible to `just check-schema`, `cargo build`, `just clippy` or any
of the ten `just verify` gates — re-measured for the three new arms.

**`a_disabled_client_reads_the_same_through_both_paths` changed, and had to.**
It seeded its row by calling `disable_client`; that call is now a CrateStack
`upsert`, so leaving it would have made the test a generated write read back
by a generated read, agreeing with itself — the exact thing that test's own
doc comment disclaimed. It seeds with an inline `INSERT` and removes with an
inline `DELETE` now, deliberately not shared with the new write test.

**The gate that was blind, and now is not (review, 2026-09-06).** Every one
of the four mutations above was re-run by the sabotage review, and all four
reproduced — _and_ all four left `cargo nextest run -p vpay-db --lib` green at
26 passed. The database-free half of the gate could not see a single missing
`@@allow` arm, including the two whose runtime effect is silent. `vpay-db`'s
unit suite now carries
`disabled_clients::tests::every_action_this_crate_calls_has_an_allow_arm`,
which reads the four `&'static [ReadPolicy]` slots off the compiled
`ModelDescriptor` and fails in 4 ms with no Docker; deleting each arm in turn
was re-run against it and it is red four times out of four. It does not
replace the container tests — a non-empty slot is not the same claim as "the
policy admits this caller" — it removes the wait to find out that a slot is
empty. Details and the transcripts:
[docs/plans/exp16-notes/opus-review.md](../../plans/exp16-notes/opus-review.md).

**Stays `NotImplemented`: nothing.** No function gained a stub, no test was
weakened, and no `#[ignore]` was added.

**Not done, named:** the transaction seam is still unexercised — no CrateStack
`run_in_tx` is called anywhere, because neither of these two writes has
anything to be atomic with. `providers`, `currencies` and every other table
are unmoved. The `Unique`, `ForeignKey` and `Check` arms of
`PersistenceError` are still unit-tested rather than exercised:
`disabled_clients` has no foreign key and no CHECK, and its only unique
constraint is the primary key the upsert exists to absorb. The release image
was not rebuilt (this change adds no dependency and no file the Dockerfile
would have to copy).
[docs/plans/exp16-notes/opus.md](../../plans/exp16-notes/opus.md) has the
transcripts and § 6 the full "not measured" list.

**Gate, run on this branch on 2026-09-06 against the pinned 1.98.0
toolchain.** `cargo build --workspace --all-targets`, `just fmt-check`,
`just clippy`, all **ten** `just verify` gates, `cargo nextest run -p vpay-db
--lib` **26 passed** (24 + the two render tests), `cargo nextest run -p xtask`
**215 passed**, `just test-doc` **96 passed, 1 ignored**, `just deny`
`advisories ok, bans ok, licenses ok, sources ok`, `just docs-check`, and
`just verify-ignored` **0 ignored (expected 0), 43 test binaries (expected
43), 1372 total**. `just test-rust`: **1372 tests run, 1372 passed, 0
skipped** across 43 binaries in 689.775 s — 1369 plus this change's three (two
`vpay-db` unit tests and one integration test; no new test binary, so
`expected_suites` stays 43 and `min_tests` stays 1080). The container-backed
drift test was re-run separately and passes with its constants unmoved.

**Re-measured by the review on the same day: 1373 / 43 binaries / 0 skipped,
`-p vpay-db --lib` 27**, after the review added the policy-slot test described
below. `just test-doc` **96 passed, 1 ignored** and all ten `just verify`
gates unchanged; `just ci` green end to end.

The first of the two `just test-rust` runs failed one test —
`webhooks the_delivered_signature_verifies_with_the_shipping_node_sdk`, with
`sh: 1: tsc: not found` — because no `node_modules` had been installed in that
worktree. It is unrelated to this change, which touches no TypeScript;
`pnpm install --frozen-lockfile` under the pinned Node 22.23.2 fixed it and
the rerun above is the green one. Recorded rather than dropped, because
"1372 passed" on a second attempt is a different claim from "1372 passed".
