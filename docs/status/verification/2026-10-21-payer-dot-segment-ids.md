# 2026-10-21 — the payer surfaces never put a dot segment in a path, now by test

Branch `claude/admiring-swirles-9b668f`, from `master` at `865420ba` (#274),
which already carries #306. **Tests and docs only: no shipping code in any
SDK changed, and no parser changed, because no test found a hole.** No route,
no server code, no migration, no gate and no `NotImplemented` token moved.

## Why

[2026-10-21-sdk-dot-segment-ids.md](2026-10-21-sdk-dot-segment-ids.md)
(#306) made both merchant SDKs refuse an empty or all-dots path id, because
URL parsers resolve `.`, `..` and `%2E%2E` before a request leaves. Its
§ "The change" read the three payer surfaces and found them not exposed, and
its § "What did not run" said so plainly: "The payer surfaces' safety was
established by reading, not by a test". This page closes that line.

The premise, re-measured on this machine before writing a test. The left
column is what a path built with the raw id resolves to, and the right one is
what it resolves to with the escaping each package applies:

```text
Node 24.14.1, new URL(`http://h/v1/browser/payment_intents/${id}`).pathname
"..",       raw -> /v1/browser/                     encodeURIComponent -> /v1/browser/
".",        raw -> /v1/browser/payment_intents/     encodeURIComponent -> /v1/browser/payment_intents/
"%2e%2e",   raw -> /v1/browser/                     encodeURIComponent -> /v1/browser/payment_intents/%252e%252e
"pi_/../x", raw -> /v1/browser/payment_intents/x    encodeURIComponent -> /v1/browser/payment_intents/pi_%2F..%2Fx

Dart 3.14.0, Uri.parse('https://h/v1/browser/payment_intents/$id').path
"..",       raw -> /v1/browser/                     Uri.encodeComponent -> /v1/browser/
".",        raw -> /v1/browser/payment_intents/     Uri.encodeComponent -> /v1/browser/payment_intents/
"%2e%2e",   raw -> /v1/browser/                     Uri.encodeComponent -> /v1/browser/payment_intents/%252e%252e
"pi_/../x", raw -> /v1/browser/payment_intents/x    Uri.encodeComponent -> /v1/browser/payment_intents/pi_%2F..%2Fx
```

Escaping handles `%` and `/`. It does nothing for `.` and `..`, which are
unreserved characters. So in both packages the **client-secret parser is the
only thing between a bare `..` and a path**: `parseClientSecret` in
`sdks/stripe-js/src/client.ts` and `_tryParseClientSecret` in
`sdks/flutter/vpay_checkout_flutter/lib/src/browser_client.dart`. Each
requires the id before `_secret_` to start with `pi_`/`cs_` and to be longer
than that prefix, so it can never be empty or only dots. That is the property
the new tests pin. The escaping is the second property, for an id that passes
the prefix rule and still carries a `/`.

## Every path each surface builds, and the tests that now cover it

**`@vaam-apps/vpay-stripe-js`.** Every path it builds takes its id from
`parseIntentSecret`/`parseSessionSecret`:

| Public method                                                       | Path                                            | Covered by                                                                                                |
| ------------------------------------------------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| `retrievePaymentIntent`, `handleNextAction`, `waitForPaymentIntent` | `GET /v1/browser/payment_intents/{id}`          | `refuses a client secret whose id is empty or only dots on every payment-intent read, sending nothing`    |
| `confirmPayment`, `confirmMobileMoneyPayment`                       | `POST /v1/browser/payment_intents/{id}/confirm` | `refuses a client secret whose id is empty or only dots on confirm, before any charge could be attempted` |
| `retrieveCheckoutSession`                                           | `GET /v1/browser/checkout/sessions/{id}`        | `refuses a client secret whose id is empty or only dots on retrieveCheckoutSession, sending nothing`      |
| `initEmbeddedCheckout`                                              | the frame's `{checkoutBaseUrl}/e/{id}`          | `refuses a session secret whose id is empty or only dots before building any frame src`                   |
| all three API routes                                                | an id carrying `/../`                           | `keeps an id that carries /../ inside one path segment on every route`                                    |
| `initEmbeddedCheckout`                                              | a session id carrying `/../`                    | `keeps a session id that carries /../ inside one segment of the frame src`                                |

`openCheckoutPopup` builds no path: it opens the hosted session `url` vpay
minted. The four refusal tests run every method through
sixteen secrets, listed once in `src/testing/unroutable-secrets.ts`, which
`tsconfig.build.json` keeps out of `dist`: `""`, `..`, `.`, `%2e%2e`, a bare
`pi_` and `cs_`, `.._secret_…`, `._secret_…`, `%2e%2e_secret_…`,
`%2E%2E_secret_…`, `_secret_…`, `re_123_secret_…`, `pi__secret_…`,
`cs__secret_x`, `pi_x_secret_` and `cs_x_secret_`. Each API-route result must
be `invalid_request_error` with `param: "clientSecret"`, and the real
`node:http` stub must have recorded **no** request. The embedded test asserts
a `TypeError` and no `iframe` in the document, with a `fetch` that fails the
test if called. The two `/../` tests read the path the stub received, or the
frame's resolved `src`, after the WHATWG parser has run.

**`vpay_checkout_flutter`.** `BrowserClient` builds the same three API paths,
each from `_parseSecretOrNull`. `CheckoutController`, `SheetController` and
`VpayCheckout.start` reach `/v1/browser` only through it. The package's only
other request is `checkout_page_config.dart`'s `GET
{checkoutBaseUrl}/.well-known/vpay-checkout`, a fixed path with no id in it.

| Method                    | Path                                            | Covered by                                                                                           |
| ------------------------- | ----------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `retrievePaymentIntent`   | `GET /v1/browser/payment_intents/{id}`          | `refuses a client secret whose id is empty or only dots on retrievePaymentIntent, sending nothing`   |
| `confirmPaymentIntent`    | `POST /v1/browser/payment_intents/{id}/confirm` | `refuses a client secret whose id is empty or only dots on confirmPaymentIntent, sending nothing`    |
| `retrieveCheckoutSession` | `GET /v1/browser/checkout/sessions/{id}`        | `refuses a client secret whose id is empty or only dots on retrieveCheckoutSession, sending nothing` |
| all three                 | an id carrying `/../`                           | `keeps an id that carries /../ inside one path segment on every route`                               |

The same sixteen secrets are copied into `test/browser_client_test.dart`, since
a Dart test cannot import a TypeScript file. A `MockClient` records every
request, and the three refusal tests assert that list is empty. The `/../`
test asserts both `Uri.path` (`…/pi_%2F..%2Fx`) and `Uri.pathSegments`,
where the id is one segment.

**`tauri-plugin-vpay-checkout`: no test, by construction.** It builds no
`/v1/browser` path. `guest-js/checkout.ts` reads the secret out of the
session URL's fragment and hands it to stripe-js's `retrieveCheckoutSession`.
The controller then calls `retrievePaymentIntent`, so both reads are covered
by the stripe-js rows above. The Rust (`src/desktop.rs`), Kotlin
(`VpayCheckoutPlugin.kt`, `VpayCheckoutActivity.kt`) and Swift
(`VpayCheckoutPlugin.swift`) hosts only parse and open the URL they are
handed. `guest-js/stop-url.ts` does substitute an id into the merchant's
`success_url`, but the id is the one the server's session read returned, and
the result is only matched against navigations, never requested. A change
that made this package build a request path would need its own test, which
[`docs/sdks/parity.md`](../../sdks/parity.md)'s Tauri section now says.

## What ran, on 2026-10-21 (macOS arm64, Node 24.14.1, Flutter 3.50.0-0.1.pre / Dart 3.14.0-214.0.dev)

```text
$ pnpm --filter @vaam-apps/vpay-stripe-js test     Test Files 9 passed (9); Tests 152 passed (152)
                                                   (146 before; +6)
$ pnpm exec vitest run -t "only dots|carries /\.\./"   Tests 6 passed | 146 skipped (152)
$ pnpm --filter @vaam-apps/vpay-stripe-js typecheck   exit 0
$ pnpm --filter @vaam-apps/vpay-stripe-js lint        exit 0   (eslint . --max-warnings 0)
$ flutter test   (sdks/flutter/vpay_checkout_flutter)  +341: All tests passed!   0 skipped, 0 failed
                                                   (337 before; +4)
$ dart analyze --fatal-infos                       No issues found!
$ dart format --set-exit-if-changed test/browser_client_test.dart   0 changed, exit 0
$ just verify-sdk-parity                           verify-sdk-parity: ok — 788 proving test(s) named in
                                                   docs/sdks/parity.md all exist, 45 dated gap(s),
                                                   35 SDK method(s) enumerated across 41 row(s)
                                                   (778 / 45 / 35 / 41 on master; +6 stripe-js, +4 Flutter)
```

`just verify`'s output is below the mutations.

### Mutations, each reverted after the run

| Mutation                                                                                          | Caught by                                                                                                                                             |
| ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| stripe-js M1: `parseClientSecret` drops `!id.startsWith(prefix)` and `id.length <= prefix.length` | the three `… only dots …` tests in `client.test.ts` and the embedded one: 4 failed, 2 passed                                                          |
| stripe-js M2: `#intentUrl` interpolates the raw id                                                | `keeps an id that carries /../ inside one path segment on every route`: 1 failed, 5 passed                                                            |
| stripe-js M3: `embeddedFrameSrc` interpolates the raw session id                                  | `keeps a session id that carries /../ inside one segment of the frame src`: 1 failed, 5 passed                                                        |
| Flutter M1: `_tryParseClientSecret` keeps only `suffix.isEmpty`                                   | the three refusal tests, each with `Expected: 'invalid_request_error'`: 3 failed, 1 passed                                                            |
| Flutter M2: `retrievePaymentIntent` interpolates the raw id                                       | `keeps an id that carries /../ inside one path segment on every route`, whose path list showed the resolved `…/payment_intents/x`: 1 failed, 3 passed |

After each run, `git status` showed only the test files and the new fixture
as modified.

### The gates, on the finished tree

```text
$ just verify                                      exit 0
  verify: ok — the fifteen gates above passed; the verify-docs report is advisory
  verify-sdk-parity: ok — 788 proving test(s) named in docs/sdks/parity.md all exist, 45 dated gap(s),
    35 SDK method(s) enumerated across 41 row(s)
  verify-links: ok — 2476 repository link(s) in 466 tracked markdown file(s) resolve to a tracked path
  (re-run after master was merged in at 73a5b5af and the flutter-example-lockfile page added:
   verify: ok, verify-sdk-parity 788 / 45, verify-links 2507 links in 468 files, verify-doc-counts ok)
  verify-doc-counts: ok — 13 documented count(s) in 11 of 294 markdown file(s) agree with what this tree measures
$ npx markdownlint-cli2@0.20.0, super-linter's default config, on the six Markdown files this change touches
                                                   0 errors (0 on master for the five that exist there)
```

`verify-doc-counts` reads the verification directory's file count off
[../README.md](../README.md), which this page moved from 84 to 85 (86 after `master`'s notchpay-step5 page landed, 87 with the flutter-example-lockfile page).

## What did not run

- **No running vpay.** Every test here runs against a stub (`node:http` for
  stripe-js, `MockClient` for Flutter) and asserts that nothing was sent.
  What a deployed vpay or an ingress would do with such a request was
  measured for the merchant SDKs on #306's page and is not re-measured here,
  because no such request can now leave either package.
- **No CI runs the Flutter half.** There is no Flutter job (D-M3), so the
  341 above is this machine's run alone, on the branch's tree, and on
  Flutter 3.50.0-0.1.pre rather than the version `flutter-toolchain.toml`
  pins.
- **Browsers other than jsdom.** The embedded test resolves the frame's
  `src` with jsdom's WHATWG URL implementation. No real browser loaded it.
- `pnpm install` was run with `CYPRESS_INSTALL_BINARY=0`, as `CLAUDE.md`
  allows, because the Cypress binary download stalled. Nothing here uses
  Cypress.
- `just fmt-check-web` fails on this machine on one file,
  `.claude/settings.local.json`, as it did for #306. That is a local harness
  file the user's global gitignore excludes, so CI never sees it. Every
  tracked file passes `prettier --check`.
- **Not `just ci` in full.** No Rust changed, so the workspace's `cargo
nextest`, clippy and doctests were not rerun. The stripe-js suite,
  typecheck and lint, the Flutter suite, analyzer and formatter, and
  `just verify` are what this change can move, and all of them ran.
