# 2026-10-23 — Notch Pay: registration off by default, and a wrong-network number is `invalid_payer`

Branch `claude/happy-hopper-r3h6rz-notchpay-decisions`, from `master` at
`9499f09d` (the merge of vaam-apps/vpay#350). Two decisions the maintainer took
on that pull request's sandbox evidence
([verification](2026-10-23-notchpay-sandbox-two-step-and-channels.md)), built
and proven against WireMock and a real Postgres. **Nothing was sent to Notch
Pay, or to any other provider.**

## Whose decision

- **The maintainer's**, both on 2026-10-23:
  - asked "vpay registers a Notch Pay webhook endpoint by default (RFC-0013,
    `register: ensure`) … Change Notch Pay's default?", they answered
    **"Default to `off` for Notch Pay"**. The evidence: the endpoint the probe
    created through `POST /webhooks` was stored with `"url": null` and
    delivered nothing in 21 minutes, and four payments processed after it
    stayed `processing`, where eleven before it settled in about 5 s
    (RFC-0012 § 10 items 24 and 25; the second is a correlation, not a shown
    cause);
  - asked "A wrong-network number (Orange number on cm.mtn) is refused with
    422 on `errors.phone`. Map it to `invalid_payer` instead?", they answered
    **"Yes, `invalid_payer`"**.
- **The implementer's, each reversible and named here:**
  - the default is **declared by the adapter**, as a new trait method
    `ProviderAdapter::default_callback_registration`, not decided in the
    binary: a `"notchpay"` match outside an adapter crate is what ADR-0002
    forbids. A method rather than a `Capabilities` field, on
    `accepted_exposure`'s precedent: nothing persists it and the core never
    branches on it;
  - `CallbackRegistrationDefault` has two values, `Ensure` and `Off`, and no
    `Check`: no rail wants reporting without writing by default;
  - `providers[].callbacks.register` became an `Option`, so `callbacks: {}`
    keeps meaning what an absent block means, and `CallbackRegistration`
    lost its `Default`, so nothing can take `ensure` silently;
  - the unset rail is resolved in the binary's plan (`callback_plan`), which
    logs one `info` line naming the rail when the result is `off`; the
    mapping onto the worker's `Registration` is a `From` in `vpay-worker`, so
    the integration suite can exercise it without the binary's private
    module;
  - `invalid_payer` only for a **process** `422` whose `errors` name `phone`
    **and nothing else**. A `422` naming another field, `phone` beside
    another field, and the same refusal on initialize (never measured) stay
    `provider_error`, because those may be vpay's own request at fault;
  - the message is vpay's own and quotes nothing of Notch Pay's, whose
    phone refusals are the likeliest to repeat the number.

## What was built

| Piece                                                                                                                                                                          | Where                                                       |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| `CallbackRegistrationDefault::{Ensure, Off}`; `ProviderAdapter::default_callback_registration`, `Ensure` by default                                                            | `vpay-provider/src/lib.rs`                                  |
| `Measured` forwards it                                                                                                                                                         | `vpay-provider/src/measured.rs`                             |
| `notchpay` overrides it with `Off`                                                                                                                                             | `vpay-adapter-notchpay/src/lib.rs`                          |
| `ProviderCallbacks::register: Option<CallbackRegistration>`; `CallbackRegistration` without `Default`                                                                          | `vpay-config/src/config.rs`                                 |
| `From<CallbackRegistrationDefault> for Registration`                                                                                                                           | `vpay-worker/src/callbacks.rs`                              |
| `callback_plan(config, adapters)`: the setting, else the adapter's default, else `ensure` for a rail with no adapter; the `info` line                                          | `vpay-server/src/callbacks.rs`, `worker.rs`                 |
| `process_refusal`: `422 if names_only_the_payers_phone(..)` → `Rejected { invalid_payer }`; `PRODUCED_FAILURE_CODES` six codes                                                 | `vpay-adapter-notchpay/src/mapping.rs`                      |
| The stub answers the sandbox's `422` "Invalid CM Mobile Money" (`errors.phone` alone) to a process on `cm.mtn` with an Orange `+23769…` number (priority 5, ahead of the rest) | `wiremock/notchpay/mappings/payments-process.json`          |
| `notchpay_a_number_on_the_other_network_is_an_invalid_payer`; the two-way gate adds `invalid_payer` for Notch Pay by hand, naming that case                                    | `backends/tests/conformance/tests/adapter_conformance.rs`   |
| `an_unset_register_leaves_a_notch_pay_account_alone`                                                                                                                           | `backends/tests/integration/tests/callback_registration.rs` |

No route was mounted or unmounted. No `NotImplemented` token moved. No gate was
added. No test binary was added, so `verify-ignored`'s `expected_suites` did
not move. No migration.

## How it is proven

**Real Postgres and WireMock**, `callback_registration.rs`,
`an_unset_register_leaves_a_notch_pay_account_alone`: the Notch Pay adapter
the suite links declares `Off`, which maps to `Registration::Off`; a pass with
that setting answers `RailCheck::Off`, leaves the registration state empty,
and the stub's journal holds **0** `GET` and **0** `POST` on `/webhooks`. The
same harness with an explicit `ensure` then answers `Created`, with **1**
`POST`.

**WireMock**, `adapter_conformance.rs`,
`notchpay_a_number_on_the_other_network_is_an_invalid_payer`: `REF_ACCEPTED`,
method type `mtn_wallet` (so the adapter sends `cm.mtn`) and the Orange number
`237690000000`. `submit` answers `Err(Rejected { invalid_payer })`. The stub's
mapping matches the channel and the number, so the case also proves the
routed charge was sent `cm.mtn`. `the_declines_prove_every_code_each_rail_can_produce`
still holds every rail's cases against its `PRODUCED_FAILURE_CODES`.

**Unit:**

- `vpay-provider`: `the_default_callback_registration_is_forwarded_not_defaulted`
  (the test double declares `Off`; `Measured` must answer `Off`).
- `vpay-adapter-notchpay`: `the_callback_registration_default_is_off`;
  `a_number_on_the_other_network_is_an_invalid_payer` (both measured messages,
  "Invalid CM Mobile Money" and "Invalid CM Orange Money", are
  `invalid_payer`; the message carries neither; the same body on initialize,
  and `phone` beside `amount` on process, stay `provider_error`);
  `the_declared_codes_are_exactly_what_the_mapping_can_produce` (now drives
  `process_refusal` with a phone-only `422`).
- `vpay-config`: `a_callbacks_block_leaves_register_unset_and_refuses_an_unknown_key`
  (`{}` is `None`; each spelling is `Some`).
- `vpay-worker`: `an_adapters_default_maps_onto_the_same_setting`.
- `vpay-server`: `the_plan_reads_each_rails_setting_else_its_adapters_default`
  (with the linked adapters: unset `notchpay` is `off`, `mtn_momo` `ensure`;
  `callbacks: {}` on `notchpay` is `off`; an explicit `ensure`, `check` or
  `off` is honoured; a disabled rail is left out).

**Mutations.** Each guard was broken, the named tests were run with
`--no-fail-fast`, and the file was restored from `HEAD`:

| Mutation                                                                               | Failed                                                                                                                                                                           |
| -------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| M1: `notchpay`'s `default_callback_registration` answers `Ensure`                      | `the_callback_registration_default_is_off`; `the_plan_reads_each_rails_setting_else_its_adapters_default`; `an_unset_register_leaves_a_notch_pay_account_alone`                  |
| M2: `Measured` answers `Ensure` instead of forwarding                                  | `the_default_callback_registration_is_forwarded_not_defaulted`; `an_unset_register_leaves_a_notch_pay_account_alone` (the suite's adapters are `Measured`-wrapped)               |
| M3: the plan takes `Ensure` for every unset rail, ignoring the adapter                 | `the_plan_reads_each_rails_setting_else_its_adapters_default`                                                                                                                    |
| M4: `From<CallbackRegistrationDefault>` maps `Off` to `Ensure`                         | `an_adapters_default_maps_onto_the_same_setting`; `the_plan_reads_each_rails_setting_else_its_adapters_default`; `an_unset_register_leaves_a_notch_pay_account_alone`            |
| M5: `process_refusal`'s phone-only `422` arm removed                                   | `a_number_on_the_other_network_is_an_invalid_payer`; `the_declared_codes_are_exactly_what_the_mapping_can_produce`; `notchpay_a_number_on_the_other_network_is_an_invalid_payer` |
| M6: `names_only_the_payers_phone` drops `errors.len() == 1` (any `422` naming `phone`) | `a_number_on_the_other_network_is_an_invalid_payer` (`phone` beside `amount` must stay `provider_error`)                                                                         |
| M7: `InvalidPayer` removed from `PRODUCED_FAILURE_CODES`                               | `the_declared_codes_are_exactly_what_the_mapping_can_produce`; `the_declines_prove_every_code_each_rail_can_produce::case_3_notchpay`                                            |

Each run exited `100` with exactly the tests named failing, and every other
test in the run passing. After the last, `git diff HEAD -- backends` was
empty.

## Gates

On the final tree, `rustc 1.98.0`, `cargo-nextest 0.9.148`, a Linux cloud
session with Docker for the WireMock and Postgres containers:

| Gate                                                                                                                                                 | Result                                                                                     |
| ---------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| `cargo fmt --all -- --check`                                                                                                                         | clean                                                                                      |
| `cargo clippy --workspace --all-targets -- -D warnings`                                                                                              | clean, 0 warnings                                                                          |
| `cargo nextest run -p vpay-provider -p vpay-adapter-notchpay -p vpay-config -p vpay-worker -p vpay-api -p vpay-server --lib --bins`                  | **789 passed, 0 skipped**                                                                  |
| `cargo nextest run -p vpay-tests-conformance -p vpay-tests-integration --test adapter_conformance --test callback_registration --test confirm_rails` | **129 passed, 0 skipped** (103 conformance, 6 `callback_registration`, 20 `confirm_rails`) |
| `cargo test --doc -p vpay-provider -p vpay-adapter-notchpay -p vpay-config -p vpay-worker -p vpay-api`                                               | **43 passed, 0 ignored** (13, 1, 5, 5, 19)                                                 |
| `just verify`                                                                                                                                        | ok, the fifteen gates; `verify-doc-counts` 13 counts agree                                 |
| `prettier@3.9.6 --check` on every changed Markdown, YAML and JSON file                                                                               | clean                                                                                      |

Not run: the whole workspace's `cargo nextest run --workspace` (`just ci`),
the TypeScript, Flutter and Tauri suites, none of which this change
touches. `vpay-server` is a binary and has no doctests.

## Not done

- **Neither change has met Notch Pay.** vpay has not sent a wrong-network
  number to the sandbox; the probe has, and the stub answers what it was
  told. vpay has not run a deployment with `register` unset against the
  sandbox either; that path asks Notch Pay nothing by construction.
- **The confirm path's answer to a wrong-network number** is not exercised
  for Notch Pay end to end. A `Rejected` at submit is the generic decline
  path, proven for MTN's `invalid_payer` in `confirm_rails.rs`; no
  `confirm_rails.rs` case sends Notch Pay a mismatched number.
- **A dashboard-registered endpoint**, which is now how an operator gets
  Notch Pay's callbacks, has delivered nothing to vpay either. The webhook
  signature format (RFC-0012 § 10 item 7) is still unmeasured.
- **The stall** after the probe's endpoint (item 25) is a correlation. This
  change stops vpay creating such an endpoint unasked; it does not show the
  endpoint caused the stall.
- **The same `422` on initialize** stays `provider_error`. Notch Pay checked
  the number at process in every measured case; nothing showed initialize
  refusing one.
- **`register: ensure` on Notch Pay** still creates an endpoint the sandbox
  stores without a URL. It is honoured because the maintainer asked for a
  default, not a ban.
