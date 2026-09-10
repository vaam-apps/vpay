# exp42 (opus) sabotage review of the haiku draft — issue #61

Branch `claude/exp42-boot-coherence`, base `970bfe0`, draft `e43530b` (one
haiku commit). Review head: see § 5. Nothing was pushed.

## 0. What the draft got right, so the rest is read in proportion

The design is the one issue #61 asked for and it is in the right place:
`boot_seeds` asks each configured rail's adapter for its `Capabilities` and
refuses an incoherent set *before* `reconcile_reference_tables`, with a
`ConfigError` variant — so the exit code is `78` at every call site, by type,
through `vpay-server`'s existing `exit_code_for`. The CHECK and its test were
not touched. The decisive mutation the brief named was verified against the
draft as delivered:

| # | Mutation | Draft as delivered | After this review |
|---|---|---|---|
| M1 | delete the `is_coherent` call from `boot_seeds` | `a_provider_with_incoherent_capabilities_is_a_config_error` **FAILS** | FAILS (unit) **and** `boot_coherence.rs` FAILS (integration, 82 s, message names the regression) |
| M2 | empty `#[error(…)]` to `"configuration error"` | **PASSES** — the test read the variant, never the message | **FAILS** on both message assertions |
| M3 | delete the call, ask a real Postgres | not measured — the "78 not 1" claim was argued | FAILS: `boot_seeds` returns `Ok`, and the seed then meets `partial_refunds_imply_refunds` as `Category::Internal` / exit `1` |

M2 is the finding that matters: issue #61's own acceptance sentence is "the
message names the rule", and nothing in the tree held it.

## 1. Findings

**F1 — gate-hole (fixed).** The draft ran no `just ci` and no container suite,
and the "exits 78 rather than 1" claim — the entire content of issue #61 — was
argued in a commit message, not measured. A unit test on a pure function
cannot say which layer answers *first*.
Fixed: `backends/tests/integration/tests/boot_coherence.rs` calls the two
shipping functions `main` calls, in `main`'s order, against a real Postgres 16,
and asserts `78` + an empty `providers` table + the CHECK's `1` for the seed
boot withheld. Mutation M3 above is what makes it decisive.

**F2 — correctness of the test (fixed).** The draft's test asserted the variant
and the `code` field and *claimed*, in its own panic message, that "the message
must name the provider and the violated rule" — while asserting nothing about
the message. Measured (M2): the message could be emptied entirely and the test
stayed green. Fixed: both substrings are asserted, in the unit test and in the
integration test.

**F3 — correctness (fixed).** The message named the rule in prose
(`supports_partial_refunds=true requires supports_refunds=true`) but not by the
name the migration, the database's own error text and
`Capabilities::is_coherent`'s doc all use. An operator who greps
`partial_refunds_imply_refunds` — the string a Postgres error hands them —
found the boot refusal in no layer. It now names it.
On the brief's question "which rule?": `is_coherent` encodes exactly one,
`!supports_partial_refunds || supports_refunds`, so the message can name it
without qualification.

**F4 — correctness, small but real (fixed).** The guard refuses every
*configured* rail, `enabled` or not, which is right — `enabled` is a column on
the row the seed becomes, so a disabled incoherent rail reaches the same CHECK
— but nothing said so and nothing tested it, and `provider.enabled &&
!coherent` is the obvious "optimisation". The test now runs both values.

**F5 — misleading claim (fixed).** `boot_seeds`' own `# Errors` section still
read "That is now the *only* error this returns" while the function had two.

**F6 — conventions / DRY (fixed).** `IncoherentTestRail` was a 56-line copy of
the module's `TestRail` differing in two booleans; ADR-0016 standard 4. It is
now `TestRail` carrying a `Capabilities`, so the incoherent rail differs from
the coherent one by exactly what is under test.
On the brief's no-mocks question: **no rule-break either way.** `verify-no-mocks`
scans `backends/apps` sources for stub-adapter *names* and the shipping
manifests for test-only *dependencies*; a `#[cfg(test)]` adapter inside a
library crate is outside both, and the module's existing `TestRail` is the
repository's own precedent, documented as such. There is no `vpay-testkit` rail
type to use instead — that crate is containers only, and the integration suite
deliberately links the **real** adapters. The new integration file carries the
same argument in its header (it declares a capability set and answers no call;
a stub *rail* is a WireMock host in configuration, ADR-0006).

**F7 — misleading docs (fixed).** `docs/flows/configuration.md` came out of the
draft asserting both "**not a `vpay-config` boot guard at all**" and, three
paragraphs later, "**This is now a `vpay-config` boot-time guard**". The second
is also inaccurate: the check is in `vpay-api`, `Config::validate_all` does not
run it, and **no YAML can trigger it** — a capability set is a property of the
adapter's code, so what boot catches is a linking mistake. The "enforced three
times, independently" list also promoted two *tests* to an enforcement layer.

**F8 — misleading docs, pre-existing and made worse (fixed).** The same section
said a seed that reaches the CHECK is a `DbError::Query` — exit `69`, "wait for
Postgres". It has been `PersistenceError::Check` → `Category::Internal` → exit
`1` since 2026-09-06. Left alone, the doc would have claimed the change moved
`69` → `78` when it moved `1` → `78`. Re-measured here on the boot path.

**F9 — the decision was left open where it was raised (fixed).** The brief's
"close the exp20 review's open item" was not done: `docs/status.md` still
carried **"Maintainer decision, surfaced not taken"** for this exact question,
including the sentence "`Capabilities::is_coherent` … never at boot", which the
draft had just falsified. Closed on both pages (`docs/status.md`, and § 4 of
`docs/plans/exp20-provider-defaults-notes/opus-review.md` where it was raised).

**F10 — docs completeness (fixed).** `docs/flows/configuration.md`'s `## Status`
section and `docs/reference/vpay-config.md`'s boot-sequence table (step 8, and
the "cheapest hard failure first" argument that is the reason for the
placement) were untouched by the draft. CLAUDE.md's finish list asks for the
flow doc's Status section by name.

**F11 — nit (fixed).** No blank line between the new enum variant and its
neighbour.

### Not fixed, and why

* **`support::reconcile_from_config`** in the integration harness is a
  hand-copy of `boot_seeds`' join and does **not** check coherence. It is a
  test harness, its own comment says it mirrors `boot_seeds`, and changing it
  is outside this issue; worth an issue of its own, since a harness that
  drifts from the binary is the failure that module's header warns about.
* **The `@vpay/ui` `select.test.tsx` flake** (§ 5). Unrelated to this branch
  and not this review's to fix; reported, not papered over.
* **No subprocess (`tests/cli.rs`) case**, which the issue's own wording
  suggested. It cannot exist and the reason is structural, not effort:
  `vpay-server` links `mtn_momo` and `orange_money`, both asserted coherent by
  `no_adapter_advertises_partial_without_full_refunds` and by the conformance
  suite, so **no YAML can drive the shipping binary into this refusal**. The
  draft said "cannot exist in production code" and was right for the wrong
  reason; the integration test is the closest thing that can exist, and the
  module header records why. (`backends/apps/vpay-server/tests/cli.rs` is also
  exp39/#99's file, which the brief put out of bounds.)

## 2. Commits

Four, one per finding-group, each carrying its own proof in the message; see
§ 6 for the list. Nothing was pushed.

## 3. Mutations, as run

Recorded in the table in § 0. Commands:
`cargo nextest run -p vpay-api -E 'test(/incoherent/)'` and
`cargo nextest run -p vpay-tests-integration -E 'binary(boot_coherence)'`,
with the mutation applied to
`backends/crates/vpay-api/src/v1/boot.rs` / `backends/crates/vpay-config/src/lib.rs`
and reverted from a copy afterwards (`git status` clean between runs).

## 4. What was not checked

* `just test-e2e` and `just helm-check` — outside `just ci` by design.
* `just docs-check-citations` — needs the network and a token; this document
  cites issue #61 and PR #99 only, both read with `gh` during the review.
* Whether a *rebase* onto #99 (exp39, `tests/cli.rs`) still applies cleanly:
  #99 was still open at review time (`gh pr view 99` → OPEN), so STEP 0's
  rebase was a no-op — `origin/master` is `970bfe0`, already the base.
* The behaviour of a **real** incoherent adapter in a shipping binary: there
  is none, by construction (see § 1, "Not fixed").

## 5. Gate

`just ci` end to end on `096c824` — the head carrying all four commits below —
**exit 0**, read from a file, not from a banner. Under rustc 1.98.0
(`rust-toolchain.toml`), Node 22.23.2 (`.nvmrc`) with `pnpm install
--frozen-lockfile`, the pinned CrateStack **0.12.0** CLI, rootless Docker.

| Recipe | Result |
|---|---|
| `fmt-check` | ok |
| `clippy` (`--workspace --all-targets -D warnings`) | ok |
| `verify` | twelve gates ok; `verify-docs` advisory report printed |
| `test-rust` | **1660 tests run, 1660 passed, 0 skipped** (1281 s). `boot_coherence` ran in 1.4 s inside the full run |
| `test-doc` | **111 passed, 1 ignored** (the ignored one is pre-existing) |
| `verify-ignored` | 0 ignored (expected 0), **46** test binaries (expected 46), 1660 total |
| `lint-web` | ok (typecheck + `pnpm -r lint`) |
| `test-web` | ok — 1284 tests across every package |
| `deny` | `advisories ok, bans ok, licenses ok, sources ok` |

**A flake, reported rather than papered over.** An earlier full run *on the
draft head* (`e43530b`) failed `test-web` on `@vpay/ui`'s
`select.test.tsx > Select > opens on ArrowDown …` while the container suites
were loading the host; `git diff origin/master -- frontends/` is empty on this
branch, the file passes 74/74 in isolation, and it passed in the run above. It
is not this branch's, and it is worth an issue.

**And one before that.** The first run on the draft head died at test
1270/1659 with `container is not ready: container startup timeout` on
`vpay-db::repositories`, with 1269 passed and none failed on merit — the host
had eighteen containers up at the time. Re-running took it past that point.
Recorded because "the gate went red once" is part of the evidence.

## 6. Commits on this branch (base `970bfe0`)

| SHA | |
|---|---|
| `e43530b` | the haiku draft, untouched |
| `ebc3b9e` | `fix(config,api)`: the refusal names the rule, and the test reads the message |
| `5cb01a3` | `docs(api)`: `boot_seeds` returns two errors, and its own `# Errors` said one |
| `d9308b1` | `test(integration)`: the 78 and the 1, measured against a real Postgres |
| `096c824` | `docs`: the flow doc contradicted itself, and the decision it settles was still open |

This document is committed on top of `096c824` and changes nothing the gate
compiles; `just verify` was re-run on the head that carries it.

## 7. Verdict

**Not safe as drafted — safe now.** The draft's design was right and its
central mutation held, but two of issue #61's three acceptance sentences were
unheld by any test (the message naming the rule; the exit code being 78 rather
than 1), the flow doc contradicted itself in the same section, and the
maintainer decision this issue *is* was still recorded as open on
`docs/status.md`. All four are closed above, each with the measurement that
would catch its regression.
