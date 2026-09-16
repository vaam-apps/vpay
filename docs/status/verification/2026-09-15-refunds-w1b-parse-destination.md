# Refunds wave 1b — the adapter parses its own destination

**Date:** 2026-09-15. **Branch:** `refunds/w1b-parse-destination`, off
`origin/claude/orange-refund-transaction-2b95e5` (`8494e89e`), which already
carries wave 1 and both of its adversarial reviews. **Host:** Linux, Rust
1.98.0 (`rust-toolchain.toml`), rootless Docker 29.7.2
(`DOCKER_HOST=unix:///run/user/1000/docker.sock`) for the WireMock containers
the conformance suite starts.

Scope was RFC-0003 **open question 4**, which the maintainer decided on
2026-09-15: the adapter owns the destination's wire shape, not the core. **No
refund is any closer to working** — see "What this did not do".

## What changed

- `ProviderAdapter::parse_destination` — a `&serde_json::Map<String, Value>`
  in, a `Result<RefundTarget, ProviderError>` out, synchronous, with a default
  body of `ProviderError::Unsupported`.
- Both adapters override it; each spells its own key in its own crate
  (`DESTINATION_MSISDN_KEY`, private, one per adapter). The two agreeing on
  `msisdn` today is a coincidence, and deliberately not factored into a shared
  helper — the next rail's different key must be a change to that rail.
- `Measured` forwards it.
- The port's error-surface table gained a `parse_destination` column.

### Why there is a default, when `parse_callback` has none

Every rail has a callback shape, so leaving `parse_callback` abstract costs an
adapter nothing it did not already owe. A destination shape is conditional on
`Capabilities::refund_destination`: a rail declaring `Origin` has no
destination to parse at all. A required method would make every such adapter
hand-write the same `Err(ProviderError::Unsupported)` — and give it the
opportunity to write `Ok` of an invented payee instead. `Unsupported` is also
the shape `refund` and `account_holder_name` already have, and means the same
thing: a permanent capability answer the core is supposed to have branched on
first.

### What `raw` is

The **rail-scoped inner map** — `destination[<rail_code>]` with the rail code
stripped. The outer key is vpay's own envelope (it is a `code()`, core
knowledge by definition, spelled the same way as the confirm path's
`payment_method_data[<code>]`), so a `destination` naming no rail is the
core's refusal. Everything inside is the rail's.

## Gates run

`just ci` was **not** run, by instruction (five concurrent local builds have
OOM-killed this host). The narrow commands for the crates touched:

| Gate                                                                                                                                         | Result                                                                                                             |
| -------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `cargo nextest run -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-tests-conformance`                         | **216 run, 216 passed, 0 skipped, 0 ignored** — 28 + 68 + 63 + 57                                                  |
| `cargo test --doc -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money`                                                    | **13 passed, 0 failed, 0 ignored** (11 + 1 + 1) — `cargo nextest` runs no doctests                                 |
| `cargo clippy -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-tests-conformance --all-targets -- -D warnings` | exit `0`                                                                                                           |
| `cargo +nightly fmt` on the four packages, `-- --check`                                                                                      | exit `0`                                                                                                           |
| `cargo check -p vpay-api --all-targets`                                                                                                      | exit `0` — the port trait changed, and `vpay-api` holds four `#[cfg(test)]` implementors that take the new default |
| `cargo xtask verify-status`                                                                                                                  | ok — 1 unimplemented item (unchanged; this change adds no `NotImplemented` token)                                  |
| `cargo xtask verify-links`                                                                                                                   | ok — and it earned its keep: it failed first on the link to **this page** before it existed                        |

The conformance run really started containers: the first attempt failed with
`PermissionDenied` on `/var/run/docker.sock` and had to be re-run with
`DOCKER_HOST` pointed at the rootless socket. A suite that skipped on a dead
daemon would have printed `ok` instead.

Not run here, and therefore not claimed: `just ci`, the rest of `just
verify`'s self-checks, the integration suite against a real Postgres, and the
web gates. CI is the gate for those.

## Mutations run

| Mutation                                                                                                                          | Caught?                                                                                                                                                                                       |
| --------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **The brief's decisive one:** `parse_destination` returns `Ok(RefundTarget::mobile_money(""))` for an empty map, in both adapters | **yes** — `a_destination_missing_this_rails_key_is_malformed` fails in each crate (mtn 67 passed/1 failed; orange 62 passed/1 failed)                                                         |
| Delete `mtn_momo`'s `parse_destination` override, so it inherits the port's default                                               | **yes** — `a_required_rail_parses_its_own_destination::case_1_mtn_momo` fails naming `Unsupported`; `case_2_orange_money` still passes, so the case is per-rail and not global                |
| Add `impl Display for RefundTarget`                                                                                               | **yes** — `a_refund_destination_has_no_impl_but_the_redacting_debug` fails; it is a `const` block, so the failure is a **build** failure of `vpay-provider`'s test target, not a run-time one |

The `Measured` forward has its own guard,
`a_defaulted_method_is_not_silently_answered_by_the_wrapper`: deleting
`Measured::parse_destination` makes the wrapper answer the port's default. It
is written against the wave-1 review's measurement that dropping the
`destination` in `Measured::refund` left 199 tests green — a defaulted method
is the same trap one step earlier, and it is the trap that reaches production,
because `vpay_api::v1::boot::adapters_by_code` wraps every shipping adapter.

## `RefundTarget` still has exactly one impl, and that is now enforced

The wave-1 correctness review recorded, under "Left alone, deliberately", that
_"nothing stops a future `#[derive(Serialize)]` on `RefundTarget`… the
machinery to assert a negative impl is more than this is worth"_. This change
builds that machinery, because `parse_destination` puts a raw payee number in
scope in two more crates and the cost changed.

`a_refund_destination_has_no_impl_but_the_redacting_debug` uses a compile-time
probe — an inherent associated const whose `impl` is bounded on the trait,
falling back to a trait const of `false`, which is `true` exactly when the
`impl` exists — and asserts, in `const` blocks, that `RefundTarget` implements
none of `Display`, `Serialize`, `DeserializeOwned`. `String` is asserted to
trip all three in the same case, so a probe that silently stopped detecting
anything fails rather than passing vacuously. Stable Rust has no negative
trait bound; this is the closest thing that is not a comment.

Each is a distinct leak: a `Display` is printed by `tracing::info!(%…)` and by
`{}`, neither of which the redacting `Debug` intercepts; a `Serialize` puts the
number one derive from a webhook payload and a persisted column while
RFC-0003 open question 2 (retention) is undecided; a `Deserialize` is the same
question read backwards.

Separately, every adapter refusal is asserted not to echo the value — the
test feeds a number under the **wrong key**, so the assertion cannot pass
merely because the number never reached the function.

## MSISDN validation: not moved, not copied, and left open

The brief said to reuse the confirm path's canonicalisation rather than write
a second spelling, and to **say so** if there is no shared helper. There is
not, and the shape of the gap is worth stating precisely because it is easy to
get backwards:

- The **confirm path does not canonicalise.**
  `vpay_api::v1::payment_intents`' `payer_instrument` does
  `data.get(code) → as_object → get("msisdn") → as_str → filter(!trim().is_empty())`
  and hands the untrimmed string to the rail. That is the whole rule for a
  payer's number.
- `vpay_api::v1::account_holders::canonical_msisdn` — the strict E.164 rule
  (`+2376XXXXXXXX` / `2376XXXXXXXX` / `6XXXXXXXX`, a 32-char input bound, a
  fixed separator set) — is used by `GET /v1/account_holders` and by the
  customer object, **not** by confirm.
- It is `pub(crate)` to `vpay-api`. The adapter crates depend on `vpay-core`
  and `vpay-provider` only and sit below `vpay-api`; they cannot name it.

So `parse_destination` applies the confirm path's rule: present, a JSON
string, not whitespace-only. A number vpay accepts as a payer it accepts as a
payee, and `not a phone number` is accepted by both and refused by the rail.
Writing a second spelling of `canonical_msisdn` inside each adapter was
declined outright; **moving** it was also declined, because that changes where
the confirm path validates and puts a Cameroon-specific rule
(`CM_COUNTRY_CODE`) into a market-agnostic crate — a decision about a rule's
home, not about refunds. It is recorded in
[../backend.md](../backend.md) § Adapters as a maintainer decision rather than
taken here.

One deliberate deviation from `payer_instrument`: the accepted value **is**
trimmed before it becomes a `RefundTarget`, where `payer_instrument` tests
`trim().is_empty()` and then stores the untrimmed string. A payee is
interpolated into a transfer body rather than compared to anything of ours,
and leading whitespace in it is never intentional. It is named in both
adapters' doc comments.

## What this did not do

- No `POST /v1/refunds`, no handler, no `V1Route` entry (wave 3 owns the
  caller). **Nothing calls `parse_destination` outside tests.**
- No core code reads `Capabilities::refund_destination` yet, so the
  "`Some` exactly when `Required`" invariant the port documents is still owed
  by a caller that does not exist.
- No MTN Disbursements `transfer`; `mtn_momo::refund` is still its
  `NotImplemented` token. No Orange transfer call and **no change to
  `orange_money`'s `supports_refunds`**, which is still `false`.
- No ledger posting and no `refunds` row written anywhere.
- `RefundTarget::mobile_money` was **not** removed. It is now the constructor
  each adapter's `parse_destination` calls, which is more callers than it had
  before, not fewer.
- RFC-0003 itself was not edited: § 1 and open question 4 already describe
  what this implements.
