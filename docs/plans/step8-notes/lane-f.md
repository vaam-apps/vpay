# Lane F — SDK parity — notes

Branch: `claude/step8-lane-f-sdk-parity`. Not part of the original Step 8
lane list (A–D + E); added after that plan was written, per the user's rule:
*"vpay sdk parity: each should share the very same kind of features. We need
a matrix for that."*

## What landed

1. [`docs/adr/0015-sdk-parity.md`](../../adr/0015-sdk-parity.md) — **Accepted**.
   The rule (capability-level parity, same wire semantics), how a gap is
   recorded (dated, owned), how the check reads the matrix, and what counts
   as a capability.
2. [`docs/sdks/parity.md`](../../sdks/parity.md) — the matrix. Two tables:
   the merchant SDKs (`sdks/rust` vs `sdks/nodejs`, one row per capability)
   and `@vpay/stripe-js` (its own table, per ADR-0015 decision 4). Measured
   by reading both SDK trees in full, 2026-09-03.
3. `cargo xtask verify-sdk-parity` (`.xtask/src/main.rs`) — parses every
   markdown table in the matrix whose header starts `Capability`, resolves
   each `✅` cell's named test(s) against that column's directory (Rust
   `#[test]`/`#[tokio::test]`, ignoring `#[ignore]`d ones; TypeScript
   `it("…")`/`test("…")`, ignoring `it.skip`), requires every `⛔` to carry a
   `YYYY-MM-DD`, and fails on any blank cell. Twelve unit tests
   (`sdk_parity_tests` module) prove the parser against a synthetic SDK
   tree, including the revert-proof property, and a thirteenth
   (`the_repositorys_own_matrix_passes`) runs it against this repository's
   real matrix and real SDK sources.
4. Wired into `just verify` (`verify: verify-no-mocks verify-status
   verify-errors verify-sdk-parity`) and into `cargo xtask verify-all`.
5. `docs/status.md`: a paragraph in the header (alongside the
   `verify-status`/`verify-errors` description) and a dated paragraph at the
   end of the "Merchant SDKs" section, both giving the current counts.
   `docs/sdks/README.md` (new), a row in `docs/flows/README.md`, and a
   sentence in the root `README.md`, all pointing at the matrix.

## Verification actually run (not reconstructed)

- `cargo xtask verify-sdk-parity` against the real tree:
  `verify-sdk-parity: ok — 267 proving test(s) named in
  docs/sdks/parity.md all exist, 24 dated gap(s)`.
- `cargo nextest run -p xtask`: **62 tests run: 62 passed, 0 skipped**,
  including all 12 `sdk_parity_tests` and `the_repositorys_own_matrix_passes`.
- **Revert-proof, actually done, not asserted:** renamed
  `mints_an_assertion_with_the_expected_claim_shape` in
  `sdks/rust/src/auth.rs` to
  `mints_an_assertion_with_the_expected_claim_shape_RENAMED`, re-ran
  `cargo xtask verify-sdk-parity`. It failed, naming both matrix cells that
  cite the old name (`docs/sdks/parity.md:31` and `:32`) and the SDK
  directory each is in. Restored the file from a backup, re-ran the check —
  passed again with `git diff` empty on that file.

See the top-level report for `just verify`, `pnpm --filter @vpay/sdk test`,
`cargo nextest run -p vpay-sdk` and `just docs-check` output — run once at
the end of the lane, not per-item, since none of those gates were changed
by this lane's edits to SDK source (none were made — see "What this lane
did not do").

## Gap list (from the matrix's own "Gap ledger" — authoritative copy is there)

| Gap | Where | Found | Owner |
|---|---|---|---|
| No CI-gated real-OP conformance for the Node assertion | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `token_type` is not validated on the token response | `sdks/rust` | 2026-09-03 | SDK maintainers |
| `invalidate()` has no compare-and-swap against the refused token | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| No retry policy beyond the single 401 re-auth | both | 2026-09-03 | SDK maintainers |
| A repeated trailing slash on `base_url` is unproven, and the two differ | both | 2026-09-03 | SDK maintainers |
| No test makes a request timeout fire | `sdks/rust` | 2026-09-03 | SDK maintainers |
| No TLS trust-root control, and no TLS test | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `events.retrieve` is served and neither SDK calls it | both | 2026-09-03 | SDK maintainers |
| A refused amount throws a bare `TypeError`, not a `VpayError` | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `request-id` is not surfaced | both | 2026-09-03 | SDK maintainers |
| `stripe-should-retry` is not read | both | 2026-09-03 | SDK maintainers |
| No assertion that a thrown error cannot carry a token | `sdks/rust` | 2026-09-03 | SDK maintainers |
| `client_secret` is not redacted from `PaymentIntent` diagnostics | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| `exactOptionalPropertyTypes` has no Rust analogue | `sdks/rust` | 2026-09-03 | n/a |
| A verified-but-undecodable body is not a distinct error | `sdks/nodejs` | 2026-09-03 | SDK maintainers |
| No `async-stripe` authenticator (three rows) | `sdks/rust` | 2026-09-03 | SDK maintainers |
| The browser package has never run against a live stack | `sdks/stripe-js` | 2026-09-03 | SDK maintainers |

24 dated cells total (some gaps span more than one column/row; the ledger
above lists 17 distinct findings, several counted twice by the check because
they occupy both columns of a row or several rows).

## What this lane did not do

- **Did not close any gap.** Per the brief ("Do not close gaps here unless
  trivial"), and per ADR-0015's own "What this does not require": the rule
  is that gaps become dated and owned, not that they are zero on adoption.
  None of the 24 found here were trivial — each is either a real behavioural
  difference or a missing test for real behaviour.
- **Did not touch `sdks/rust`, `sdks/nodejs`, or `sdks/stripe-js` source at
  all**, beyond the one rename-then-restore used to prove the check fails
  (reverted; `git diff` on those trees is empty).
- **Did not build a shared cross-language conformance runner** — considered
  and rejected in ADR-0015's "Alternatives considered" as a larger project
  than the gap it would close.
- **Did not implement link-checking for `docs-check`** — that recipe already
  only runs `verify-status` with a note that link checking isn't
  implemented; unrelated to this lane and not touched.

## Rebased onto the Step 8 integration branch, 2026-09-04

`git rebase --onto claude/step8-production-gate 679e65a` — the integration
branch is `master` with Step 7 merged plus lanes D and B (`068d8b7`). Three
files conflicted and were resolved by keeping both sides; nothing from
either side was dropped (checked by diffing the resolved file against each
side and reading every removed line):

- `.xtask/src/main.rs` — Step 7's `verify-docs` command, its `#[from]`
  delegation rule and its `connect_lazy` no-mocks rule are all intact; this
  lane's `verify-sdk-parity` block and `sdk_parity_tests` are unchanged. The
  dispatcher gained one arm, `verify-all` one `.and_then`, and the usage
  string one name. `signing_key_tests::TempDir` stays `pub(crate)` with its
  `path()` accessor, which `sdk_parity_tests` uses.
- `justfile` — `verify` now runs **five** lines: the four gates
  (`verify-no-mocks`, `verify-status`, `verify-errors`, `verify-sdk-parity`)
  and Step 7's advisory `verify-docs` report. `verify-docs` is kept last in
  the dependency list because Step 7's own comment turns on it being last
  and never a gate; the recipe *definitions* are in the order
  `verify-docs`, `verify-sdk-parity`. `verify-ignored`'s three counters are
  lanes B/D's and were not touched.
- `docs/status.md` — Step 7's header note (`verify-errors`' broadened
  delegation rule, the `verify-docs` report and its doc-ratio table) and
  this lane's `verify-sdk-parity` paragraph both kept, in that order. Step
  7's "`just verify` is three gates and one report" now points forward to
  the fourth gate rather than contradicting it.

`docs/flows/README.md` and `README.md` merged without markers; both index
rows are present.

Re-measured on the rebased tree (not carried over from the pre-rebase run):

- `cargo xtask verify-sdk-parity`: `ok — 267 proving test(s) named in
  docs/sdks/parity.md all exist, 24 dated gap(s)` — **no drift**; Step 7
  renamed nothing under `sdks/*`.
- `cargo nextest run -p xtask`: **83 tests run, 83 passed, 0 skipped** (69
  from the integration branch + this lane's 14 `sdk_parity_tests`).
- `just verify`: all five lines, `verify: ok`. `verify-errors` now reports
  15 error types / 14 `#[from]` variants (lanes B/D's additions).
- `cargo nextest run -p vpay-sdk`: **113 passed, 0 skipped**.
  `pnpm --filter @vpay/sdk test`: **150 passed**, 9 files.
- `just docs-check`: `verify-status` ok (link checking still not
  implemented — pre-existing).
- `cargo fmt --all -- --check` and `cargo clippy -p xtask --all-targets --
  -D warnings`: clean.
- **Revert-proof re-run on the merged check, not asserted:** renamed
  `mints_an_assertion_with_the_expected_claim_shape` in
  `sdks/rust/src/auth.rs`; `verify-sdk-parity` failed naming
  `docs/sdks/parity.md:31` and `:32` and the column each is in. Restored,
  `git status` on that file empty, check green again.

**Not re-run for this rebase:** the full `cargo nextest run --workspace`,
`just verify-ignored`, `just ci`, the demo, `sdks/stripe-compat` and
Cypress. This rebase changed no shipping Rust or TypeScript source — only
`.xtask`, the `justfile` and docs — but that is an argument, not a
measurement, and the integration branch's own gates have not been re-run
here.
