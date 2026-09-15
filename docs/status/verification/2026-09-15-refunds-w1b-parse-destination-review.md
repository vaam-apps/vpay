# Refunds wave 1b — adversarial review of `parse_destination`

**Date:** 2026-09-15. **Branch:** `review/w1b-parse-destination`, off
`origin/refunds/w1b-parse-destination` (`0917ce43`). **Host:** Linux, Rust
1.98.0 (`rust-toolchain.toml`), rootless Docker 29.7.2
(`DOCKER_HOST=unix:///run/user/1000/docker.sock`).

Reviews
[2026-09-15-refunds-w1b-parse-destination.md](2026-09-15-refunds-w1b-parse-destination.md).
`just ci` was **not** run, by instruction; the narrow commands below are what
was run, and CI is the gate for everything else.

## Every claim the arm made, re-run

| Claim                                                              | Re-run result                                                                                                                                                                                       |
| ------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `216 run, 216 passed, 0 skipped, 0 ignored` — 28 + 68 + 63 + 57    | **exact**, per crate: 28 / 68 / 63 / 57                                                                                                                                                             |
| nothing skips on a dead daemon                                     | **held** — identical result with `--run-ignored all`; zero `#[ignore]` in all four crates; conformance cases take 1.1–2.8 s each, which is real containers                                          |
| `cargo test --doc` — 13 passed, 0 ignored (11 + 1 + 1)             | **exact**                                                                                                                                                                                           |
| clippy `--all-targets -- -D warnings` on the four crates           | exit `0`                                                                                                                                                                                            |
| `cargo +nightly fmt … -- --check`                                  | exit `0`                                                                                                                                                                                            |
| `cargo check -p vpay-api --all-targets`                            | exit `0`                                                                                                                                                                                            |
| `cargo xtask verify-status`                                        | ok — 1 unimplemented item                                                                                                                                                                           |
| `cargo xtask verify-links`                                         | ok — 1625 links in 359 files                                                                                                                                                                        |
| "RFC-0003 itself was not edited"                                   | **true** — the RFC is byte-identical to the arm's base `8494e89e`                                                                                                                                   |
| `canonical_msisdn` is `pub(crate)` and unreachable from an adapter | **true** — `account_holders.rs:329`; neither adapter names `vpay-api`                                                                                                                               |
| `payer_instrument` is the rule the adapters copied                 | **true** — `payment_intents.rs:1146`, `get(code) → as_object → get("msisdn") → as_str → filter(!trim().is_empty())`, stored untrimmed                                                               |
| `Measured` is the only shipping decorator                          | **true** — the only other `impl ProviderAdapter for` outside the two adapters are `#[cfg(test)]` stubs; `boot::adapters_by_code` wraps every adapter, and the integration suite goes through it too |

The arm's reporting accuracy was exact on every number. What follows is what
its own tests could not have told it.

## Mutations run by the review

| Mutation                                                                            | Caught?                                                                                                                                                       |
| ----------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `impl Display for RefundTarget`                                                     | **yes** — build failure of `vpay-provider`'s test target, naming the `const` block                                                                            |
| `#[derive(Serialize)]` on `RefundTarget`                                            | **yes** — build failure                                                                                                                                       |
| `#[derive(Deserialize)]` on `RefundTarget`                                          | **yes** — build failure                                                                                                                                       |
| Delete the probe's three inherent impls, so every constant falls back to `false`    | **yes** — `"the probe must detect impls that do exist"`. The probe cannot pass vacuously for the traits it covers                                             |
| `impl Deref for RefundTarget { type Target = str; }`                                | **NO, before this review** — `destination.to_string()` printed `+237699887766` and the case passed. Caught now                                                |
| `impl AsRef<str> for RefundTarget`                                                  | **NO, before**; caught now                                                                                                                                    |
| `impl From<RefundTarget> for String`                                                | **NO, before**; caught now                                                                                                                                    |
| Delete `Measured::parse_destination`                                                | **yes** — `a_defaulted_method_is_not_silently_answered_by_the_wrapper`, and nothing else. It is the only guard, and it is enough                              |
| `Ok(RefundTarget::mobile_money(""))` for an empty map, both adapters (the brief's)  | **yes** — `a_destination_missing_this_rails_key_is_malformed` in each crate, **and** `a_required_rail_parses_its_own_destination` in conformance              |
| Drop `.map(str::trim)`                                                              | **yes** — `a_documented_destination_parses_to_the_payee`                                                                                                      |
| Fall back to the first string value in the map when the key is missing              | **yes** — two cases                                                                                                                                           |
| Append `; got {raw:?}` to both adapters' refusal, so the message echoes its input   | **yes in the adapter suites** (`a_refused_destination_never_echoes_the_number`); **NO in conformance**, whose assertion ran against the empty map. Caught now |
| An adapter that descends through its own `code()`, i.e. accepts the un-stripped map | **NO, before** — nothing asserted the boundary at all. Caught now                                                                                             |

## Findings

**Medium — `Malformed`'s classification is written for a rail, and this is the
one call site with no rail in it.** `ProviderError::Malformed` is the right
_variant_ (the port's error table reasons it, and RFC-0003 open question 5 left
inventing one to the first adapter that makes a real transfer). But
`Classify` derives `Category::Rail` from it, and a forwarded refusal is
therefore HTTP **502**, code `provider_error`, `Retry::AfterBackoff`,
`Severity::Warn`, and the envelope message _"The payment rail is temporarily
unavailable. The charge will be retried."_ — for a merchant's typo in their own
parameters. `public_message` is the category's canned sentence for this
variant, so the parameter name both adapters carefully put in `context`, and
which `a_refused_destination_never_echoes_the_number` asserts is there "for an
integrator", reaches the operator's log and **never the integrator**. Nothing
is wrong today because nothing calls `parse_destination`; the trap is one a
wave-3 author will paste straight through. Recorded in the port's doc comment
and pinned by `a_malformed_destination_is_classified_as_a_rail_fault`. The fix
is wave 3's: map it to `ApiError::invalid_param("destination", …)`, as the
confirm path already answers for `payment_method_data`.

**Medium — the `raw` boundary was prose on both sides.** The arm named it "the
load-bearing boundary line and a wave-3 handler must match it", and nothing
connected the two sides — not a type, not a test, not a caller. Nothing can,
until wave 3 writes one: the signature takes a `serde_json::Map` either way, so
both halves compile whichever map is handed over. What is now pinned is the
direction the mistake fails in
(`the_outer_destination_map_is_refused_rather_than_misread`, both adapters): an
un-stripped outer map is `Malformed`, never an `Ok` carrying a payee nobody
nominated. A newtype — `parse_destination(raw: RailScoped<'_>)` — would make it
unmissable, and was **not** done here: the maintainer wrote that signature
verbatim into RFC-0003's DECIDED text, and changing it is theirs, not a
reviewer's.

**Low — the probe was narrower than the claim it licenses.** See the mutation
table. Three constants added; `String` is the positive control for each, so the
self-check still fails if any of them stops detecting. Still not a closed set —
a `From<RefundTarget>` for some other printable type would slip past — and the
doc comment now says so rather than implying completeness.

**Low — the conformance no-echo assertion was unfalsifiable.** It asserted on
the refusal from an _empty_ map, so no implementation could have echoed a
number that was never handed in. Fixed to use a key no rail names, and
falsified against the `{raw:?}` mutation.

**Low, for Arm E — a doc comment that wave 2 will make false.**
`orange_money::parse_destination`'s heading is "Why this exists on a rail whose
`refund` does not", and its body states `supports_refunds` is `false` and
`refund` is the port's permanent `Unsupported`. `refunds/w2-orange` flips
exactly those two. The reasoning underneath stays correct — the destination is
a fact about the rail's product, the missing `refund` a fact about this
repository — but the paragraph needs rewriting when the flip lands, and no gate
checks prose. Not changed here: it is true today and Arm E's to update.

**Checked and clean.** Orange implementing `parse_destination` under
`supports_refunds: false` does **not** pre-empt Arm E: that arm changes
`supports_refunds`, `supports_partial_refunds` and `refund`, and touches
neither `refund_destination` (already `Required`) nor `parse_destination`. The
new conformance case branches on `refund_destination`, so the flip does not
disturb it either. And the reasoning holds: inheriting the default would have
Orange answer `Unsupported` to a question about the payee, i.e. claim it
returns money to the instrument that paid, on a redirect rail where `payer_ref`
is `None` and vpay never learns who paid. All three of `ProviderAdapter`'s
defaulted methods — `parse_destination`, `refund`, `account_holder_name` — are
forwarded by `Measured`; there is no sibling hole.

## Scope added mid-review: the fallible, canonicalising constructor

The maintainer decided on 2026-09-15, while this review was running, that
`RefundTarget::mobile_money` must canonicalise and must **refuse** a string that
is not a usable payee — closing the open question the arm had correctly left to
them. Implemented here because the file was already open.

### The shape chosen, and what it costs

The rule lives in `vpay-provider`, in a **private** `canonical_msisdn`, so the
fallible constructor is the only way to obtain a `RefundTarget` and no adapter
can hold a second spelling. `vpay_api::v1::account_holders::canonical_msisdn`
was **not** moved and not copied: it hardcodes `CM_COUNTRY_CODE`, and a
market-agnostic crate that learns one market is how the next rail's country
gets silently assumed. What crossed the boundary is the _specification_ — the
same eight-character separator set, the same 32-character input bound, and the
same digits-only `237600000200` output a rail's `partyId` takes — and not the
code.

**The cost, stated rather than buried: `vpay-provider` requires the leading
`+`.** It has no country to attach to a bare national form. Read as an
international number, `600000200` is nine digits under country code `6`, which
is Malaysia — so accepting it would not be leniency, it would be a different
subscriber in a different country receiving the refund. The consequence is a
real asymmetry between two vpay surfaces: `GET /v1/account_holders` accepts
`600000200` and a refund destination does not. It runs in the safe direction (a
lookup that guesses wrong returns the wrong name; a transfer that guesses wrong
sends the money) and it is recorded in [../backend.md](../backend.md) and in
[../../flows/provider-port.md](../../flows/provider-port.md) so that nobody
rediscovers it from a support ticket.

The error type is `InvalidMsisdn`, and **every variant is a unit variant**.
That is the design, not an omission: an error about a phone number is the
easiest place to leak one, and a variant carrying the input would put it one
`#[derive]` away from `ProviderError`'s `context` and every log line that
reaches. There is structurally nothing to carry it.
`an_invalid_msisdn_never_names_the_number_it_refused` is what fails if a
variant ever grows a field. The price is that an integrator is told which rule
they broke and not what they sent — the same trade `RefundTarget`'s redacting
`Debug` already makes.

Four variants: `NotInternational` (no `+`), `NotADigitString`, `WrongLength`
(E.164's 15-digit ceiling, and an 8-digit floor that catches a truncated
paste), `TrunkPrefix` (`+0…`, a national trunk prefix that survived a
merchant's `+`). No digit-count rule catches a single-digit typo; that is what
`account_holder_name` is for on the rails that have it, and it is why the floor
is documented as a floor and not as a validity oracle.

### Mutations on the new rule

| Mutation                                                                                               | Caught?                                                                                                                                                                      |
| ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **The maintainer's decisive one:** `canonical_msisdn` returns `Ok(input.trim())` for everything        | **yes** — 10 unit cases across all three crates, plus both conformance cases: 157 passed / 10 failed                                                                         |
| Drop **only** the `+` requirement, keeping every other rule — the exact leniency the decision is about | **yes** — 4 cases: `a_number_that_is_not_an_unambiguous_payee_is_refused`, `an_invalid_msisdn_never_names_the_number_it_refused`, and the new per-adapter case on both rails |

### What moved as a consequence

`msisdn()` now returns `237600000200`, not `+237600000200` — the canonical form
is the rail-facing one, matching the `partyId` both adapters already send on
the charge path and matching `canonical_msisdn`'s own output, so the workspace
holds one spelling. Five existing assertions were updated for it, and the
conformance suite now carries two constants (`DOCUMENTATION_MSISDN`, as a
merchant sends it, and `DOCUMENTATION_MSISDN_CANONICAL`, as the rail is given
it) with every negative assertion checking **both** — a redaction that hid one
spelling and printed the other would have leaked the number just the same.

New coverage: `every_spelling_of_one_payee_canonicalises_to_one_string` (seven
spellings including the two Unicode spaces a French locale inserts),
`a_number_that_is_not_an_unambiguous_payee_is_refused` (thirteen inputs, each
against its expected variant), `an_invalid_msisdn_never_names_the_number_it_refused`,
and `a_number_that_is_not_a_usable_payee_is_malformed_and_never_echoed` in each
adapter — which is the arm's `a_refused_destination_never_echoes_the_number`
extended to the new failure mode, as asked: six well-formed-but-invalid inputs,
each asserted `Malformed`, each asserted absent from `Display` and `Debug`, and
each asserted still to name `destination[<rail_code>][msisdn]`.

## Gates run on the review's own head

| Gate                                                                                                                 | Result                                                                                                          |
| -------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `cargo nextest run -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money -p vpay-tests-conformance` | **224 run, 224 passed, 0 skipped, 0 ignored**                                                                   |
| `cargo test --doc -p vpay-provider -p vpay-adapter-mtn-momo -p vpay-adapter-orange-money`                            | 14 passed, 0 failed, 0 ignored                                                                                  |
| `cargo clippy … --all-targets -- -D warnings`                                                                        | exit `0`                                                                                                        |
| `cargo +nightly fmt … -- --check`                                                                                    | exit `0`                                                                                                        |
| `cargo check -p vpay-api --all-targets`, and `cargo check --workspace --all-targets`                                 | exit `0` — the constructor became fallible, so the whole workspace was checked for callers, not just `vpay-api` |
| `cargo xtask verify-status`                                                                                          | ok — 1 unimplemented item, unchanged                                                                            |
| `cargo xtask verify-links`                                                                                           | ok                                                                                                              |

224 = 216 + 8: one classification case and one boundary case per the review's
own findings (3), and five for the canonicalisation the maintainer added
mid-review. The 14th doctest is `mobile_money`'s. Not run, and therefore not
claimed: `just ci`, the rest of `just verify`, the integration suite against a
real Postgres, the web gates.

## What this review did not do

- It did not change the port's signature, the `Malformed` variant choice, or
  any capability. Each is a maintainer decision already taken in RFC-0003.
- It did not move `canonical_msisdn` out of `vpay-api`, and the **confirm**
  path is unchanged: `payer_instrument` still accepts any non-whitespace
  string for a _payer_. Only the payee is held to the new rule. Whether the
  confirm path should move too is a decision about the charge path and was not
  taken here.
- It did not add a `RailScoped` newtype for `parse_destination`'s `raw`, for
  the reason given above.
- **No refund is any closer to working.** Nothing calls `parse_destination`
  outside tests; there is still no `POST /v1/refunds`, no MTN Disbursements
  call, no Orange transfer, and no ledger posting.
