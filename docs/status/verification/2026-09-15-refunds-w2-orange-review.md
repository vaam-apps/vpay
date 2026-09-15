# 2026-09-15 — adversarial review of wave 2 / arm E (the Orange capability flip)

A review of `refunds/w2-orange`, landed on `review/w2-orange`. The arm's own
record is
[2026-09-15-refunds-w2-orange-flip.md](2026-09-15-refunds-w2-orange-flip.md);
two corrections were **appended** to it rather than edited into it, and are
repeated below.

`just ci` was **not** run — the brief forbade it. What is below is what was
actually run in this worktree, and nothing else is claimed.

## What the review changed

### 1. Eleven pages the flip falsified and the sweep missed

`verify-links` checks that a link resolves to a tracked path. It has never
checked that the sentence around the link is true, and a capability flip
falsifies sentences, not links. Eleven live pages and doc comments still said
Orange answers `ProviderError::Unsupported`, or declares
`supports_refunds: false`:

| Page                                             | The claim that had stopped being true                               |
| ------------------------------------------------ | ------------------------------------------------------------------- |
| `docs/api/README.md`                             | the `POST /v1/refunds` "not served" row: "Orange Money answers `Unsupported`" |
| `docs/reference/vpay-db/events-refunds-and-webhooks.md` | "`Unsupported` on Orange"                                     |
| `docs/reference/vpay-db/cratestack-money-tables.md`     | the `refunds` table's second blocker                          |
| `docs/flows/invoices.md`                         | "Orange Money answers `Unsupported` (its Web Payment product documents no refund API at all)" |
| `docs/flows/merchant-auth/resource-contract.md`  | why every refund `fee` is `null`                                    |
| `docs/flows/provider-port.md`                    | the dated 2026-09-05 `fee` entry — annotated, not rewritten         |
| `docs/roadmap/phase-4-rail-adapters.md`          | "`orange_money::refund` left the list"; it came back, and the Goal's count is two again |
| `docs/status/backend.md`                         | the rail table's Refunds column, plus two paragraphs                |
| `backends/crates/vpay-provider/src/lib.rs`       | three doc comments, including `is_coherent`'s reason 1 and the paragraph that **predicted its own expiry** and then outlived it unnoticed |
| `sdks/nodejs/src/types.ts`                       | the refund `fee` doc: "Orange has no refund API"                    |
| `sdks/rust/src/model.rs`                         | the same sentence                                                   |

Each carries the sentence it replaced. Deliberately **not** changed:
`docs/plans/`, `docs/adr/`, `docs/status/verification/` and RFC-0003's own
"where we are" survey, whose premise its own § 5 supersedes in the same
document. Those are dated records and rewriting one falsifies it.

The `is_coherent` comment is the one worth remembering: it said in so many
words that its reason 1 "stops being true the day RFC-0003 § 5 flips
`orange_money`'s `supports_refunds`, and nothing fires on _this_ text when it
does". Nothing did. A comment that names its own expiry date is not a gate.

### 2. `verify-status` could not tell which rail owned a token — fixed

The gate compares two **sets of token strings** and knows nothing about where
a token is written, so an adapter answering another rail's token satisfies
both directions. Measured on this tree, in two steps:

| Step                                                                                    | `cargo xtask verify-status`                                                       |
| --------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Orange's `refund` answers `NotImplemented("mtn_momo::refund")`                           | **fails**: "docs/status.md declares these unimplemented items and no shipping code carries them: orange_money::refund" |
| …and then the repair that message invites — delete the Orange bullet                     | **"ok — 1 unimplemented item(s)"**                                                 |

At that point a whole rail's gap has left `docs/status.md` with a green build,
and the adapter blames MTN for it.

**Third direction added.** A token whose prefix names a rail this workspace
ships a `vpay-adapter-*` crate for must be carried by that crate. The rail
codes are derived from the directory names (`vpay-adapter-orange-money` →
`orange_money`), because `xtask` is a text tool that must not link the
workspace it checks. Prefixes naming no rail are deliberately unconstrained:
nothing in this repository says where a `worker::…` token may live, and a gate
inventing that rule would be a rule about characters.

Re-running the original failure against the fixed gate:

```
xtask: these unimplemented items name one rail and are carried by another crate
  (a copy-paste between adapters, which the two directions above cannot see because
  they compare token *strings* and neither knows where a token lives):
  - `mtn_momo::refund` is carried by backends/crates/vpay-adapter-orange-money/src/lib.rs
    — a `mtn_momo::…` token belongs to backends/crates/vpay-adapter-mtn-momo
```

`a_token_naming_another_rail_is_refused_however_well_the_page_matches` pins it.
Deleting the rule fails that test (measured); the same token in its **own**
adapter still passes, so the rule cannot degrade into "refuse every
rail-prefixed token".

The conformance assertion the arm added is kept. It is not redundant — it
covers `refund` on the two configured rails and fails on the mutation at step
one, before the misleading message can send anyone to the status page — but it
is not what `AGENTS.md` points at when it says every token must appear in
`docs/status.md`, and it needs Docker.

### 3. A conformance case whose body had stopped executing — deleted

`refund_is_refused_when_the_capability_is_absent` asserted
`supports_partial_refunds ⇒ supports_refunds` behind an `if !supports_refunds`
guard. After the flip no rail entered its body. The arm documented that in
place; this review deleted the case instead.

**No rule was retired, which is the only reason deleting it was safe:**

- `every_adapter_declares_coherent_capabilities`, forty lines above it in the
  same file, holds the same implication **unconditionally**, on every rail,
  with no guard to go dead;
- `partial_refunds_imply_refunds` in `vpay-provider` pins that
  `Capabilities::is_coherent` really refuses the bad pair, so the case above
  cannot pass by the helper being weakened;
- `partial_refunds_without_refunds_is_rejected_by_the_database` proves
  migration `0002`'s CHECK fires on the same pair.

The deleted case was a strict subset of those three. The reason is recorded on
the surviving case's doc comment, not only here.

`a_rail_without_the_refund_capability_answers_unsupported` was **kept**, and
the distinction matters: its `if` arm runs on both rails and asserts, so only
its `else` is unreachable. A test whose body is dead and a test with one dead
arm are different things.

### 4. The ADR-0006 citation behind the fixture-rail rejection was wrong

The arm rejected adding a fixture rail to the conformance suite and cited
ADR-0006 in `docs/flows/provider-port.md` and `docs/status/backend.md`.
ADR-0006 forbids a double **reachable from `vpay-server` or
`vpay-worker-bin`**; a dev-only fixture in a `tests/` package is reachable from
neither, and `verify-no-mocks` walks non-dev edges from the binaries and would
not have objected.

**The conclusion stands on its other reason, which is the real one:**
`adapters()` is the shipping registry that *every* case in that file iterates,
so a fixture rail would also have to satisfy the wire-level cases,
`adapter_codes_are_unique` and the destination cases — by the time it did, it
would be a rail, and one nobody ships. Both live pages now say that; the dated
record was not rewritten.

### 5. `supports_partial_refunds: false` — right value, incomplete reasoning

The decision is correct and was left alone. Two things were added to the
comment:

- The arm defends `supports_refunds: true` as a claim about the **rail**
  ("Orange makes transfers") and `supports_partial_refunds: false` on
  merchant-visible blast radius. Those are two different decision procedures
  applied to adjacent booleans. The honest statement is that **neither value is
  a true claim about Orange**, because nobody here knows the amount semantics —
  the same modelling gap `Capabilities::is_coherent` already records for
  `refund_destination`, a two-valued field with no spelling for "unknown" — and
  blast radius is the tie-break *after* admitting that, not instead of it.
- The comment said "the core refuses a part-refund on this rail and a merchant
  is told no". **There is no such refusal.** `POST /v1/refunds` is unrouted, and
  `is_coherent` plus the `providers` seed write are the only readers of the flag
  in the entire workspace (grepped). Describing a refusal that does not exist is
  how this repository starts sounding more finished than it is.

The asymmetry argument itself survives the test: withdrawing a capability
merchants have integrated against is breaking, adding one is not.

## Measurements

All on `review/w2-orange`, Rust 1.98.0, `DOCKER_HOST=unix:///run/user/1000/docker.sock`.

| Command                                                                                          | Result                                    |
| ------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `cargo nextest run -p vpay-adapter-orange-money -p vpay-provider -p vpay-tests-conformance -p xtask` | **405 run, 405 passed, 0 skipped, 0 ignored** |
| `cargo nextest run -p vpay-tests-conformance` (containers, alone)                                 | 56 run, 56 passed, **0 skipped**           |
| `cargo nextest run -p vpay-adapter-orange-money`                                                  | 66 run, 66 passed, 0 skipped               |
| `cargo nextest run -p vpay-provider`                                                              | 33 (of the 99 with the adapter), 0 skipped |
| `cargo nextest run -p xtask`                                                                      | 250 run, 250 passed, 0 skipped             |
| `cargo test --doc -p vpay-adapter-orange-money`                                                   | 1 passed, 0 ignored                        |
| `cargo test --doc -p vpay-provider`                                                               | 12 passed, 0 ignored                       |
| `cargo test --doc -p vpay-db`                                                                     | **8** passed, 0 ignored                    |
| `cargo test --doc -p vpay-api`                                                                    | **18** passed, 0 ignored                   |
| `cargo clippy -p vpay-adapter-orange-money -p vpay-provider -p vpay-tests-conformance -p xtask --all-targets --all-features -- -D warnings` | clean |
| `cargo fmt --all -- --check`                                                                      | clean                                      |
| `cargo xtask verify-status`                                                                       | **2 unimplemented item(s)**, both declared |
| `cargo xtask verify-links`                                                                        | 1 660 links, 364 files                     |
| `cargo xtask verify-no-mocks`                                                                     | clean                                      |

The conformance suite went 57 → 56 with the deleted case. Nothing was skipped
and nothing is `#[ignore]`d: the containers really started, which is the only
reason "56 passed" means anything.

**Not run, and therefore not claimed:** `just ci`, the integration suite, the
workspace-wide doctest sweep, `verify-docs`, `prettier`/`just fmt-check-web`,
and the remaining nine gates in `just verify`. CI is the gate for all of them.
In particular the two SDK sources touched here are comment-only changes that
`prettier` has not been run over in this worktree.

## Mutations

Each applied to the shipping source, the named command run, the source
restored with `git checkout`.

| Mutation                                                              | Caught by                                                                                                                                                                                                       |
| ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Orange's `refund` answers `Err(ProviderError::Unsupported)`           | `refund_is_a_token_about_vpay_not_an_answer_about_orange` **fails**; `a_rail_without_the_refund_capability_answers_unsupported::case_2_orange_money` **fails**; `verify-status` **fails** (declared, not carried) |
| Orange's `refund` answers `NotImplemented("mtn_momo::refund")`        | conformance **fails** on the own-rail assertion; `verify-status` **fails** on the declared-not-carried direction                                                                                                 |
| …and the Orange bullet then deleted from `docs/status.md`             | **before this review: `verify-status` printed "ok — 1 unimplemented item(s)"**. After: fails, naming the token, the file carrying it and the crate that owns the prefix                                          |
| The new third direction deleted from `verify_status`                  | `a_token_naming_another_rail_is_refused_however_well_the_page_matches` **fails**                                                                                                                                 |

## What this review did not settle

- **The load-bearing claim of the whole arm** — that no Orange transfer API is
  documented anywhere in this repository — was spot-checked by grep, not
  exhaustively re-derived. If one exists and was missed, the token is wrong and
  a call should have been written.
- **`supports_partial_refunds` has no honest value** on a rail whose transfer
  semantics nobody here knows. Whether `Capabilities` should grow an
  `Option`-shaped spelling for "unknown" — the same question
  `Capabilities::is_coherent` already parks for `refund_destination` — is a
  modelling decision for the maintainer, not a review finding.
- MTN declares `supports_partial_refunds: true` while `mtn_momo::refund` is
  also an unbuilt token. That rests on Disbursements being a **documented**
  product, which is the distinction that makes Orange's `false` different
  rather than inconsistent — but it is an asymmetry a maintainer may want to
  look at rather than one this review resolved.
