# Refunds wave 2 — Orange's capability flip

**Date:** 2026-09-15. **Branch:** `refunds/w2-orange`, off
`origin/claude/orange-refund-transaction-2b95e5` (`4caf7648`), which already
carries waves 1 and 1b and their adversarial reviews. **Host:** Linux, Rust
1.98.0 (`rust-toolchain.toml`), rootless Docker
(`DOCKER_HOST=unix:///run/user/1000/docker.sock`) for the WireMock containers
the conformance suite starts.

Scope was RFC-0003 **§ 5**, Orange half: the maintainer's decision that an
Orange refund **is** a transfer back. **No refund is any closer to working** —
see "What this did not do", which is the more important half of this page.

## What changed

| Before                                                    | After                                                               |
| --------------------------------------------------------- | ------------------------------------------------------------------- |
| `supports_refunds: false`                                 | `supports_refunds: true`                                            |
| `refund` inherits the port's `ProviderError::Unsupported` | `refund` overrides it with `NotImplemented("orange_money::refund")` |
| `verify-status`: 1 unimplemented item                     | `verify-status`: **2**, both declared                               |
| `supports_partial_refunds: false` (implied)               | `supports_partial_refunds: false` (**decided** — see below)         |

`refund_destination` was already `Required` and did not move.
`parse_destination` was already built (wave 1b) and did not move. That is why
this was a declaration and not a design: wave 1b deliberately declared the
destination truthfully while the flag was still `false`, so the flip cost one
line about refunds and no new guess about payees.

### `supports_partial_refunds` stays `false`, and it is a decision

`partial_refunds_imply_refunds` (migration `0002`) and
`Capabilities::is_coherent` both permit `true` now. Permitted is not decided.

The `true` above rests on exactly one known thing — that an Orange refund is a
transfer. Nothing else about Orange transfers is known in this repository: no
endpoint, no body, no amount semantics, no minimum, no per-transfer limit.
"Any amount up to the charge" is a property of a transfer product, and this
repository has never seen Orange's.

So the merchant-visible direction settles it. `false` refuses a part-refund and
tells the merchant so. `true` accepts one, and if the real product turns out to
reverse whole payments only, or to floor at some amount, a capability merchants
have already integrated against has to be **withdrawn** — a breaking change
made on a guess. `false` → `true` is additive and costs nobody anything.
Flipping it belongs to whoever writes the transfer call against a real
specification.

## What this did not do

- **No Orange transfer wire call**, and this is the point of the arm rather
  than an omission from it. This repository documents no Orange transfer API —
  not even reconstructed. The three implemented calls came from Orange
  Developer's public overview plus several community SDKs that agree with each
  other; for transfers no such source exists here, so an endpoint path and a
  request body would be **invented**, in the money path, on a rail nobody has
  ever called. `CLAUDE.md` names that failure mode first.
- **No core change.** `POST /v1/refunds` is still unrouted (wave 3), nothing
  reads `refund_destination`, nothing writes a `refunds` row, no ledger posting
  is made. A merchant sees no difference whatsoever.
- **No MTN change.** `mtn_momo::refund` is untouched.
- **Orange has still never been called.** Every assertion below was made
  against WireMock.

What unblocks the token is item 5 of `docs/flows/adapter-orange-money.md`'s "To
confirm with Orange Cameroun" list, rewritten by this arm from "Refund/
disbursement availability" — a question RFC-0003 § 5 settled — into the
specification request that is now the only item on that list blocking a
shipping token.

## The tripwire, and the case that lost its rail

### `a_refund_on_this_rail_would_need_a_payee` fired as designed

Its failure message was a checklist of prose no compiler checks. All of it was
worked: `Adapter::new`'s doctest, the `supports_refunds` row and the paragraph
under it in `docs/flows/provider-port.md`, and `docs/status.md`'s
`NotImplemented` list. The `!supports_refunds` line was dropped and the
checklist is now restated in the case's own doc comment, so that a tripwire
deleted without a trace stays distinguishable from one deleted to go green. The
`refund_destination` assertion — the one the message told the reader **not** to
drop — is untouched.

### `a_rail_without_the_refund_capability_answers_unsupported` has no rail for its `false` arm

`orange_money` was the workspace's only rail declaring `supports_refunds:
false`. Both rails now declare `true`, so that arm executes on nothing.

**It was not deleted.** It is kept as a written assertion, exactly as that
suite's `Origin` arm in `a_required_rail_parses_its_own_destination` is kept, so
that a rail added or flipped tomorrow is checked rather than silently skipped.

**Keeping it is not the same as still proving it**, so the property it
exercised — the port's `refund` default is `Unsupported`, not `NotImplemented`
— moved to `a_rail_with_no_refund_api_takes_the_default_and_answers_unsupported`
in `vpay-provider`, on a stub that overrides nothing optional (the same pattern
as the existing `an_origin_rail_takes_the_default_and_answers_unsupported`).

**What did not move, stated plainly:** the conformance version proved it
through a real adapter against a real container. The unit test proves the
trait's default body. There is no longer any rail in this workspace that can
prove the configured-rail half, and there will not be until a rail with no
refund API is added.

A fixture rail inside the conformance suite was considered and **rejected**:
that suite is one body parameterised over the workspace's _real_ adapters
against real WireMock containers (ADR-0006), `adapters()` enumerates what
ships, and a rail invented to keep an arm running is a rail nobody ships. That
is a judgement, not a rule, and it is recorded here so it can be overruled.

The case keeps its name even though the name now describes the arm that does
not run: `docs/status.md`, `docs/flows/provider-port.md`,
`docs/status/backend.md` and the dated
`verification/2026-09-15-refunds-w1-provider-port.md` all cite it by name, and
that last one must not be rewritten.

### The live arm got stronger

It is the assertion that catches this exact change half-done. It now also
checks that an unbuilt refund's token names **its own** rail — `verify-status`
matches `docs/status.md`'s bullets against these strings verbatim and cannot
tell which rail a token came from, so a copy-pasted `mtn_momo::refund` inside
the Orange adapter would have kept that gate green while the status page named
the wrong rail's gap.

## Measurements

All run on `refunds/w2-orange`, Rust 1.98.0.

| Command                                                                                                             | Result                                        |
| ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| `cargo nextest run -p vpay-adapter-orange-money -p vpay-tests-conformance`                                          | **123 run, 123 passed, 0 skipped, 0 ignored** |
| `cargo nextest run -p vpay-provider`                                                                                | **33 run, 33 passed, 0 skipped, 0 ignored**   |
| `cargo test --doc -p vpay-adapter-orange-money`                                                                     | 1 passed, 0 ignored                           |
| `cargo test --doc -p vpay-provider`                                                                                 | 12 passed, 0 ignored                          |
| `cargo clippy -p vpay-adapter-orange-money -p vpay-provider -p vpay-tests-conformance --all-targets -- -D warnings` | clean                                         |
| `cargo xtask verify-status`                                                                                         | **2 unimplemented item(s)**, both declared    |
| `cargo xtask verify-links`                                                                                          | clean                                         |

The adapter crate's own count went 65 → 66: one case removed (the tripwire
line was an assertion inside an existing case, so no case was lost) and one
added, `refund_is_a_token_about_vpay_not_an_answer_about_orange`. The
conformance suite's count is unchanged at 57 per its own binary; nothing was
added or removed there, only strengthened.

`just ci` was **not** run — the brief forbade it (five concurrent local builds
have OOM-killed this host). CI is the gate for everything outside the table
above, and in particular the integration suite, the doctest sweep across the
workspace and `verify-docs` were not run here.

## Mutation testing

Each mutation was applied to the shipping source, the named command run, and
the source restored.

| Mutation                                                                                                 | Caught by                                                                                                                                                                                                                                                                                                  |
| -------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `refund` answers `Err(ProviderError::Unsupported)` — i.e. the flag is flipped and the override forgotten | `refund_is_a_token_about_vpay_not_an_answer_about_orange` **fails**; `a_rail_without_the_refund_capability_answers_unsupported::case_2_orange_money` **fails**; `verify-status` **fails** with `docs/status.md declares these unimplemented items and no shipping code carries them: orange_money::refund` |
| `refund` answers `NotImplemented("mtn_momo::refund")` — a copy-paste from the other adapter              | `a_rail_without_the_refund_capability_answers_unsupported::case_2_orange_money` **fails** on the new own-rail assertion. `verify-status` stays green, which is exactly why that assertion exists                                                                                                           |

The second one is the finding worth keeping: the gate this arm's token is
tracked by is blind to which rail declared it.

## What a reviewer should attack

- **The `supports_partial_refunds: false` decision.** It is a judgement about
  merchant-visible blast radius, not a fact, and the argument above is the
  whole of it.
- **Keeping `a_rail_without_the_refund_capability_answers_unsupported`'s dead
  arm rather than building a fixture rail.** The alternative was considered and
  rejected on ADR-0006 grounds; a reviewer who thinks a test-only rail in the
  conformance package is worth its cost should say so.
- **The claim that no Orange transfer API is documented in this repository.**
  It is the load-bearing claim of the whole arm. If one exists and was missed,
  the token is wrong and a call should have been written.
