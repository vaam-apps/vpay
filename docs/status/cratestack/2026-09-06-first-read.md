# CrateStack — the first read (2026-09-06)

_Archived from [docs/status.md](../../status.md) on 2026-09-11 by exp57, which split a 6 151-line page into pages a person can read. **The text below is the original, unedited** — every dated measurement, every struck-through claim and every correction is here as it was written; only the relative links gained a `../../` because the file moved two directories down._

_It still says "above" and "below" where it once pointed at another part of the same page. Those targets are on sibling pages now, and [README.md](../README.md) is the index of them._

#### The first CrateStack read (2026-09-06)

Three things landed together, and each has a mutation that turns a gate red.

**1. The dependency.** `cratestack = { package = "cratestack-pg", version =
"=0.12.0", default-features = false, features = ["postgres"] }` in
`[workspace.dependencies]`, taken by `vpay-db` and by no other crate. The
rename is forced: the schema macros emit absolute `::cratestack::*` paths and
cannot be told otherwise. The exact pin is forced too, by three other places
that already name the same version — `justfile`'s `cratestack_version`, the
drift measurement, and `rust-toolchain.toml`'s 1.98.0 (which exists _because_
every crate of the pinned release declares `rust-version = "1.98.0"`; 0.11.1
when this landed, 0.12.0 since 2026-09-07, and the floor is the same at
both). The library and the CLI
must answer about one grammar.

`Cargo.lock` goes **469 → 497 packages (+28)**; `syn` moves 3.0.3 → 3.0.5.
Twelve of the twenty-eight are `cratestack-*`, all MIT. The rest:
`ar_archive_writer` (Apache-2.0 WITH LLVM-exception), `ariadne`, `chumsky`
(MIT), `const-oid`, `erased-serde`, `hashbrown`, `object`, `psm`, `stacker`,
`typeid`, `unicode-segmentation`, `unicode-width`, `wasm-streams` (MIT OR
Apache-2.0), `foldhash` (Zlib) — and **`minicbor` + `minicbor-serde`
(BlueOak-1.0.0)**, which is the whole reason for the licence exception below.
`cargo tree -i aws-lc-rs` is still empty and `cargo tree -d` still shows
exactly one `sqlx` (0.9.0). **Corrected 2026-09-06 by review:** +28 counts
`Cargo.lock` _entries_, and only **25 are new crate names** — `const-oid`,
`foldhash` and `hashbrown` are extra _versions_ of crates already in the
graph, which is also why the new version duplicates are three and not two:
`const-oid` (0.9.6/0.10.2), `foldhash` (0.1.5/0.2.0) **and `hashbrown`**
(three versions to four). All under `multiple-versions = "warn"`, and
`cargo deny check bans` is green.

**The MSRV floor moved 1.94 → 1.98** as a direct consequence: the twelve
`cratestack-*` packages are now the sole maximum over the graph's declared
`rust_version` fields, where the seven `sqlx-*` were. It is still a metadata
floor and still not verified by compiling at it; that it now equals the
toolchain pin is a coincidence of one release, not a policy change.

**2. The licence exception.** `deny.toml` gains a **scoped** exception —
`minicbor` and `minicbor-serde` only, by name, never an entry on the `allow`
list. Blue Oak 1.0.0 is OSI-approved and permissive with an express patent
grant, so it clears the bar that list's comment sets; it is scoped because a
_third_ Blue Oak crate arriving is a new fact that should fail the gate.

It is not avoidable. `cratestack-pg` declares `cratestack-axum` and
`cratestack-client-rust` non-optional with no feature gating either, and both
take `minicbor` unconditionally; `default-features = false, features =
["postgres"]` — the smallest set that still yields a data layer — does not
remove them. Dropping to `cratestack-sqlx` + `cratestack-macros` directly does
not help either: `include_server_schema!` emits `pub mod axum { use
::cratestack::HttpTransport; … }` unconditionally.

- **Decisive test:** delete the exception and `cargo deny check licenses`
  **FAILS**, naming `minicbor` and `minicbor-serde` at
  `license = "BlueOak-1.0.0"`. With it, `cargo deny check` reports
  `advisories ok, bans ok, licenses ok, sources ok`. **No other new licence
  or ban appeared** — that was checked before the exception was written, not
  assumed.

**3. One read, and only one.** `DisabledClients::is_client_disabled` — the
OAuth kill-switch lookup — is now
`self.cs.disabled_client().find_unique(id).run(&system_context())`. Nothing
about the trait's public surface changed. `schemas/vpay.cstack` gains `model
DisabledClient` with `@@allow("read", auth().isSystem())` and nothing else;
`cratestack_min_declarations` goes 12 → 13.

**What did NOT move, stated plainly:** ~~the two `disabled_clients` _writes_
are still raw sqlx~~ **— superseded later the same day; see "The first
CrateStack writes" below** — no other table moved, no transaction, no enum
column, and nothing about the transport — the generated `pub mod axum`
compiles and is never referenced. `backends/migrations/*.sql` is still the
authoritative schema and still drives every migration.

- **Decisive test (the policy trap):** delete `@@allow("read",
auth().isSystem())` from `model DisabledClient` and
  `a_disabled_client_reads_the_same_through_both_paths` in
  `backends/crates/vpay-db/tests/repositories.rs` **FAILS** — the CrateStack
  read returns `None` for a row a direct `SELECT` finds. This is the failure
  mode the whole adoption most has to be unable to make silently: model
  policies are compiled into the `WHERE` clause and a model with no `@@allow`
  is deny-by-default, so a policy mistake does not raise an error, it turns
  the kill-switch off. `just check-schema` stays green through it, and so
  does every other gate. **Run on 2026-09-06, not merely designed:** the
  mutation was applied and the test failed with `CrateStack says false, sqlx
says true`; the schema was restored afterwards and the tree is clean.
- **Decisive test (the gate):** make `mod schema;` public — or add `pub use
schema::cratestack_schema;` — and `cargo xtask verify-repositories`
  **FAILS**. **Measured against the gate as it stood before this change:
  both spellings printed `ok`.** The module `include_server_schema!` creates
  does not exist in any source file, so neither of the gate's two original
  signals, nor rustdoc, nor any lint could see it. `DB_HANDLE_TYPES` also
  learned `Cratestack` and `SqlxRuntime`, so a store holding the generated
  runtime instead of a `PgPool` is recognised as an implementation;
  whole-identifier matching keeps `CratestackError` and `CratestackContext`
  out, asserted rather than assumed.

  **Extended 2026-09-06 by review, after two more spellings were measured
  past it.** `pub type CratestackHandle = crate::schema::cratestack_schema::Cratestack;`
  and a `pub fn` returning the same type both compiled and both printed
  `ok`: the check read `pub mod` and `pub use` and nothing else, and
  `concrete_repository_types`' alias fixpoint cannot help, because it only
  promotes an alias whose target is already in the set and `Cratestack` is
  declared by no source file. The gate now also fails any unrestricted-`pub`
  item _signature_ naming the module, anywhere in the crate;
  `PgRepositories`' real `pub(crate) cs` field and `boxed`'s body still pass,
  which is asserted rather than hoped for. `cargo nextest run -p xtask`
  197 → 198.

**Errors.** `PersistenceError` (`vpay-db/src/persistence.rs`) is the leaf,
`DbError::Persistence` `#[from]`s it and delegates, and `classify_cratestack`
is the single place a `CratestackError` is read — the mirror of
`classify_write`, branching on the same SQLSTATEs. CrateStack's own
`status_code()` is **not used**: its `DatabaseTyped` is a 500 with `"internal
error"`, which would answer a duplicate charge as an outage. Two honest
limits, both in the code's own doc comments and in
[docs/reference/vpay-db.md](../../reference/vpay-db.md): a CrateStack _read_ never
carries a SQLSTATE at all (`FindUnique::run` stringifies its `sqlx::Error`),
so today every failure of this one query lands on `Storage` — the same answer
the `SELECT` it replaced gave; and a policy denial cannot be produced by the
read path either, because a refused read is a `WHERE` clause rather than an
error. The SQLSTATE and `Denied` arms are unit-tested, not exercised.

**Stays `NotImplemented`: nothing.** No function gained a stub, no test was
weakened, and every method touched already worked and still works.

~~**Three container-backed cases on this branch have NEVER been executed, and
are owed to CI.**~~ **Superseded 2026-09-06: the host's Docker daemon came
back and all three were run.** `just ci` ran **end to end, exit 0**, on this
branch rebased onto master at `6978901` (#50, `refunds.fee`): `just fmt-check`,
`just clippy`, all **ten** `just verify` gates, `just test-rust` **1369 tests
run, 1369 passed, 0 skipped** across 43 binaries in 722.977 s, `just test-doc`
**96 passed, 1 ignored**, `just verify-ignored` **0 ignored (expected 0), 43
test binaries (expected 43), 1369 total**, `just lint-web`, `just test-web`
(1369 is master's 1359 plus this branch's ten tests; this branch still adds no
test binary, so `expected_suites` stays 43 and `min_tests` stays 1080), and
`just deny` `advisories ok, bans ok, licenses ok, sources ok`. The three cases
by name, all **PASS**:

1. `vpay-db::repositories a_disabled_client_reads_the_same_through_both_paths`
   — the parity test. **Executed for the first time on 2026-09-06: PASS**,
   6.371 s standalone and 1.627 s inside the full run. "The CrateStack read
   returns what the sqlx read returns" is now a measurement rather than a
   reading of the generated query builder.
2. The decisive mutation on it — deleting `@@allow("read", auth().isSystem())`
   from `model DisabledClient` — **was run, and the parity test FAILED** with
   `CrateStack says false, sqlx says true`, i.e. the kill-switch silently OFF,
   while `just check-schema` stayed green. The schema was restored. The
   transcript is in [plans/exp14-notes/opus.md](../../plans/exp14-notes/opus.md) §8.
3. `just test-rust` itself — run, not merely listed, including the
   pre-existing `disabled_client_lookup_reflects_disable_and_enable` that
   exercises the changed method (PASS, 1.274 s), `client_store`'s
   `find_client_reflects_the_disabled_clients_kill_switch` (PASS), and all ten
   `merchant_token_flow` cases.

The drift test was re-run on the final rebased tree: **PASS**, and its
constants do not move — still **85 changes over 16 relations**, 18 unmappable
columns. Migration `0031` adds `refunds.fee` to a table `schemas/vpay.cstack`
does not declare at all, and an undeclared table is one report line whatever
its column count.

~~The server image was not built, so **the size cost of CrateStack's graph on
the static musl link is unmeasured**.~~ **Built 2026-09-06, and it found a
defect this branch had introduced:** `backends/Dockerfile` never copied
`schemas/` into the build context, so `include_server_schema!` failed at macro
expansion with `failed to read schema file .../schemas/vpay.cstack: No such
file or directory` and **the release image could not be built at all**. No
gate in this repository would have caught it — `just ci` builds on the host,
where the file is present. Fixed by adding `COPY schemas ./schemas` to both
the `planner` and `builder` stages. The size cost, measured paired on one host
and builder on 2026-09-06 (master rebuilt from a `git archive` of `6978901`
rather than compared against an older quoted figure): master **16.1 MB**, this
branch **16.9 MB** — **+0.8 MB (+5.0%)** for CrateStack's twelve crates plus
`minicbor`, `chumsky` and `ariadne`. Both images run and print `vpay-server
0.1.0`.
`docs/plans/exp14-notes/opus.md` § 7 has the full list.
