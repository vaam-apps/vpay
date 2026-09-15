# Refunds wave 1 — the refund destination in the provider port

**Date:** 2026-09-15. **Branch:** `refunds/w1-port`, off
`origin/claude/orange-refund-transaction-2b95e5` (`c7f06382`). **Host:** Linux,
Rust 1.98.0 (`rust-toolchain.toml`), rootless Docker 29.7.2 for the
WireMock containers the conformance suite starts.

Scope was §1 of
[RFC-0003](../../rfc/0003-refunds-destinations-and-the-first-ledger-postings.md)
and nothing else: the port learns to carry a refund destination. **No refund
is any closer to working** — see "What this did not do" below.

## Gates run

`just ci` was **not** run, by instruction (five concurrent local builds have
OOM-killed this host). The narrow commands for the crates touched were:

| Gate                                                           | Result                                                             |
| -------------------------------------------------------------- | ------------------------------------------------------------------ |
| `cargo nextest run -p vpay-provider -p vpay-tests-conformance` | **78 passed, 0 skipped, 0 ignored** (zero `FAIL` lines in the log) |
| `cargo clippy -p vpay-provider --all-targets -- -D warnings`   | exit `0`, no warnings                                              |

Beyond the two mandated commands, because this change edits files they do not
cover and a break there is a CI failure the next arm inherits:

| Gate                                                                      | Result                                                                                                                     |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `cargo test --doc -p vpay-provider`                                       | **11 passed, 0 failed, 0 ignored** — `RefundTarget::msisdn`'s example is one of them, and `cargo nextest` runs no doctests |
| `cargo nextest run -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money` | **121 passed, 0 skipped**                                                                                                  |
| `cargo check -p vpay-api --all-targets`                                   | exit `0`                                                                                                                   |
| `cargo check -p vpay-tests-integration --all-targets`                     | exit `0`                                                                                                                   |
| `cargo fmt --all -- --check`                                              | exit `0`                                                                                                                   |
| `pnpm exec prettier --check docs/flows/provider-port.md`                  | clean (it was clean before the edit, checked)                                                                              |

Not run here, and therefore not claimed: `just verify`'s twelve self-checks
(`verify-serde`, `verify-status`, `verify-links`, `verify-docs`, …), the
integration suite against a real Postgres, the web gates, and `just ci`. CI is
the gate for those.

## The ADR-0002 check the brief named

`grep -rn "if.*provider.*==\|match.*code()" backends/crates/vpay-api/src/`
returns the same two lines as at the base commit — both of them prose in doc
comments, neither a branch. The diff against `c7f06382` under
`backends/crates/vpay-api/src/` adds **no** line matching that pattern. The
core branches on the capability value; `RefundDestination` is what the wave-3
handler will read.

## What this did not do

- No `POST /v1/refunds`, no handler, no route constant (wave 3).
- No MTN Disbursements `transfer`; `mtn_momo::refund` is still its
  `NotImplemented` token.
- No change to `orange_money`'s `supports_refunds`, and no `refund` override
  on it — that flip is another arm's and RFC-0003 § 5 says what it owes.
- No ledger posting, no `refunds` row written anywhere.
- **No coherence rule and no migration.** See below.

## The coherence decision, recorded

A coherence rule pairing `refund_destination` with `supports_refunds` was
considered and **refused**. The obvious candidate — "a rail that cannot refund
declares `Origin`" — is false on a live rail today: `orange_money` declares
`Required` with `supports_refunds: false`, because an Orange refund is an
outbound transfer and it is vpay that has not built it. Neither value means
anything while refunds are off, so the rule would be picking an arbitrary
sentinel; and `Capabilities::is_coherent` is one half of a pair whose other
half is a CHECK on `providers`, a table this capability deliberately has no
column in (the `supports_account_holder_lookup` precedent).

`a_refund_destination_is_inert_to_coherence` in `vpay-provider` pins that
decision, over all four combinations of the two flags, and says what would have
to change — the column and the migration — before it may be reversed.

## Amended the same day by an adversarial review (conventions, blast radius)

Branch `review/w1-port-conventions`. The correctness and privacy review is
recorded in the section below this one.

**Confirmed, independently of the report above.** The brief's decisive grep
returns the same two lines under `backends/crates/vpay-api/src/` as at the
base commit, both prose in doc comments; the diff adds none. Every
`ProviderAdapter` implementor in the workspace is accounted for — the two
adapters, `Measured`, and four `#[cfg(test)]` fixtures — and nothing outside
the test trees constructs a `Capabilities`. `RefundTarget` is modelled on
`AccountHolder` (private field, redacting `Debug`, `# Errors: None` carrying
the caller's obligation) and that is this crate's practiced convention, not an
invention. `RefundDestination` is deliberately not `#[non_exhaustive]`, which
matches what `vpay_api::ApiError` and `vpay_worker`'s error enum say in so
many words about workspace-internal enums.

**Changed by the review.**

- `vpay-adapter-orange-money`'s `Adapter::new` doctest still said Orange
  "documents no refund API … the port's default `Unsupported` is the
  permanent, correct answer", three hundred lines above a new
  `capabilities()` comment saying the opposite. The value is unchanged and
  still `false`; the _reason_ now matches the maintainer's 2026-09-15 decision
  and RFC-0003 § 5.
- `a_refund_on_this_rail_would_need_a_payee`'s failure message named "Arm E",
  a word that appears nowhere else in this repository and that no reader can
  resolve, and carried a fourteen-space typo `cargo fmt` cannot see. It now
  names RFC-0003 § 5 and lists the prose claims that must move with the flag.
- `RefundTarget`'s doc claimed opacity "in `RefExtra`'s sense". `RefExtra` is
  untyped and adapter-constructed; a destination is **core**-constructed, so a
  non-mobile-money upstream costs a core change (not a rail-code branch —
  ADR-0002 still holds). The doc now says that, and names the alternative
  (`parse_callback`'s symmetry) without deciding it: it is RFC-0003 open
  question 3.
- `is_coherent`'s first reason for owing no coherence rule stops being true
  when RFC-0003 § 5 flips `orange_money`'s flag. The doc now says the decision
  survives on reasons 2 and 3, and that reason 2 is really an argument about
  the field's type (`Option<RefundDestination>`), which nothing here decided.
- `docs/flows/provider-port.md` § Status gained the list of claims on that
  page that go stale, and which single test guards one of them.

**Left deliberately.** The three doc edits the report flagged as scope creep
are kept: `docs/status.md` § "Where a new row goes" sends a new capability to
`status/backend.md` and gate output to a dated page, and `CLAUDE.md` step 3
requires the `docs/flows/*.md` **Status** section, so reverting them would
break the repository's own rule rather than avoid a conflict.
`docs/status/README.md`'s "verification log, newest first" is **not** amended
here: it already omits the three 2026-09-14 pages that merged before this
work, and six arms each inserting at the head of one list is a six-way
conflict. That list is the seam owner's, in one pass, and the omission is
pre-existing rather than introduced by this arm.

### Gates re-run after the review's edits

| Gate                                                                                      | Result                              |
| ----------------------------------------------------------------------------------------- | ----------------------------------- |
| `cargo nextest run -p vpay-provider -p vpay-adapter-orange-money`                         | **82 passed, 0 skipped, 0 ignored** |
| `cargo test --doc -p vpay-provider -p vpay-adapter-orange-money`                          | **12 passed, 0 failed, 0 ignored**  |
| `cargo clippy -p vpay-provider -p vpay-adapter-orange-money --all-targets -- -D warnings` | exit `0`                            |
| `cargo fmt --all -- --check`                                                              | exit `0`                            |
| `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc -p vpay-provider --no-deps`  | exit `0`                            |
| `cargo check --all-targets` for every crate depending on `vpay-provider`                  | exit `0`                            |
| `cargo xtask verify-serde`                                                                | ok — 92 types, 16 exempted          |
| `cargo xtask verify-errors`                                                               | ok — 19 error types, all classified |
| `cargo xtask verify-status`                                                               | ok — 1 unimplemented item           |
| `cargo xtask verify-links`                                                                | ok — 1621 links in 358 files        |

`just ci` was not run, by instruction. The conformance suite was compiled
(`cargo check -p vpay-tests-conformance --all-targets`, exit `0`) and **not**
executed by this review — its WireMock containers are the arm's own evidence
above, not re-measured here. `pnpm exec prettier --check` could not run: the
web dependencies are not installed in this worktree. `proseWrap` is
`"preserve"` in `.prettierrc.json` and no table was touched, so the markdown
edits are reflow-free, but that is reasoning rather than a gate. CI is the
gate for both.

## Correctness-and-privacy review, 2026-09-15 (`review/w1-port-correctness`)

An adversarial re-run of the claims above, on a second branch off
`refunds/w1-port`. Every number here was measured on this host with rootless
Docker 29.7.2 (`DOCKER_HOST=unix:///run/user/1000/docker.sock`), not copied.

### The claims, re-measured

| Claim                                                                       | Result                                                                                                                                                                                  |
| --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-provider -p vpay-tests-conformance`              | **true** — 78 run, 78 passed, 0 skipped. No `#[ignore]` exists in either package                                                                                                        |
| The conformance cases ran against real containers                           | **true** — 1.1–3.5 s per wire-level case; `start_wiremock` returns a `Result` and has no skip-on-missing-daemon path, so a dead daemon would have failed the run rather than passing it |
| `cargo nextest run -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money`   | **true** — 121 run, 121 passed, 0 skipped                                                                                                                                               |
| `cargo test --doc -p vpay-provider`                                         | **true** — 11 passed, 0 ignored                                                                                                                                                         |
| `RefundTarget` has no `Serialize`/`Deserialize` and nothing else renders it | **true** — `Debug` is the only `impl` on the type anywhere in the workspace; nothing embeds it; `Measured` derives no label from it                                                     |
| The ADR-0002 grep gains no rail-code branch                                 | **true** — the two matches under `vpay-api/src/` are prose in doc comments                                                                                                              |

### Mutations run

| Mutation                                                          | Caught as delivered?                                                                                                       |
| ----------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `ProviderAdapter::refund`'s default returns `Ok(Refunded { .. })` | **yes** — `a_rail_without_the_refund_capability_answers_unsupported::case_2_orange_money` fails naming the fabricated `Ok` |
| `RefundTarget`'s `Debug` prints `self.msisdn`                     | **yes** — the unit case and the `msisdn` doctest both fail (two runners)                                                   |
| `mtn_momo::refund` returns `Ok(Refunded { .. })`                  | **yes** — `refund_is_not_implemented_and_does_not_pretend` fails                                                           |
| `Measured::refund` forwards `None` instead of the destination     | **NO** — 199 tests across four packages stayed green                                                                       |
| `destination_for(Required)` returns `None`                        | **NO** — all 54 conformance cases stayed green                                                                             |
| `destination_for(Origin)` returns `Some(..)`                      | **NO** — same                                                                                                              |

The last three are one failure with one cause: **no adapter reads the
`destination` argument yet**, so every test that passes one passes it into a
body that discards it, and nothing observes what arrived. That is tolerable in
an argument nobody reads — except that `Measured` is the decorator
`vpay_api::v1::boot::adapters_by_code` wraps **every** shipping adapter in, so
the dropped-destination mutation is one that would reach production and
address every refund on a `Required` rail to nobody.

### What the review changed

- `vpay-provider/src/measured.rs`:
  `the_destination_reaches_the_inner_adapter_and_never_a_metric`, with a
  recording inner adapter. It asserts the destination arrives unchanged, that
  the call is still counted on the `refund` series, and that the payee's
  number appears nowhere in a real Prometheus scrape.
- `backends/tests/conformance/tests/adapter_conformance.rs`: the per-rail
  invariant — a payee is supplied exactly when the rail declares `Required` —
  is now asserted against the adapter's own capability inside
  `a_rail_without_the_refund_capability_answers_unsupported`, and
  `a_destination_is_offered_exactly_when_the_capability_demands_one` pins the
  helper's `Origin` arm, which no rail in this workspace reaches.
- `vpay-adapter-orange-money/src/lib.rs`: a panic message carried fourteen
  stray spaces from a line wrap, and it and the doc comment above it named an
  orchestration arm a reader of this repository cannot resolve. Reworded to
  cite RFC-0003 § 5, which they already cited. No assertion changed.

After the change each of the three surviving mutations fails: the `Measured`
one on the new case, `destination_for(Required) => None` on three cases, and
`destination_for(Origin) => Some(..)` on the new conformance case.

### Gates re-run on the review head

| Gate                                                                      | Result                    |
| ------------------------------------------------------------------------- | ------------------------- |
| `cargo nextest run -p vpay-provider -p vpay-tests-conformance`            | **80 passed, 0 skipped**  |
| `cargo nextest run -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money` | **121 passed, 0 skipped** |
| `cargo test --doc -p vpay-provider`                                       | **11 passed, 0 ignored**  |
| `cargo clippy -p vpay-provider --all-targets -- -D warnings`              | exit `0`                  |
| `cargo clippy -p vpay-tests-conformance --all-targets -- -D warnings`     | exit `0`                  |
| `cargo clippy -p vpay-adapter-orange-money --all-targets -- -D warnings`  | exit `0`                  |
| `cargo fmt --all -- --check`                                              | exit `0`                  |

Not run here, and therefore not claimed: `just ci`, `just verify`'s
self-checks, the integration suite, and the web gates. CI is the gate for
those.

### Left alone, deliberately

- **The invariant the port documents is still owed by a core that does not
  exist.** `ProviderAdapter::refund`'s doc says `destination` is `Some`
  exactly when the rail declares `Required`; nothing enforces that, because no
  core path creates a refund. Wave 3 is where it becomes enforceable, and
  writing a check now would be checking a caller nobody has written.
- **Nothing stops a future `#[derive(Serialize)]` on `RefundTarget`.** The
  absence is a true statement about today's source and a doc comment saying
  why, not a gate: `verify-serde` checks that a serde type spells
  `rename_all`, not that a type has no serde impl. Recorded rather than
  fixed — the machinery to assert a negative impl is more than this is worth
  while RFC-0003 open question 2 is undecided, and question 2 is what would
  settle it.
