# `verify-ignored`: 59 -> 63 test binaries

_Written 2026-11-28 on a branch from `origin/master` (`7a9fee5`)._

## What was wrong

`just verify-ignored` was red on master since #426 (`48a520e`, the OpenTelemetry export). That change added the
`vpay-telemetry` crate and its test binaries and did not raise `expected_suites` in the `justfile`. CI's `rust` job
said: `verify-ignored: 0 ignored (expected 0), 63 test binaries (expected 59), 2855 total (minimum 1080)`.

## The four new binaries

Found by diffing `git diff 2aeb2c3 48a520e --name-status` (added `tests/*.rs` and the crate) against the suite list
printed by the recipe in the CI log:

| Binary | Source |
| --- | --- |
| `vpay-telemetry` | the crate's lib target (unit tests) |
| `vpay-telemetry::dashboards` | `backends/crates/vpay-telemetry/tests/dashboards.rs` |
| `vpay-telemetry::export` | `backends/crates/vpay-telemetry/tests/export.rs` |
| `vpay-telemetry::hang` | `backends/crates/vpay-telemetry/tests/hang.rs` |

`vpay-server::cli` was already counted before #426 and is not new.

## Measured

Read from CI, not run here (the disk is nearly full, so no test build was made): run 46079302465, job 140774430782
(#428 at `aacb1b5`, now master `7a9fee5`):
`verify-ignored: 0 ignored (expected 0), 63 test binaries (expected 59), 2859 total (minimum 1080)`.
The earlier master run (46077024438) read 2855 total; #428 added four tests.

`expected_suites` is now 63. `min_tests` is a floor and did not move.

## Not run

`just verify-ignored` itself (it builds every test binary), and nextest.
