# Issue #65 verification: popup completion on two channels, and the shop's `Referrer-Policy`

_Written 2026-11-17 while the work was done, on `impl/65-popup-completion`
over `master` at `ea469b40`. Every number below was measured on the tree this
page was committed with. **Nothing here opened a real popup**: every window
in the SDK's tests is a stub, as before._

What was built is in [docs/status/merchant-sdks.md](../merchant-sdks.md)
("Popup completion: vpay's page first, `success_url` as fallback, once per
session"), in `sdks/stripe-js/README.md` ("The popup, and its two completion
channels"), in `examples/shop/README.md` ("The popup, and the two messages
that can complete it") and in
[hosted-checkout/page-memory-and-protocols.md](../../flows/hosted-checkout/page-memory-and-protocols.md)
("Who listens on the opener's side"). This page is what was read, what was
run, what was broken on purpose, and what could not be run.

## Facts read in the tree before relying on them

| Fact                                                                                                                                                                              | Where it was read                                                                                                       |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| vpay's page posts `{type: 'vpay:complete', session, status}` — the session id and the session's re-read status — at most once, to the opener when its channel is `peer: 'opener'` | `CompleteMessage` in `frontends/apps/checkout/src/lib/frame.ts`; `#announceComplete` in `controller.ts` and `return.ts` |
| The payment page pins the opener with `resolveParentOrigin(document.referrer, checkout_origins)` and opens no channel for an empty referrer                                       | `lib/origins.ts`, `lib/entry.ts`, `components/checkout-client.tsx`                                                      |
| With no channel, the page's "Back to {merchant}" navigates the popup itself to the forwarding URL                                                                                 | `CheckoutController.forward`, `ReturnController.forward`                                                                |
| The return page after a redirect rail pins by `soleOrigin`, not the referrer                                                                                                      | `components/return-client.tsx`, `lib/origins.ts`                                                                        |
| `examples/shop` passes `checkoutBaseUrl` (`VPAY_CHECKOUT_URL`) to `loadStripe` on its popup path, and set no `Referrer-Policy` anywhere before this change                        | `src/components/checkout-form.tsx`; `next.config.ts`; a grep of `src/` for `referrer`                                   |
| The page's message already names the session, so it was **not** extended                                                                                                          | as above; `controller.test.ts` and `return.test.ts` already assert the three-field shape                                |

## What else opens a popup, and was not changed

`sdks/nodejs` and `sdks/rust` have no popup. Two other payer surfaces do, and
both still pin the **merchant's** origin only, so they ignore vpay's own
message and complete on the `success_url` fallback:
`sdks/tauri/tauri-plugin-vpay-checkout/guest-js/host-web.ts` (`event.origin
!== resolved.location.origin`) and
`sdks/flutter/vpay_checkout_flutter/lib/src/platform/web_checkout_platform.dart`
(`messageEvent.origin != web.window.location.origin`). Out of this issue's
scope (it names the shop SDK of #64); recorded on
[merchant-sdks.md](../merchant-sdks.md).

## Tests, and the mutations that turn them red

`sdks/stripe-js/src/popup.test.ts`: **39** cases (27 before; 12 new). Each
mutation below was applied to the shipping source, the suite run, and the
file restored from a copy:

| Mutation                                                    | Cases that failed                                                                                                     |
| ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| M1 — primary channel never matches (`fromCheckout = false`) | 8 (every primary-channel case, including the `loadStripe` wiring)                                                     |
| M2 — no per-session dedupe                                  | 1: `drops a session already reported even on a later popup that reused the opener, and not a different one`           |
| M3 — no dedupe **and** no `stop()` on completion            | 5, including both orders of arrival (`fires once when vpay's page reports first…`, `…the return page reports first…`) |
| M4 — no `event.source` check                                | 2: the fallback's and the primary's wrong-window cases                                                                |
| M5 — empty `session` accepted                               | 1: `ignores a malformed message from the checkout origin without spending the session`                                |
| M6 — `client.ts` does not pass `checkoutBaseUrl`'s origin   | 1: `pins vpay's message to checkoutBaseUrl's origin`                                                                  |
| M7 — dedupe once per opener instead of per session          | 1: the reused-window case (a different session must still complete)                                                   |
| M8 — dedupe record per popup instead of per opener          | 1: the reused-window case                                                                                             |

**Review fix (same day):** a session reported as `open` (the payer pressed
"Back to merchant" before paying) can be reopened, and a permanent claim on it
would have dropped its real completion and left the handle waiting. Such a
claim now lasts only until the next `openCheckoutPopup` on that window; every
other status stays claimed. Two cases were added (**41** in the file, **166**
in the package): `completes a session reopened after the payer left it open…`
fails with the release removed (M9), and `keeps a session that ended in
anything but open claimed across later popups` fails, with the reused-window
case, when every claim is released (M10).

Both orders of arrival are protected by either mechanism alone — the
listener is removed on the first completion, and the session record would
drop the second — so only M3, which removes both, turns them red.

`examples/shop/src/referrer-policy.test.ts`: 2 cases. `REFERRER_POLICY =
"no-referrer"` fails the first; the catch-all `source` narrowed to
`/checkout` fails the second.

## Commands run, and what they answered

| Command                                                                          | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| -------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pnpm --filter @vaam-apps/vpay-stripe-js test`                                   | 9 files, **166 passed**, 0 failed, 0 skipped (164 before the review fix)                                                                                                                                                                                                                                                                                                                                                                                                      |
| `pnpm --filter @vaam-apps/vpay-stripe-js typecheck` / `lint`                     | clean / clean (`--max-warnings 0`)                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `examples/shop`: `tsc --noEmit`, `eslint . --max-warnings 0`, `prettier --check` | clean, after building `@vaam-apps/vpay-sdk`'s TypeScript half only (`build:ts`) — see below                                                                                                                                                                                                                                                                                                                                                                                   |
| `examples/shop`: `vitest run`                                                    | 14 files: **83 passed, 26 failed**, 0 skipped. All 26 are in `src/server/orders.test.ts` (20) and `src/server/vpay.test.ts` (6), and fail on `Cannot find module '../../wasm-test/vpay_sdk_wasm.js'` — see below                                                                                                                                                                                                                                                              |
| `prettier --check` over every touched doc and package                            | clean                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `just verify`                                                                    | `verify: ok — the seventeen gates above passed`, run on the committed tree. Among them: `verify-sdk-parity: ok — 749 proving test(s) … 42 dated gap(s)`; `verify-links: ok — 3224 repository link(s) in 535 tracked markdown file(s)`; `verify-doc-counts: ok — 9 documented count(s)`. A first run with this page still untracked passed `verify-doc-counts` and failed it once the page was committed: `docs/status/README.md` said 134 verification pages and now says 135 |

## What could not be run here

- **`examples/shop`'s package scripts.** `test`, `typecheck` and `lint` all
  run `build:deps`, which builds `@vaam-apps/vpay-sdk`'s WebAssembly core and
  needs `wasm-bindgen` (Cargo.lock pins 0.2.128); it is not installed in this
  environment. The checks above were run directly instead. The 26 failing
  shop cases need that WebAssembly build at runtime and touch nothing this
  change touched; CI's `web` job builds it.
- **A real browser.** No Cypress spec drives the popup; the parity matrix's
  dated ⛔ row for it stands.
- **`frontends/apps/checkout`'s suite** was not run: nothing in that app
  changed.
