# Issue #189 lane 2 — the native checkout sheet

**Date:** 2026-09-17. **Branch:** `claude/flutter-native-sheet`, on top of
lane 1 (`8af543a6`) merged with `origin/master` (`8cce0514`). **Host:**
Linux, the repository's pinned Flutter toolchain (3.47.2 / Dart 3.13.2),
`emulator-5554` (the maintainer's `Pixel_10a` AVD) for the hand-driven walk.
Every exit code below was read from a file, never a harness banner —
`<command> > <log> 2>&1; echo $? > <file>`.

## What landed

- **`VpayCheckoutSheet`** (`lib/src/sheet/checkout_sheet.dart`) — the widget
  every one of the 13 screen states renders as, plus `showVpayCheckoutSheet`
  (a large-detent, draggable `showModalBottomSheet`) and
  `showVpayCheckoutSheetRoute` (a full route). Inherits `ThemeData`; no
  fixed vpay palette, no `VpayCheckoutTheme`.
- **`SheetController`** (`lib/src/sheet/sheet_controller.dart`) — the impure
  half `checkout_screen.dart`'s reducer never had: session read, confirm,
  jittered poll (Lane 1's `poll_jitter.dart`, now actually wired into a
  controller for the first time), the redirect hand-off to the existing
  `VpayCheckoutPlatform`, and `dismiss()`'s D4-shaped mid-payment handling.
- **`BrowserClient.confirmPaymentIntent`** (`lib/src/browser_client.dart`) —
  new: the sheet drives a rail's confirm directly, form-encoded,
  bracket-nested `payment_method_data`, no rail code branched on to build
  it.
- **`PaymentIntent.redirectUrl`** (`lib/src/models.dart`) — Lane 1's model
  had no field for `next_action.redirect_to_url.url`; added, parsed the same
  way `controller.ts`'s `redirectUrlOf` does (absolute `http(s)` only).
- **i18n** (`lib/src/sheet/i18n.dart`) — the same 72 keys as
  `frontends/apps/checkout/src/i18n/{en,fr}.ts`, French default
  (`VpayLocale.fallback`).
- **"Remember this number"** (`lib/src/sheet/remember_msisdn.dart`) —
  `shared_preferences`-backed, opt-in, 90-day TTL enforced on read, written
  only after a real accepted confirm, no PIN, a "Forget" affordance. The
  shared-phone warning is inside `CheckboxListTile`'s own merged semantics
  label (Flutter's `title`+`subtitle` merge into one accessible name), not
  a tooltip.
- **Locale-aware money presentation** (`lib/src/sheet/money_format.dart`) —
  built on Lane 1's `money.dart` digit surgery; no float anywhere.
- **Focus management and the live region** — each screen heading is
  focused on transition; `Semantics(liveRegion: true)` is mounted at first
  build and never unmounted; `SemanticsService.sendAnnouncement` fires the
  transition text.
- **The example app** (`example/lib/main.dart`) — the Buy button now opens
  `showVpayCheckoutSheet` instead of `VpayCheckout.start`.

**Decisions recorded, not silently dropped:**

- `outcome.no_destination` (the hosted page's "nowhere to navigate" screen)
  has no native equivalent — a sheet always has a way back (dismissing it),
  so `VpayCheckoutSheet`'s outcome screen always renders the one button.
  Recorded in `checkout_sheet.dart`'s own doc comment and
  `docs/flows/mobile-checkout.md`.
- `frame.ts`/`origins.ts`/`csp.ts`/`entry.ts` (D4/D8 iframe refusal,
  `postMessage`) have no native analogue — see
  [ADR-0021](../../adr/0021-flutter-checkout-plugin.md)'s 2026-09-17
  addition and `docs/flows/mobile-checkout.md`.
- Cards are out of scope, structurally: `RailFieldKind.fromJson` decodes an
  unrecognised field type (`"card"` included) to `RailFieldKindUnknown`,
  which `rails.dart`'s `railChoices` already refused to render (Lane 1).
- The redirect entry screen's own "remember Orange Money on this device"
  checkbox renders but writes nothing — only the MSISDN half of page memory
  has a persistence layer. Named in `mobile-checkout.md`'s Status section.

## A real defect, found by this lane's own gate — not a unit test

Every confirm the sheet sent was refused by the real server:
`payment_method_data[type]` never arrived, so
`vpay-api`'s handler answered "A confirm needs the payment method to use,
sent as `payment_method_data[type]`.". Root cause:
`BrowserClient`'s form encoder percent-encoded the bracket syntax's own
structural characters (`payment_method_data%5Btype%5D`), and
`backends/crates/vpay-api/src/form.rs`'s parser splits a raw key on the
_literal_ `[` before decoding anything (its own module doc: "the split
happens before any decoding"), so the whole percent-encoded string decoded
to one flat key instead of a nested path.

Every assertion in `test/browser_client_test.dart` passed throughout,
because they read the request back through `Uri.splitQueryString`, which
decodes the whole key before an assertion ever sees it — exactly the "green
tests, broken product" shape this session's brief warns about. Found only
because the hand-driven walk on `emulator-5554`, against a freshly rebuilt
`vpay-demo` stack, actually tried to pay and the confirm came back refused.

**Fixed**: `BrowserClient._bracketKey` percent-encodes each path segment and
leaves the structural brackets literal. **Proven decisive**: reverted the
fix locally and confirmed the (also-fixed) test —
`test/browser_client_test.dart`'s `'POSTs form-encoded key, client_secret,
payment_method_data[type] and the nested field'` — fails against the
reverted code, asserting on the literal wire bytes rather than the
already-decoded map. Commit `349af376`.

## Gates, exit codes read from files

| Gate                                  | Command                                          | Exit                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| ------------------------------------- | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `flutter test`                        | `sdks/flutter/vpay_checkout_flutter`             | 0 — **258 passed / 0 skipped** (was 200/0 after Lane 1)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `dart analyze --fatal-infos .`        | same dir                                         | 0 — no issues                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `dart format --set-exit-if-changed .` | same dir                                         | 0 — 57 files, 0 changed                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `cargo xtask verify-links`            | repo root                                        | 0 — 1706 links, 396 files (this lane's own doc edits included)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `cargo xtask verify-sdk-parity`       | repo root                                        | 0 — 661 proving tests, 37 dated gaps, 39 rows                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `just fmt-check-web`                  | repo root                                        | **1 — pre-existing for `docs/sdks/parity.md` and two other files this lane also edited** (`docs/adr/0021-flutter-checkout-plugin.md`, `docs/status/mobile-flutter-plugin.md` — confirmed by stashing each and re-running: all three still fail with no edit of this lane's own applied). `docs/flows/mobile-checkout.md` and this page were newly non-conformant after this lane's own edits and are now `prettier --write`-clean, matching Lane 1's own new verification page (also clean) rather than the pre-existing, ungrandfathered files above. |
| `just test-flutter-e2e`               | repo root, against the rebuilt `vpay-demo` stack | 0 — 3 passed (mint, uniform-404, expired-session-refuses-confirm), real `cs_…`/`pi_…` ids, real webhook, shop order `paid`                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `just test-flutter-emulator`          | repo root, `VPAY_EMULATOR_SERIAL=emulator-5554`  | 0 — both suites (`checkout_dismiss_test.dart`, `checkout_external_browser_test.dart`) green on the real device                                                                                                                                                                                                                                                                                                                                                                                                                                         |

`just ci` was not run, per this lane's own brief.

**The `vpay-demo` stack had to be rebuilt.** It was running a pre-#186
image (started 2026-09-16 04:43, no `rails` key in a session read at all),
which made `just test-flutter-e2e` fail identically on the pre-Lane-2 commit
(`8cce0514`, confirmed in a scratch worktree) — a stale shared environment,
not a Lane 2 regression. `just demo-up` (which rebuilds and recreates only
the `vpay-demo` project's own containers) was re-run rather than torn down;
nothing outside that project was touched.

## The hand-driven walk — the real gate

Screenshots: `<scratchpad>/sheet-ui/{1-app,2-select-rail,3-msisdn,4-waiting,5-paid,6-back-in-app}.png`
(merchant app visible, dimmed, behind the sheet in 2–5, confirming the
"popup, not full-screen" presentation).

**Three cold launches, MTN, to `paid` in `examples/shop`'s own database**
(`orders.get`, read after the real signed webhook, never a value this app
computed):

| #   | Checkout session              | Payment intent                | Shop order                  | Shop status |
| --- | ----------------------------- | ----------------------------- | --------------------------- | ----------- |
| 1   | `cs_xdsgjxgnms125191q5fbt46j` | `pi_8ge830rszs2a7597m3ef4ntr` | `cmu51xbw4000k01mwdzfm8jf5` | `paid`      |
| 2   | `cs_55sfavjrss5v149zxjm19sb6` | `pi_fzjs43a3q14ns6bfa8fsnknk` | `cmu5202tw000m01mw8e2kej82` | `paid`      |
| 3   | `cs_hxmzrr3f1h39bea1fpsehs8e` | `pi_vfc05aw6157pd8247zt06sa1` | `cmu521wpi000o01mwegxccuaf` | `paid`      |

Each: `am force-stop` then relaunch (a genuine cold start, not a resumed
process), tap Buy → sheet rises over the merchant app → MTN Mobile Money →
`237671234567` (see "A stale test number" below) → Payer → waiting screen
→ `Paiement reçu` → `Retour vers Njangi Store` → back in the merchant app,
`Paid ✓ {intent id}`.

**One Orange (redirect) payment, through the hand-off and back:**
`cs_dd94vfqvb51kh5m28136h165` / `pi_y2bshj0sah3tn51gsbanv5g9` — Buy → Orange
Money → ready-redirect screen ("Vous reviendrez ici une fois le paiement
effectué.") → Payer → confirm → `CheckoutRedirecting` → the **same**
`VpayCheckoutPlatform` Custom Tab the browser cutover built opens the
WireMock Orange stub page → "Pay with this number" (`237600000000`, the
stub's own default) → the stub redirects to vpay's own hosted return page
(`Paiement reçu`, on `localhost:3080`) → closed the tab → back in the sheet,
already resolved (the sheet's own poll had also reached the outcome) →
`Paiement reçu` → `paid` in the shop's database.

**One dismissal mid-payment — never `canceled`:** same Orange flow,
confirmed through to `CheckoutRedirecting`, but the Custom Tab was
abandoned (hardware back) **before ever pressing Pay or Cancel on the
rail's own page** — genuinely mid-flight, not a race against an
already-settled intent (Orange's WireMock stub guarantees `PENDING` for a
bounded window from confirm regardless of payer interaction, so this is
deterministic, unlike MTN's near-instant settlement in this demo). Result:
the sheet returned to its **waiting** screen ("Consultez votre téléphone" —
D4's dismissal-then-poll, not a fabricated outcome) and stayed there for
the whole observed window; `examples/shop`'s own `orders.get` for
`cs_narecnxq71791f0k1ak7c7wj` / `pi_yvxcfgzkeh5mhb87vst7h3ry` read `unpaid`
at every check — never `canceled`, never `failed`. The identical property
is pinned deterministically (not timing-dependent) by
`test/sheet/sheet_controller_test.dart`'s `'mid-payment (after confirm,
still moving) resolves Pending — never canceled'` and `'before any confirm,
resolves Unresolved — never canceled'`.

**A stale test number, found along the way.** `examples/shop/README.md`'s
own MTN table documents `237600000000` as "pays, any number not listed
below does the same" — true against `/v1/checkout/sessions` (the JS SDK's
own `phonenumber` validation predates #186), but the _native sheet's_
confirm goes through `vpay-api`'s `phonenumber`-crate validation (#186), and
`237600000000` is not a real, assignable Cameroon mobile number under it —
confirmed directly with `curl`, independent of this SDK, before assuming a
client bug: `400 payment_method_data[mtn_momo][msisdn] must be a valid
phone number`. `237671234567` (a real MTN prefix, already used throughout
Lane 1's own fixtures) confirms cleanly. Not a defect in this lane; a
documentation staleness the walk surfaced, left for the maintainer to
decide whether `examples/shop/README.md` should change its own advertised
number.

## What was not done, honestly

- iOS and macOS were not touched by this lane — Android only, on
  `emulator-5554`, per the environment section of this session's brief.
- The redirect hand-off's own stop-URL matching is still the same
  unverified-on-every-platform deep-link signal the browser cutover
  documented; every Orange walk above ended in `dismissed`, never
  `stopUrlReached`, and D4's poll is what made that correct anyway.
- The "remember Orange Money" checkbox on the redirect entry screen renders
  but persists nothing — only MSISDN has a store.
- `just ci`, `just test-storybook` and `cargo xtask verify-citations` were
  not run — out of this lane's brief.
- A matching PR against `vaam-apps/vpay-skills` was not opened — outside
  this worktree's remit as briefed.
