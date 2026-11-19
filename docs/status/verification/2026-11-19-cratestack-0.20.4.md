# 2026-11-19 — CrateStack 0.16.0 → 0.20.4 (issue #412)

Branch `chore/412-cratestack-0.20.4`, from `master` at `acafba2`. The area record
is [../cratestack.md](../cratestack.md), "CrateStack 0.16.0 → 0.20.4".

## Environment

- macOS (aarch64) with Docker Desktop, so the testcontainers suites ran.
- `cratestack` CLI 0.20.4, built with `cargo install cratestack-cli --locked
--version 0.20.4`; `just check-schema` printed "cratestack 0.20.4".
- `wasm-bindgen` 0.2.128 (the lock's pin), installed the same way, for the Node SDK's
  wasm build.
- Toolchain 1.98.0 (`rust-toolchain.toml`).

## Gates

| Step                                           | Result                                                                                                                                                                                                                   |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `just fmt-check`, `just clippy`                | passed                                                                                                                                                                                                                   |
| `just verify`                                  | "verify: ok — the eighteen gates above passed" (`check-schema` included, with a regenerated `dash-client`)                                                                                                               |
| `cargo nextest run --workspace --no-fail-fast` | 2799 run, 2792 passed, 7 failed — see below                                                                                                                                                                              |
| the 7 failures, re-run                         | 3 passed on re-run (two Docker `PortNotExposed` starts of the WireMock container, one a missing `wasm-bindgen` that was then installed); **4 did not pass here, all `vpay-tests-integration::staff_sign_in`**, see below |
| `just test-doc`                                | passed                                                                                                                                                                                                                   |
| `just verify-ignored`                          | "0 ignored (expected 0), 59 test binaries (expected 59), 2799 total"; `expected_suites` 58 → 59 for `wire_golden.rs`                                                                                                     |
| `just lint-web`, `just test-web`               | passed (with `wasm-bindgen` installed)                                                                                                                                                                                   |
| `just audit-web`, `just deny`                  | passed                                                                                                                                                                                                                   |
| `just check-schema`                            | passed; the postgres drift test `the_cstack_schema_drifts_from_the_migrations_by_a_measured_amount` passed unchanged (232 changes, 31 relations) under the 0.20.4 CLI                                                    |

**The four that did not pass** (`a_forwarded_for_header_from_an_untrusted_peer_buys_no_fresh_budget`,
`the_second_factor_is_rate_limited_and_not_only_the_password`,
`the_sign_in_rate_limit_is_per_source_address`, `two_replicas_share_one_sign_in_budget`)
bind source addresses `127.0.0.2`, `.4`, `.20` and `.30` through `client_bound_to`
(`backends/tests/integration/tests/support/mod.rs`), which macOS does not assign to `lo0`
(`EADDRNOTAVAIL`, os error 49; the test's message names the `sudo ifconfig lo0 alias` fix).
The failure is environmental, but these tests do go through `vpay-db`'s rate limit (the
generated `RATE_LIMIT_WINDOW_MODEL`), which is CrateStack-generated code: **they still need the Linux CI run on 0.20.4.**

**Not run:** `just test-e2e` (the compose stack: builds images and boots the rails),
`just test-storybook`, `just helm-check`. CI's `e2e` job is the gate of record for the
first.

## Wire compatibility

Method and results are in [../cratestack.md](../cratestack.md). The pairing and refusal
tests ran in a throwaway crate (not committed); the committed evidence is
`sdks/rust/tests/wire_golden.rs`, which drives CrateStack's client envelope and
`EnvelopeLayer` only. Its guards were each broken once to prove a failure:

| Mutation                                                                                                                | Failed                                                                                                                                                                                                                                                                                                                                                           |
| ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| server skew widened to 1,000,000 s                                                                                      | `…refused_outside_the_clock_skew`                                                                                                                                                                                                                                                                                                                                |
| replay memory dropped (fresh store for the second request)                                                              | `…refused_the_second_time`                                                                                                                                                                                                                                                                                                                                       |
| "unknown signer" given the real merchant key                                                                            | `…tampered_or_foreign…`                                                                                                                                                                                                                                                                                                                                          |
| one hex digit of the golden request changed                                                                             | 4 tests, including the seal and open ones                                                                                                                                                                                                                                                                                                                        |
| the foreign-audience layer given the sealed audience                                                                    | `…tampered_or_foreign…`                                                                                                                                                                                                                                                                                                                                          |
| client pins a different server key                                                                                      | `…opens_a_0_16_0_answer_under_the_pinned_key`                                                                                                                                                                                                                                                                                                                    |
| signer clock moved by one second                                                                                        | the seal and open tests                                                                                                                                                                                                                                                                                                                                          |
| **signature verification disabled** in a patched `cratestack-cose` 0.20.4 (`MUT_SIG=1`, one `key.verify` call bypassed) | the flipped-payload-byte case in `…tampered_or_foreign…`, and `a_0_16_0_answer_does_not_open_for_another_request` (the 0.16.0 answer's signature is what refuses it). With the variable unset the patched crate passes all 7; the real crate passes all 7. Run alone with the payload offset removed, the flipped-signature-byte case fails the same way (`a flipped signature byte`: 201 where 401 was expected) |

**Correction (review of this branch).** The first version of the tamper case flipped byte 66
of 132, which is the CBOR length byte `0x58` before the signature, so its `401` came from
the CBOR parser and still passed with signature verification off. It now flips a payload
byte (offset 50) and a signature byte (offset 100) and asserts the handler never ran.

## Outside workspace

A fixture workspace pinned to `=0.20.4` with `vpay-sdk = { path = "…/sdks/rust" }` builds, and its
`Cargo.lock` holds one version of each of the 13 `cratestack-*` packages it links, all
`0.20.4`. The control (the same fixture against `master`'s SDK) locks `cratestack-core`
at both `0.16.0` and `0.20.4`.
