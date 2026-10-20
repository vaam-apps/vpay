# Verification — the `notchpay` adapter (2026-10-20)

RFC-0012 delivery steps 0, 2, 3 and 4, on branch `claude/notchpay-adapter`
from `master` at `b8be107`. Linux (the cloud container), Rust 1.98.0,
`cratestack` 0.15.0, Docker 29.9.1 with `wiremock/wiremock:3.9.2` and
`postgres:16-alpine`.

## What was **not** run, first

- **No call to Notch Pay, of any kind.** `NOTCHPAY_TEST_PUBLIC_KEY` and
  `NOTCHPAY_TEST_PRIVATE_KEY` were absent from the environment, so RFC-0012
  delivery step 1 (the sandbox session: § 10 items 2–5, 7 and 11) and step 5
  (the first test-mode call through the adapter) did not happen. Every
  mapping is transcribed from `openapi.yaml` 2.1.0 and marked "To confirm".
  The `notchpay` Collect and Status read cells of
  [providers.md](../providers.md) say "stub-only; never called", and nothing
  on this page is evidence about Notch Pay.
- **Not the whole `just ci`.** `cargo nextest run --workspace` was not run in
  one invocation: a full workspace test build exhausted the disk allowance
  here. It ran in the slices below, and CI's `rust` job is the whole-workspace
  evidence. Of `backends/tests/integration`'s 22 binaries, four ran
  (`boot_coherence`, `provider_callback`, `refunds`, `confirm_rails`). The
  other eighteen were compiled (by `cargo clippy --workspace --all-targets`)
  and not run here.
- **`verify-ignored`'s binary count is counted, not measured.** 51 → 52 from
  `cargo metadata --no-deps` diffed against `master`: the one new target is
  `vpay-adapter-notchpay`'s lib. The expected total is **2306**, derived as
  `master`'s 2232 (its own CI measurement) plus the 74 tests this change adds
  (below: 68 in the first commit, six with the livemode acknowledgement). CI
  is the measurement.
- **No e2e or demo walk.** No compose stack enables `notchpay`, so
  `just test-e2e` would not touch it and was not run.

## Gates

`just verify`, one invocation, exit 0. The fifteen gates as printed:

| Gate                       | Printed                                                                                            |
| -------------------------- | -------------------------------------------------------------------------------------------------- |
| `verify-no-mocks`          | no test double reachable from a shipping binary                                                    |
| `verify-status`            | **2** unimplemented items (`orange_money::refund`, `notchpay::refund`), all declared               |
| `verify-errors`            | 20 error types, 17 `#[from]` variants                                                              |
| `verify-sdk-parity`        | 769 proving tests, 45 dated gaps, 35 SDK methods, 41 rows                                          |
| `verify-links`             | 2387 links in 458 tracked markdown files                                                           |
| `verify-npm-scope`         | 3 publishable packages, 1 private                                                                  |
| `check-schema`             | 30 model/enum declarations under cratestack 0.15.0                                                 |
| `verify-serde`             | 123 types, 17 exemptions                                                                           |
| `verify-repositories`      | 4 implementations, 95 source files outside                                                         |
| `verify-toolchain`         | 1.98.0                                                                                             |
| `verify-ui`                | silent, exit 0                                                                                     |
| `verify-migrations`        | 50 files                                                                                           |
| `verify-versions`          | 26 version references, all 0.8.0                                                                   |
| `verify-privacy-inventory` | 314 columns / 26 elements / 10 surfaces; 36 `debug_protections`                                    |
| `verify-doc-counts`        | 13 counts in 11 of 292 markdown files. It first **failed**, on this change's own new page: 78 → 79 |

`cargo clippy --workspace --all-targets -- -D warnings`: exit 0, after eight
`indexing_slicing` findings in this change's own tests were fixed.
`cargo fmt --all -- --check`: clean.

## Tests

`cargo nextest run`, by package, every one this change touched:

| Package                  | Tests | Ignored | This change added                                                                  |
| ------------------------ | ----: | ------: | ---------------------------------------------------------------------------------- |
| `vpay-adapter-notchpay`  |    29 |       0 | 29 (new crate: 17 in `tests.rs`, 12 in `mapping.rs`)                               |
| `vpay-notchpay`          |    35 |       0 | 1 (the per-request deadline)                                                       |
| `vpay-provider`          |    40 |       0 | 1 (`Measured` forwards the exposure)                                               |
| `vpay-config`            |   142 |       0 | 12 (the `notchpay` boot rules; 5 of them the acknowledgement)                      |
| `vpay-api`               |   429 |       0 | 2 (which rails are warned about; the warning's text)                               |
| `vpay-server`            |    51 |       0 | 3 (the real binary's boot warning, sandbox and livemode; only `notchpay` declares) |
| `vpay-sdk` (Rust)        |   187 |       0 | 1 (confirm on `notchpay`)                                                          |
| `xtask`                  |   315 |       0 | 0                                                                                  |
| `vpay-tests-conformance` |    92 |       0 | 25 (the `notchpay` row, against a real WireMock container)                         |

The first eight in one invocation: **1222 run, 1222 passed, 0 skipped**,
before the livemode acknowledgement; `vpay-config`, `vpay-api` and
`vpay-server` were re-run after it (below). The
conformance suite on its own: **92 run, 92 passed**, 169 s.

`backends/tests/integration`, four binaries, against a real Postgres: **47
run, 46 passed, 1 failed** on the first invocation.
`refunds::a_replayed_key_answers_the_stored_refund_and_creates_no_second`
failed after 63 s. It **passed** run alone (6 s), and the `refunds` and
`provider_callback` binaries together then ran **33 of 33**. This change
touches neither the integration crate nor `vpay_api::v1::refunds`. The first
failure is recorded, not explained: it was not reproduced, and no cause was
found.

`just test-doc`: **130 passed, 1 ignored** (the one ignored is the workspace's
existing one).

`sdks/nodejs`: typecheck and lint clean, **230 tests passed**.
`examples/shop`: **108 passed**. `pnpm -r typecheck` and `pnpm -r lint`
clean, after `pnpm --filter @vaam-apps/vpay-sdk build`, which CI also runs
first.

## The conformance row can fail

Two mutations, each reverted after it was measured:

1. **Delete `.with_request_timeout(config.request_timeout)`** from the
   adapter's client construction:
   `an_unavailable_rail_is_a_transport_error_never_a_decline::case_3_notchpay`
   **fails**. The stub's slow read (2 s) outlasts the 100 ms the case
   configures, and only the adapter applying the configured deadline makes
   it a `Transport` error in time.
2. **Stop sending `X-Grant`** in `vpay_notchpay::Client`:
   `submit_returns_a_reference_and_a_flow_shaped_result`,
   `pending_then_successful_walks_the_scenario` and
   `a_declined_charge_maps_to_the_documented_failure_code` **fail** on
   `notchpay`. Every stub mapping requires the header (ADR-0031 condition 1),
   so an adapter that stopped sending the private key gets WireMock's `404`,
   which it reads as `Config`.

## What the boot warning looks like

`vpay-server`'s `a_notchpay_rail_is_warned_about_at_boot_and_its_keys_are_never_printed`
runs the real binary on `tests/fixtures/notchpay-config.yml` (sandbox) and
asserts the `WARN` line names the rail, ADR-0031, `credentials.public_key`
and `docs/flows/adapter-notchpay.md`, and that neither key appears anywhere
in the output. The livemode text (`LIVEMODE, REAL MONEY: …`) is asserted in
`vpay-api`'s
`the_boot_warning_names_the_exposure_louder_in_livemode_and_never_a_key`, on
a captured subscriber. No livemode process was booted.

## The livemode acknowledgement (added the same day)

The maintainer answered the question this change's pull request put to them:
"Livemode acknowledgement: YES, require it." ADR-0031 condition 4 was amended
in place. `REQUIRED_RAIL_KEYS` gained a fourth column,
`livemode_acknowledgement`, set on the `notchpay` row to
`settings.accept_public_key_exposure` for `credentials.public_key`, and
`ConfigError::PublicKeyExposureNotAcknowledged` is the refusal.

| Case                                       | Test                                                                                                                                                                       |
| ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Livemode, no acknowledgement or `false`    | `a_livemode_notchpay_block_without_the_acknowledgement_is_refused` (config); the first half of `a_livemode_notchpay_rail_boots_only_acknowledged_and_still_warns` (binary) |
| The refusal names, and never prints a key  | `the_acknowledgement_refusal_names_the_setting_and_the_key_never_a_value` (config); both halves of the binary test assert neither key reaches the output                   |
| Livemode, acknowledged: boots, still warns | `a_livemode_notchpay_block_written_as_the_docs_show_loads` (config, an unquoted YAML `true`); the binary test's second half, which asserts `LIVEMODE, REAL MONEY`          |
| Sandbox, no acknowledgement: warns only    | `a_sandbox_notchpay_block_needs_no_acknowledgement` (config); `a_notchpay_rail_is_warned_about_at_boot_and_its_keys_are_never_printed` (binary, unchanged)                 |
| Only `true` or `false`                     | `the_acknowledgement_is_true_or_false_and_nothing_else`                                                                                                                    |

`cargo nextest run -p vpay-config`: **142 run, 142 passed**. The `notchpay`
tests of `vpay-server` (3) and the exposure tests of `vpay-api` (2): all
passed. **Mutation**: disabling the livemode check (`if false && livemode …`)
fails three `vpay-config` tests —
`a_livemode_notchpay_block_without_the_acknowledgement_is_refused`,
`a_livemode_notchpay_block_written_as_the_docs_show_loads` and
`the_acknowledgement_refusal_names_the_setting_and_the_key_never_a_value` —
and was reverted. The binary test was run against the real `vpay-server`;
it stops on the missing signing key after the warning, so no livemode
process served traffic and nothing called Notch Pay.
