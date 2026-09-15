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
