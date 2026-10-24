# 2026-10-24 — a refund's `destination` may name the charge's payment-method type

Branch `claude/happy-hopper-r3h6rz-refund-by-method-type`, from `master` at
`630c8a34` (v0.12.1). `POST /v1/refunds` now accepts the payment-method type a
charge records as the `destination` key, beside the rail's code. Proven by
unit tests run here and an integration case written for CI. **No rail was
called, and the integration case could not run in this environment**
(§ "What was not run").

## Whose decision

- **The maintainer's**, on 2026-10-24. Asked how to close the gap
  [2026-10-23-method-type-routing.md](2026-10-23-method-type-routing.md)
  recorded (a routed charge is refunded under its rail's code, and no `/v1`
  object names the rail), they chose **"Both"**: "Accept the method type now
  and show the rail on /v1 later." This page is the first half. The second,
  the rail on `/v1`, is decided and not built. _(Built later the same day, in
  the shape the maintainer chose ("Go with A"): the intent's
  `payment_method_details`,
  [2026-10-24-rail-on-v1.md](2026-10-24-rail-on-v1.md).)_
- **The implementer's, each reversible and named here:**
  - the type accepted is the one the charge **records** (`charges.method_type`,
    migration `0053`), not "the type the charge was routed by". `0053` sets
    it on every routed charge and also on a rail-code charge whose rail serves
    one type, so `destination[mtn_wallet]` also refunds an `mtn_momo` charge
    confirmed by rail code since `0053`. A charge that records no type (a
    rail-code charge on a rail serving several, or one written before `0053`)
    takes its rail's code only;
  - **both keys at once is a `400`** naming `destination`, rather than one
    winning: two keys can carry two numbers, and choosing one is a guess about
    where a payer's money goes;
  - another type, or a type on a charge that records none, is refused with the
    same `400` as a key naming another rail. The extra keys a map may carry
    beside the one read are still ignored, as before;
  - the adapter is handed the same rail-scoped inner map under either key, so
    `parse_destination`, the account-holder check and the transfer are
    unchanged;
  - the three error messages name both spellings and stay static: none says
    which rail the charge went to. Showing the rail is the second half's job,
    on a field, not in an error string;
  - a stored type equal to the rail's own code is ignored rather than read as a
    second key. The vocabularies are disjoint (migration `0053`'s CHECK), so
    this only guards a corrupt row.

## What changed

- `backends/crates/vpay-api/src/v1/refunds.rs`: `resolve_destination` takes
  the charge's `method_type`; `resolve_target` passes
  `charge.method_type.as_deref()`, which `ChargeRow` already carried (the
  refund path already handed it to the adapter as `ChargeRef::method_type`).
  The doc comment gains § "The charge's payment-method type also names it".
- Seven unit tests in the same file, and one integration case in
  `backends/tests/integration/tests/refunds.rs`.
- Docs: `docs/api/README.md` (the `/v1/refunds` row and § payment-method
  types), `docs/flows/stripe-sdk-compat.md`,
  `docs/flows/merchant-auth/resource-contract.md`, `docs/status/backend.md`
  (the routing section's "Not done"),
  [2026-10-23-method-type-routing.md](2026-10-23-method-type-routing.md) (the
  gap bullet, marked half closed) and [../README.md](../README.md) (this page,
  116/93 once master's a11y-gate-built-stylesheet and
  notchpay-dashboard-webhooks pages had merged in).
- SDK doc comments only, no signature: `sdks/rust/src/resources.rs`
  (`RefundDestination::MobileMoney`) and `sdks/nodejs/src/types.ts`
  (`RefundDestination`). Both said a type there is refused, which vaam-apps/vpay#359
  wrote on 2026-10-24 and this change makes untrue against a server carrying
  it. They now say a type is accepted for a charge that records it, and
  refused by an older server.

## Tests

`cargo test -p vpay-api --lib v1::refunds`: **30 passed, 0 failed, 0
ignored** (23 before this change, 7 added):

| Test                                                                 | What it pins                                         |
| -------------------------------------------------------------------- | ---------------------------------------------------- |
| `a_charge_accepts_its_method_type_as_the_destination_key`            | the type key parses to the same payee                |
| `a_charge_with_a_method_type_still_accepts_its_rail_code`            | the rail key still works on a typed charge           |
| `naming_both_the_rail_and_the_method_type_is_refused`                | two keys, `400` naming `destination`                 |
| `a_method_type_key_on_a_charge_with_no_recorded_type_is_refused`     | a type on an untyped charge, `400`                   |
| `another_method_type_is_refused`                                     | `orange_wallet` on an `mtn_wallet` charge            |
| `an_origin_rail_refuses_a_destination_keyed_by_method_type`          | `Origin` refuses any key, unchanged                  |
| `a_malformed_payee_under_the_method_type_is_still_the_callers_error` | a bad payee under the type key is a `400`, not `502` |

**Mutations**, each applied alone to `resolve_destination`, the file restored
and compared byte for byte after each:

| Mutation                                             | Result                                                                                                                  |
| ---------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| both keys accepted, the rail's winning               | 29 passed, 1 failed: `naming_both_the_rail_and_the_method_type_is_refused`                                              |
| the type key never read                              | 28 passed, 2 failed: `a_charge_accepts_its_method_type_as_the_destination_key`, `naming_both_…`                         |
| any known type accepted, whatever the charge records | 28 passed, 2 failed: `a_method_type_key_on_a_charge_with_no_recorded_type_is_refused`, `another_method_type_is_refused` |

Not caught by a unit test: `resolve_target` passing `None` instead of the
charge's type. Only the integration case below covers that line.

**Integration**: `a_charge_is_refunded_under_the_method_type_it_records`, in
`backends/tests/integration/tests/refunds.rs`, drives `POST /v1/refunds`
through the real router against a real Postgres and the MTN WireMock rail.
Three intents, one per branch: a charge recording `mtn_wallet` refunded under
`destination[mtn_wallet]` (`201`, `pending`, one transfer in MTN's journal
under the refund's reference), an untyped charge refused under the same key
(`400` naming `destination`), and both keys at once (`400`). One refund row
is written in all. The type is written onto the seeded charge directly, for
the suite's stated reason.

## Gates

- `cargo fmt --all -- --check`: clean.
- `cargo clippy -p vpay-api --all-targets -- -D warnings`: clean.
- `cargo clippy -p vpay-tests-integration --test refunds -- -D warnings`:
  clean, so the new case compiles.
- `just verify`: exit `0`, "verify: ok — the fifteen gates above passed";
  `verify-doc-counts` agrees with this page's 114/91 in `../README.md`.
  Re-run on the tree with master merged in, where the count is 115/92 after
  vaam-apps/vpay#349 and 116/93 after vaam-apps/vpay#364: exit `0` both times.
- `prettier@3.9.6 --check` on the changed Markdown and TypeScript: clean.

## What was not run

- **The integration case.** This session's Docker CLI has no daemon:
  `failed to create a container: Error in the hyper legacy client: client
error (Connect)` at `postgres:16-alpine`, before any test body. The existing
  `a_missing_or_malformed_destination_is_the_callers_400` fails identically
  here, so it is the environment, not the case. CI's `rust` job is its first
  run.
- **Any rail.** No refund reached MTN, Orange or Notch Pay. `mtn_momo::refund`
  has still never been called against MTN; `orange_money::refund` and
  `notchpay::refund` are still `NotImplemented`.
- **The rail on `/v1`**, the maintainer's second half. Not built. _(Built
  later on 2026-10-24: [2026-10-24-rail-on-v1.md](2026-10-24-rail-on-v1.md).)_
