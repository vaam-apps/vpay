# Lane D — the Flutter plugin's Dart core against a real, running vpay

**Date:** 2026-09-14. **Branch:** `claude/flutter-e2e-real-stack`, worktree
`/home/selast/dev/vpay-flutter-e2e`. **Host:** Linux, Flutter 3.47.2 / Dart
3.13.2 ([`flutter-toolchain.toml`](../../../flutter-toolchain.toml)), Node
22.23.2 (`.nvmrc`), rootless Docker. **No macOS, no iOS toolchain, no
device, no browser, no real rail.**

Every exit code below was read from a file, never from a harness banner —
`<command> > <log> 2>&1; echo $? > <file>` — because this repository has
measured a background run reporting exit 0 for a run that exited 1.

## What this proves, in one sentence

Until this lane, every server `sdks/flutter/vpay_checkout_flutter`'s test
suite ever spoke to was `package:http/testing.dart`'s `MockClient`. Now
`test_e2e/real_stack_e2e_test.dart`, run only by `just test-flutter-e2e`,
drives the package's own `BrowserClient`/`CheckoutController` with a real
`http.Client` against a real, running `vpay-server` — on a session minted
through `examples/shop`'s real server with a real `private_key_jwt`
exchange — and it found a real bug no mock fixture could.

## Gates

| Gate                                                                                         | Exit code | What it printed                                                                                                                                                            |
| -------------------------------------------------------------------------------------------- | --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `just test-flutter-e2e`, stack UP                                                            | `0`       | 3 passed — real `cs_…`/`pi_…` ids in the log (see below), settling to `VpayCheckoutSucceeded`, re-confirmed independently twice more                                       |
| `just demo_port=19999 demo_shop_port=19998 test-flutter-e2e`, stack DOWN (THE DECISIVE TEST) | `1`       | fails at the very first `curl -fsS .../healthz` check, before any Flutter process runs: `FAIL — nothing answers http://localhost:19999/healthz.`                           |
| `just test-flutter` (unit, stack UP)                                                         | `0`       | **80 passed, 0 skipped**                                                                                                                                                   |
| `just test-flutter` (unit, stack DOWN) — implied by the recipe never opening a socket        | `0`       | `test/` contains no `http.Client()` construction anywhere (`grep`ped); every test there is `MockClient`-only by construction, so this is unconditionally stack-independent |
| `dart analyze --fatal-infos`                                                                 | `0`       | No issues found                                                                                                                                                            |
| `dart format --set-exit-if-changed .`                                                        | `0`       | 24 files, 0 changed                                                                                                                                                        |
| `cargo run -p xtask -- verify-sdk-parity`                                                    | `0`       | **554 proving tests** (551 baseline + 3 new), **35 dated gaps** (unchanged — one ⛔ became one ✅ + one narrower ⛔)                                                       |
| `cargo run -p xtask -- verify-links`                                                         | `0`       | clean, after this page was added (see "What broke first", below)                                                                                                           |
| `just fmt-check-web`                                                                         | `0`       | clean — see the log excerpt below                                                                                                                                          |

`just ci` was not run, by instruction (D-M3: none of the Flutter recipes are
in it).

## The real ids, from an actual `just test-flutter-e2e` run

```
test-flutter-e2e: checking http://localhost:8080/healthz
test-flutter-e2e: stack is up
test-flutter-e2e: shop container is client_id=shop-merchant
test-flutter-e2e: minted 2 real checkout sessions (real private_key_jwt, real cs_… ids)
test-flutter-e2e: expired session cs_tjt2gqmew95y1byqjd3yvd1g (real private_key_jwt token, real 200)
test-flutter-e2e: fixture written — driving the plugin's own BrowserClient/CheckoutController
00:00 +0: real stack — BrowserClient + CheckoutController end to end mints, preflights, confirms and polls a real session to succeeded
Shell: [real_stack_e2e] preflight ok — session cs_h2k16900cd47nc1jfqfqq23w, intent pi_7ffcr32bss4xxa20t4kb3m8x
Shell: [real_stack_e2e] confirmed pi pi_7ffcr32bss4xxa20t4kb3m8x — status requires_action, next_action present: true
Shell: [real_stack_e2e] resolved -> VpayCheckoutSucceeded(sessionId: cs_h2k16900cd47nc1jfqfqq23w, paymentIntentId: pi_7ffcr32bss4xxa20t4kb3m8x)
Shell: [real_stack_e2e] raw HTTP confirms it too: pi pi_7ffcr32bss4xxa20t4kb3m8x is succeeded on the server
00:12 +1: real stack — the uniform 404 an unknown id, a wrong secret and a wrong key all answer the same 404
Shell: [real_stack_e2e] unknown id / wrong secret / wrong key all -> the same 404 (resource_missing), against the real server
00:12 +2: real stack — a session that is not open refuses the confirm the intent read still answers, the pre-flight fails closed, and the confirm is refused with checkout_session_expired
Shell: [real_stack_e2e] confirm on an expired session -> 409 checkout_session_expired (real, against the running server)
00:12 +3: All tests passed!
```

`pi_7ffcr32bss4xxa20t4kb3m8x` settles the way
`examples/merchant-demo`'s own outcome table documents: 12 000 XAF (one
`njangi-tote`) matches no amount-keyed WireMock mapping, so `vpay-worker`'s
status poll answers `PENDING` once and the catch-all `SUCCESS` on the next
rung, about ten seconds later — proven end to end by the real 12-second
runtime above, not asserted.

## The real bug this test found and fixed

`CheckoutSession.fromJson` required a top-level `client_secret` key.
`vpay_api::browser::checkout_sessions::retrieve`'s own doc comment says the
route never re-serves the session's own secret — only the intent's, and
only while the session is `open` — but every fixture in `test/` had been
fabricating that key, so `flutter test` stayed green while this exact call
against a real server threw a cast error inside `fromJson`, caught by
`_decode`, and surfaced as `CheckoutPreflightFailure(unexpectedResponse(200))`.
Measured before the fix:

```
preflight refused a fresh, open session: VpayError(type: api_error, code: unexpected_response, message: The vpay API returned an unexpected response (HTTP 200)., param: null)
```

Fixed by making `CheckoutSession.fromJson` take `clientSecret` as a
parameter — the value the caller already authenticated the read with —
instead of reading it off a body that will never carry it. Every existing
`MockClient` fixture that had been asserting the wrong field is updated to
drop it; none of them was testing that key's absence or corruption
specifically, so nothing was weakened. Full account:
`git log` on this branch, the commit titled "fix(flutter):
CheckoutSession.clientSecret is caller-supplied, not a wire field".

A second, smaller bug in the **test harness itself** (not the package): the
first version of this suite's raw confirm call used
`http.Client.post(uri, body: {...})`, whose `Map<String, String>` helper
percent-encodes the KEY too (`payment_method_data%5Btype%5D`). vpay's form
grammar (`backends/crates/vpay-api/src/form.rs`) splits a key on its raw,
unescaped `[`/`]` by design — brackets are structural wire syntax, never
percent-encoded, matching `sdks/nodejs/src/form.ts`'s own encoder — so the
real server answered `400 payment_method_data[type]` on the Dart `http`
package's own encoding. Fixed by building that one request body by hand.

## What broke first, and how it was closed

- `cargo run -p xtask -- verify-sdk-parity` failed the first time the new
  parity row was written, because every backtick span in a ✅ cell is
  checked as a claimed test name — not just the ones meant as test names.
  The first draft's cell also backtick-quoted `just test-flutter-e2e`,
  `compose.demo.yml`, `examples/shop`, `private_key_jwt`, `MockClient` and
  `CheckoutSession.fromJson`, each of which then failed as "does not exist
  under `sdks/flutter/vpay_checkout_flutter`". Fixed by keeping only the
  three real test titles in backticks and de-quoting everything else.
- `dart_test_names` (`.xtask`) is a character walk that stops at the first
  closing quote — it does not concatenate adjacent Dart string literals the
  way `dart format` may wrap a long title across two. Two of this suite's
  test titles were originally split across two adjacent literals; cited in
  full in `docs/sdks/parity.md` they would never be found. Rewritten as
  single literals.
- `cargo run -p xtask -- verify-links` failed once, pointing at this very
  page before it existed — expected, and resolved by writing it.

## `just fmt-check-web`

Repo-wide over markdown and JSON too, not just the frontend — every
markdown file this lane touched (`docs/sdks/parity.md`,
`docs/status/mobile-flutter-plugin.md`, `docs/flows/mobile-checkout.md`,
`sdks/flutter/vpay_checkout_flutter/README.md`, and this page) needed
`pnpm exec prettier --write` at least once before this gate went green —
table column widths, mostly. First run:

```
$ just fmt-check-web
pnpm exec prettier --check .
Checking formatting...
[warn] docs/sdks/parity.md
[warn] docs/status/mobile-flutter-plugin.md
[warn] docs/status/verification/2026-09-14-flutter-e2e-real-stack.md
[warn] Code style issues found in 3 files. Run Prettier with --write to fix.
error: Recipe `fmt-check-web` failed on line 889 with exit code 1
```

After `pnpm exec prettier --write` on those three files, exit `0`, clean.
