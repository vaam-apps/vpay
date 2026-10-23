# 2026-10-23 — payment-method types and a scored, explained router

Branch `wip/method-routing`, from `0fb04ec5` (the head of vaam-apps/vpay#341,
which merged as `c672fffc`). It builds slice 1 of
[the plan](../../plans/2026-10-23-method-type-routing.md): everything in
[RFC-0008](../../rfc/0008-payment-method-routing.md)'s 2026-10-23 revision that
does not depend on the score's final formula. **It is proven against WireMock
stubs and a real Postgres. Nothing was sent to any payment provider.**

## Whose decision

- **The maintainer's:** that the payer chooses a network and vpay shows "the
  correct rail" (#8, #10); that routing is "a score of each payment solution
  for each checkout"; the spellings `mtn_wallet` and `orange_wallet`.
  ADR-0029's Status carries the quotes, D12/D13 confirmed and D8/D9 reopened.
- **Still the maintainer's, asked in RFC-0008's revision R8:** which score
  components v1 has, who sets the weights, whether health may move traffic
  automatically, what happens when render and confirm disagree, and
  exploration.
- **The implementer's, each reversible and named here:**
  - the v1 formula `provisional-1`: one component, a configured weight per
    rail or per rail and type (`routing.weights`, 0–1000, default 100), ties
    to the lower rail code. With equal weights that sends `mtn_wallet` to
    `mtn_momo` and `orange_wallet` to `notchpay`; that is a tie-break, not a
    preference;
  - `Capabilities::method_types` as a field rather than a new trait method;
  - the pin on the **confirm** only (`payment_method_options[<type>][rail]`),
    never stored on the intent;
  - a create with a method type is refused when no enabled rail serves it in
    the intent's currency, while a rail code keeps being checked only at the
    confirm (D6: a rail code's behaviour is unchanged);
  - `charges.method_type` is also set on a rail-code charge whose rail serves
    exactly one type, and left `NULL` on a `notchpay` rail-code charge;
  - no `model` for `routing_decisions` or the new column in
    `schemas/vpay.cstack` (drift +3, measured below);
  - exhaustive enumeration over small inputs in place of a property-testing
    dependency.

## What was built

| Piece                                                                                                                                                                   | Where                                                             |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| `MethodType`, `MethodTypes`, `Weights`, `route` (hard filter, scored and explained ranking, `Decision` with formula and a 16-hex config version); names no rail         | `vpay-core/src/routing.rs`                                        |
| `Capabilities::method_types`, `method_types_are_coherent`; `ChargeRef::method_type`                                                                                     | `vpay-provider/src/lib.rs`                                        |
| `mtn_momo` serves `mtn_wallet`, `orange_money` `orange_wallet`, `notchpay` both; Notch Pay's channel per charge (`method_type_channel`, `channel_for`; RFC-0012 step 8) | the three adapter crates                                          |
| `routing.weights` and four file refusals; two boot refusals                                                                                                             | `vpay-config/src/config.rs`, `vpay-api/src/v1/boot.rs`            |
| Migration `0053`: `charges.method_type`, `routing_decisions`                                                                                                            | `backends/migrations/`                                            |
| `NewRoutingDecision`, `TxRepositories::insert_routing_decision`, `Charges::routing_decision_for`; `charges.method_type` read and written                                | `vpay-db/src/routing_decisions.rs`, `charges.rs`, `repository.rs` |
| Create accepts types; a confirm by type is routed; the pin; the decision row committed with the charge                                                                  | `vpay-api/src/v1/payment_intents.rs`, `v1/mod.rs`                 |
| The checkout session's browser answer: one entry per type, flowing as the first-ranked rail                                                                             | `vpay-api/src/browser/checkout_sessions.rs`                       |
| The worker and the refund path carry `method_type` to the adapter                                                                                                       | `vpay-worker/src/handlers.rs`, `vpay-api/src/v1/refunds.rs`       |
| Both SDKs spell `mtn_wallet`, `orange_wallet` on a create                                                                                                               | `sdks/rust/src/model.rs`, `sdks/nodejs/src/types.ts`              |
| The privacy inventory classifies the seven new columns                                                                                                                  | `schemas/privacy-inventory.yaml`                                  |

No route was mounted or unmounted. No `NotImplemented` token moved. No gate was
added. No test binary was added, so `verify-ignored`'s `expected_suites` did not
move.

## How it is proven

**Real Postgres and WireMock, `backends/tests/integration/tests/confirm_rails.rs`:**

- `a_method_type_confirm_is_routed_and_commits_its_decision_with_the_charge`:
  an intent created through the Rust SDK with `PaymentMethodType::MtnWallet`,
  confirmed by type. `200 processing`; the charge is `mtn_momo` with
  `method_type = mtn_wallet`; the `routing_decisions` row has `provisional-1`,
  a 16-hex `config_version`, no pin, and `considered` exactly
  `[mtn_momo scored 100 (configured_weight 100), orange_money excluded
does_not_serve]`.
- `the_highest_score_wins_a_pin_overrides_it_and_notch_pay_is_told_the_network`:
  a second server on the same database with `notchpay` added on its stub and
  `routing.weights` giving it 300 for `mtn_wallet`.
  1. Unpinned `mtn_wallet` goes to `notchpay` (300 over `mtn_momo`'s 100), and
     the stub's journal holds exactly one `process` call on `cm.mtn`.
  2. Pinned to `mtn_momo`, it goes there. The row records the pin, and
     `notchpay` as `not_pinned`.
  3. Pinned to `orange_money`: `400` naming
     `payment_method_options[mtn_wallet][rail]`, and no charge.
  4. `orange_wallet` at equal weights goes to `notchpay` by the tie-break, as a
     push, and the journal holds one `process` call on `cm.orange`.
- `a_type_this_deployment_cannot_take_is_a_400_naming_the_parameter`: `card`
  and `bank_transfer` refused as reserved, an unknown type refused, and
  `mtn_wallet` in EUR refused, all naming `payment_method_types`. A confirm by
  a type the intent was not created with names `payment_method_data[type]`; an
  option that is not a pin names `payment_method_options[mtn_wallet]`; no
  charge is written.
- `a_push_confirm_the_rail_accepts_moves_the_intent_to_processing` (extended):
  a rail-code confirm behaves as before, records `method_type = mtn_wallet`
  because `mtn_momo` serves one type, and has **no** decision row.

**Unit and conformance:**

- `vpay-core`: nine routing tests, covering eligibility, ranking, the
  tie-break, the pin, the config version and serde. They are exhaustive over
  small inputs.
- `vpay-config`: two routing tests, one accepting and one refusing every
  mistake the file can show.
- `vpay-api` boot: two refusal tests. Payment intents: two tests on method
  types and the pin. Checkout: one test for the collapse entry.
- The three adapters: each declares exactly its networks. Notch Pay's channel
  test covers every type and `None`.
- Conformance: `every_adapter_declares_the_method_types_it_serves` pins the
  table.
- `vpay-server`: `the_router_names_no_rail` reads `routing.rs`'s code lines
  for any linked rail code; `every_linked_adapter_serves_a_method_type`.
- SDKs: `create_sends_payment_method_types_beside_rail_codes_as_written`
  (Rust) and `payment_intents.create sends a payment-method type beside a rail
code` (Node), byte for byte the same body.

**Mutations.** Each guard was broken and the named tests were run; each run
failed, and the code was restored:

| Mutation                                      | Failed                                                            |
| --------------------------------------------- | ----------------------------------------------------------------- |
| router: the `enabled` filter removed          | `vpay-core` routing tests                                         |
| router: the tie-break reversed                | `vpay-core` routing tests                                         |
| router: the lowest score wins                 | `vpay-core` routing tests                                         |
| router: the pin ignored                       | `vpay-core` routing tests                                         |
| boot: `RailServesNoMethodType` removed        | `an_enabled_rail_serving_no_method_type_is_refused_…`             |
| boot: `RoutingWeightOnAnUnservedType` removed | `a_weight_on_a_type_the_rail_does_not_serve_is_refused_at_boot`   |
| config: `UnknownRoutingRail` removed          | `routing_weights_are_refused_for_every_mistake_the_file_can_show` |
| config: `UnknownMethodType` removed           | the same                                                          |
| config: `DuplicateRoutingWeight` removed      | the same                                                          |
| config: `RoutingWeightTooLarge` removed       | the same                                                          |
| API: `insert_routing_decision` not called     | the two routed `confirm_rails.rs` cases                           |
| `routing.rs`: a line naming `notchpay` added  | `the_router_names_no_rail`                                        |

**Schema drift, measured on the pinned `cratestack` 0.15.0:** `drift detected
in 28 table(s)/view(s) (209 change(s) total)`, from 206 over 27. Three lines,
all expected: `charges.method_type` and its CHECK undeclared, and
`routing_decisions` undeclared. Unmappable columns go from 19 to 20
(`routing_decisions.considered`, `jsonb`). Recorded in
[status/cratestack.md](../cratestack.md).

## Gates on the final tree

Run on 2026-10-23 on this branch's tree, with Docker for Postgres and WireMock.

| Gate                                                                   | Result                                                                                                                          |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `cargo fmt --all -- --check`                                           | clean                                                                                                                           |
| `cargo clippy --workspace --all-targets -- -D warnings`                | clean                                                                                                                           |
| `cargo test --workspace --all-targets --no-fail-fast`                  | 56 test binaries: **2 409 passed, 3 failed, 0 ignored**. The three are explained below the table and pass under nextest's rule  |
| `cargo test --doc --workspace` (doctests, a separate runner)           | **136 passed, 0 failed, 1 ignored**                                                                                             |
| `postgres_smoke` and `confirm_rails`, rerun after the renumber to 0053 | **54** and **18 passed**, 0 failed, 0 ignored                                                                                   |
| `just verify`                                                          | ok, fifteen gates; `verify-migrations` 52 files, `verify-sdk-parity` 792 tests and 46 dated gaps, `verify-doc-counts` 13 counts |
| `sdks/nodejs`: `vitest run`, `tsc --noEmit`, `eslint`                  | **237 passed** in 9 files; clean; clean                                                                                         |
| `sdks/stripe-compat`: `tsc --noEmit`, `eslint`                         | clean; clean                                                                                                                    |
| vpay-skills `tools/verify-coverage.mjs` against this tree              | ok: 21 skills, 265 vpay paths, 48 feature pages                                                                                 |

**The three failures are the runner's.** All three assert exact values of
the process-global metrics recorder:
`webhooks.rs::the_ladder_walks_delivery_delay_and_then_succeeds`,
`worker_e2e.rs::a_decline_after_submission_returns_the_intent_to_requires_payment_method`
and `worker_e2e.rs::a_confirmed_payment_is_driven_to_succeeded_and_the_merchant_sees_it`.
`cargo test` shares one process across a binary, and `worker_e2e.rs`'s own
`METRICS` doc says that is where those counts break. `cargo nextest`, which
`just test-rust` and CI run, gives every test its own process. It is not
installed on this host. So each of the three was rerun alone with
`cargo test -- --exact`, the same one-process-per-test condition, and each
passed. None of the three touches routing.

**Not run here:**

- `just verify-ignored`, which needs `cargo nextest list`. No `#[ignore]` and
  no test binary were added, so `expected_suites` stays at 54.
- The Stripe-compat `compat` suite, which needs a live compose stack.
- `pnpm -r test` beyond the Node SDK.
- `just test-tauri-rust` and the frontends, which this change does not touch.

## Not done

- **The checkout page** rendering a method type (slice 2). The server's
  browser answer is in place; no frontend test covers it.
- **The SDKs' confirm by type and the pin**: a ⛔ row dated 2026-10-23 in
  [sdks/parity.md](../../sdks/parity.md). `orange_wallet`'s fields depend on
  the rail, so a server-side confirm has to pin.
- **Every score component but the configured weight** (slice 3), until the
  maintainer answers R8.
- **`cm.mtn` and `cm.orange` against Notch Pay's real sandbox.** Stub only.
- **A merchant-readable decision**: the method type and the rail are on no
  `/v1` object (D10 as written).
- **No Stripe-compat case** sends a method type.
