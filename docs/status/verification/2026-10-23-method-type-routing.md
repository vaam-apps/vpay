# 2026-10-23 — payment-method types and a scored, explained router

Branch `wip/method-routing`, from `0fb04ec5` (the head of vaam-apps/vpay#341,
which merged as `c672fffc`), **rebased onto the two-step submit**
(vaam-apps/vpay#345, merged as `3633ca0c`): see § "Rebased onto the two-step
submit" for what changed and the gates re-run there. It builds slice 1 of
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
  exploration. _(Answered later the same day; see the addendum below.)_
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

**Addendum, 2026-10-23, after this page was written.** The maintainer
answered R8 with "go with your defaults" and, on the score, "first go with
your defaults. Just document everything". The answers:

- **R8.1:** the estimated fee next, then the completion rate. The completion
  rate is reported and set by hand as a configured weight, never an automatic
  score component ("Go with (a)", asked after the first answers).
- **R8.2:** the operator sets weights until RFC-0009.
- **R8.3:** health reports and never moves traffic.
- **R8.4:** the confirm follows the rail the page showed while it is
  eligible.
- **R8.5:** no exploration.

The tie-break above is kept. Nothing on this page changes: none of the
answers is built by it, and R8.4's is a gap in it. RFC-0008 § R8 records
each answer with its consequence.

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

## Rebased onto the two-step submit

The change was built on `0fb04ec5`. ADR-0032's two-step submit
(vaam-apps/vpay#345) landed first, so the eight commits were rebased onto its
head `c7aaad3d`, whose tree `3633ca0c` on `master` is. Both changes touch the
port, the three adapters, the confirm path, the worker and the tests; every
conflict was resolved so both survive:

- **Every `Capabilities` and `ChargeRef` literal carries both new fields**:
  `submit_steps` beside `method_types`, and `method_type` on each
  `ChargeRef`, including the two #345 added. The confirm path's
  `prepare_on_rail` now builds its `ChargeRef` with the routed
  `target.method_type`, as `submit_to_rail` does; the worker builds both its
  `prepare` and its `submit` `ChargeRef` in one function, which reads
  `charges.method_type`.
- **Notch Pay's channel is chosen at `submit`**, the call that sends the
  process body. `prepare` (initialize) sends no channel, and both still refuse
  a bad `settings.channel` before any request.
- **Migrations**: #345's `0052_provider-requests-prepare.sql`, then this
  change's `0053_method-types-and-routing-decisions.sql`, both in
  `MANIFEST.sha256` in order. The counts this change had left at 52 are 53:
  `backends/migrations/README.md`, `postgres_smoke.rs`'s applied count, and
  `docs/status/infrastructure.md`, which names both. The drift figures stay
  this change's (209 changes, 28 relations, 20 unmappable): `0052` moves
  none of them, and the drift test passes with them.
  Between this change's API commit and its renumber commit two files carry
  `0052`, so those intermediate commits compile but would not migrate; the
  branch head does both.
- **Docs**: `adapter-notchpay.md`, `provider-port.md`, `docs/status/backend.md`
  and `docs/status/README.md` carry both changes' text. The sample
  configurations, `configuration.md`, RFC-0012's process example, the
  sandbox runbook and three code comments no longer call `cm.mobile` "the only
  channel": it is the only value `settings.channel` takes, and a routed
  charge is sent `cm.mtn` or `cm.orange`.

### Gates after the rebase

Run on `land/method-routing` after the rebase, with Docker for Postgres,
WireMock and Keycloak, and **`cargo nextest`**, which CI runs.

| Gate                                                      | Result                                                                                                                                                       |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `cargo fmt --all -- --check`                              | clean                                                                                                                                                        |
| `cargo clippy --workspace --all-targets -- -D warnings`   | clean                                                                                                                                                        |
| `cargo nextest run`, every test binary of the workspace   | **54 binaries, 2 429 tests: 2 429 passed, 0 failed, 0 skipped**, counting the one rerun below. `confirm_rails` 20, `worker_recovery` 36, `postgres_smoke` 54 |
| `cargo test --doc --workspace`                            | **136 passed, 0 failed, 1 ignored**                                                                                                                          |
| `just verify`                                             | ok, fifteen gates; `verify-migrations` 53 files, `verify-doc-counts` 13 counts, `verify-sdk-parity` 792 tests and 46 dated gaps                              |
| `just verify-ignored`'s three figures, from the runs      | 0 ignored, 54 binaries (expected 54), 2 429 tests (minimum 1080)                                                                                             |
| `sdks/nodejs`: `vitest run`, `tsc --noEmit`, `eslint`     | **237 passed**; clean; clean                                                                                                                                 |
| `sdks/stripe-compat`: `tsc --noEmit`, `eslint`            | clean; clean                                                                                                                                                 |
| vpay-skills `tools/verify-coverage.mjs` against this tree | ok: 21 skills, 265 vpay paths, 48 feature pages                                                                                                              |

**How it was run, and the one rerun.** This host's disk could not hold all
54 debug test binaries at once (about 400 MB each), so the integration
package ran one binary at a time with each executable deleted after its run,
and the rest of the workspace in one `cargo nextest run`. `just verify-ignored`
was not run as a recipe, because its `cargo nextest list` builds all 54 again;
its three figures are read from those runs instead, and no `#[ignore]`
attribute exists in the tree. `postgres_smoke`'s
`the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` failed
once, with the measured 209 in its message, because this shell exported
`RUST_BACKTRACE=1` and the `cratestack` CLI it runs appended a stack backtrace
to the stderr the case compares exactly. CI's workflow does not set it. Rerun
with the variable unset, the binary passed **54 of 54**.

## Gates on the final tree, before the rebase

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
  `/v1` object (D10 as written). _(The type and the rail are since
  2026-10-24, as the intent's `payment_method_details`; the decision record
  itself is still on none.
  [2026-10-24-rail-on-v1.md](2026-10-24-rail-on-v1.md).)_
- **No Stripe-compat case** sends a method type.

Found by an independent read-only review of the final tree, on 2026-10-23.
None of these moves money wrongly, and none is fixed here:

- **The request body a routed confirm needs depends on which rail wins.**
  `orange_wallet` is served by `notchpay`, a push rail that needs `msisdn`,
  and by `orange_money`, a redirect rail that declares no payer fields and
  needs `return_url`. A server-side confirm that sends
  `payment_method_data[orange_wallet][msisdn]` works while `notchpay` ranks
  first. It becomes a `400` if an operator re-weights `orange_money` above it
  or disables `notchpay`. The workaround is the pin. Whether the routing
  filter should take the supplied payer fields into account is R8.4's
  question.
- **A payer's number is not checked against the network they chose.** A
  routed Notch Pay charge is sent `cm.mtn` or `cm.orange` as the payer chose,
  even if they then type the other network's number. `cm.mobile` resolved the
  network from the number; the chosen channel does not. vpay carries no
  number-plan table, by design. What Notch Pay answers for a mismatched
  number has not been observed. _(Observed the same evening through the
  probe, not vpay: a `422` at process whose `errors` name `phone`
  ([2026-10-23-notchpay-sandbox-two-step-and-channels.md](2026-10-23-notchpay-sandbox-two-step-and-channels.md)),
  which the adapter maps to `invalid_payer` since
  [2026-10-23-notchpay-defaults-and-wrong-network.md](2026-10-23-notchpay-defaults-and-wrong-network.md).
  Noted 2026-10-24.)_
- **Rolling back below this change.** An older image does not know
  `mtn_wallet` or `orange_wallet`. An intent created with one returns a `500`
  on its checkout page (`build_rails` finds no adapter for it) and a `400`
  on a confirm. The same holds during a multi-replica rollout, while old
  pods still serve. [deploy-and-rollback.md](../../runbooks/deploy-and-rollback.md)
  § 4 already says an older image against migration `0053` is unsupported;
  this is the data-level face of that.
- **A scalar `payment_method_options` is now refused.** Before this change a
  confirm whose `payment_method_options` was a plain string was accepted and
  the field ignored. It must now be a map. No Stripe SDK sends a scalar.

Found on 2026-10-24 by vpay-docs' re-read for v0.12.0, and not fixed either;
none moves money:

- **A routed charge cannot be refunded by its method type, and `/v1` does
  not say which rail it went to.** `POST /v1/refunds` keys `destination` on
  the charge's **rail** code (`refunds.rs`, `map.get(code)`), as it always
  has. Before this change a rail code was the only thing an intent could
  name, so the API reference, three of the route's own error messages and
  both SDKs' doc comments called the key `<payment_method_type>`. They now
  say `<rail code>` and that a type is refused. A merchant refunding a routed
  `mtn_wallet` charge on `mtn_momo` must send `destination[mtn_momo][…]`;
  `destination[mtn_wallet][…]` is a `400` naming `destination`, and the rail
  that won is on no `/v1` object (the "merchant-readable decision" above).
  The SDKs' `RefundDestination` takes a `PaymentMethodType`, which since this
  change also holds `MtnWallet` and `OrangeWallet`, so an SDK caller can build
  the refused key. Two ways out, neither taken: the refund route also
  accepting `destination[<the charge's method type>]`, or `/v1` naming the
  rail. Both are API decisions for the maintainer. Today only `mtn_momo` and
  `orange_money` need a destination, so the gap is a routed charge that
  landed on one of those two. _(Half closed 2026-10-24: the maintainer chose
  "Both". The refund route now also accepts the type the charge records
  ([2026-10-24-refund-by-method-type.md](2026-10-24-refund-by-method-type.md)),
  so neither the merchant nor an SDK caller builds a refused key any more.
  The rail on `/v1` is the other half, decided and not built. Built later
  that day: [2026-10-24-rail-on-v1.md](2026-10-24-rail-on-v1.md).)_
