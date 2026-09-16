# Issue #189 lane 1 — the server-driven rail spec and the screen/return machines

**Date:** 2026-09-16. **Branch:** `claude/flutter-native-sheet`, base
`0d978282`. **Host:** Linux, the repository's pinned Flutter toolchain.
**Scope:** pure-Dart core only — no widgets, no i18n catalogue, no
presentation. See [issue #189](https://github.com/vaam-apps/vpay/issues/189)
and `docs/flows/mobile-checkout.md`'s Status section for what this lane does
and does not cover.

Every exit code below was read from a file, never from a harness banner —
`<command> > <log> 2>&1; echo $? > <file>`.

## What landed

- **The #186 rail spec, parsed.** `lib/src/models.dart` gained `RailSpec`,
  `RailField`, `RailFieldKind` (`RailFieldKindPhone`/`RailFieldKindUnknown`),
  `RailDisplayName`, `RailFlow` and `CheckoutSessionPaymentStatus`;
  `CheckoutSession.rails` and `.paymentStatus` are parsed by
  `CheckoutSession.fromJson`, confirmed against the exact wire shape in
  `backends/crates/vpay-api/src/model.rs`'s `RailSpec`/`RailDisplayName` and
  `vpay-provider`'s `PayerField`/`PayerFieldKind`, not only the issue's own
  JSON example.
- **The payment screen machine** — `lib/src/sheet/checkout_screen.dart`, the
  Dart port of `frontends/apps/checkout/src/lib/machine.ts`: the 12 named
  `CheckoutState` variants (13 screens counting `CheckoutRefused`'s two
  reasons separately, matching the issue's own count), the full transition
  table, and `stateForContext`'s session/intent precedence including the
  `expired` + `payment_status: failed` nuance.
- **The return machine** — `lib/src/sheet/return_screen.dart`, the Dart
  port of `return.ts`: a separate reducer with no `confirm`-shaped
  transition. `ReturnPaymentIntent` structurally has no `client_secret`
  member, so nothing reachable from it can build a confirm request even by
  mistake (D6).
- **The shared terminal-rule implementation** — `lib/src/sheet/outcome.dart`
  (`outcomeFor`): both screen machines read the same function, so they
  cannot independently drift on "bare `requires_payment_method` is not
  terminal."
- **`rails.ts`, rebuilt structural rather than per-code** —
  `lib/src/sheet/rails.dart`: `railChoices` decides support from
  `RailSpec.flow` and each field's `RailFieldKind`, never from `.code`. A
  card-shaped field (`RailFieldKindUnknown`) makes a rail unsupported (D9)
  rather than rendering it — the mechanism that keeps a card field out
  without a single `if (field.type == "card")` anywhere.
- **`money.ts`'s no-float half** — `lib/src/sheet/money.dart`:
  `toDecimalString`'s digit surgery and the six-currency zero-decimal table.
  Locale-aware formatting (`Intl`/`formatAmount`) is **not** ported — it is
  presentation, out of scope for this lane; see the file's own doc comment.
- **`msisdn.ts`** — `lib/src/sheet/msisdn.dart`: `normalizeCameroonMsisdn`/
  `formatCameroonMsisdn`, NBSP (U+00A0) and narrow-NBSP (U+202F) included
  among accepted separators.
- **`failures.ts`'s `providerReason` half** — `lib/src/sheet/failures.dart`:
  300-char cap, control-character stripping, whitespace collapse.
  `FAILURE_MESSAGES`/`failureMessage` (the code → i18n-key table) is **not**
  ported — it needs the i18n catalogue, out of scope for this lane; the
  screen machine carries the raw `FailureCode` through unchanged, exactly as
  `machine.ts`'s own `Outcome.failure` does.
- **A jittered poll-delay primitive** — `lib/src/sheet/poll_jitter.dart`:
  `client.ts:57-58,682-694`'s `[0.75, 1.25] × interval` ladder, with an
  injectable `JitterSource` so it is deterministic in tests. Not yet wired
  into a controller — the impure poll loop that would use it is a later
  lane's `SheetController`.
- **A mechanical no-rail-code-branching test** —
  `test/sheet/no_rail_code_branching_test.dart`: scans every `.dart` file
  under `lib/` for a literal rail code or a structural "branch on a code"
  pattern and fails if it finds one, rather than relying on review alone.

## Gates

- `flutter test`: **200 passed, 0 skipped, 0 failed.** Exit 0. (Before this
  lane: 80 passed / 0 skipped, per this page's own "Browser, not WebView"
  section.)
- `dart analyze --fatal-infos`: **no issues found.** Exit 0.
- `dart format --set-exit-if-changed .`: **0 files would change.** Exit 0.
- `cargo run -p xtask -- verify-sdk-parity`: **ok — 632 proving test(s)
  named in `docs/sdks/parity.md` all exist, 37 dated gap(s), 35 SDK
  method(s) enumerated across 39 row(s).** Exit 0. Ten new capability rows
  were added for what this lane actually proves; none of the new rows
  reused the split, multi-line-concatenated titles `dart format` produces
  for a few longer test descriptions, since this repository's parity
  checker reads a Dart `test('…')` call's first string literal only, not an
  implicit adjacent-literal concatenation — citing one of those would have
  named a test that (from the checker's point of view) does not exist.
- `cargo run -p xtask -- verify-links`: **ok — 1697 repository link(s) in
  394 tracked markdown file(s) resolve to a tracked path.** Exit 0.
- `just fmt-check-web` (`pnpm exec prettier --check .`): **exit 1, and this
  is a PRE-EXISTING failure, not one this lane introduced.** The same 28
  files (diffed byte-for-byte against a `git stash`-free baseline run
  before this lane touched anything) are flagged before and after this
  lane's change, `docs/sdks/parity.md` among them — it was already
  prettier-non-conformant on `0d978282`. This lane's own new content
  (`docs/sdks/parity.md`'s ten new rows) does not add a new file to the
  violation list; the file was already on it. `CLAUDE.md`'s own line about
  this gate ("it has bitten four times in this project") is corroborated,
  not contradicted, by this run. Fixing the other 27 unrelated files is
  outside this lane's scope and was not attempted.

## Three mutation proofs, run and reverted

Each mutation was applied to the real source, the named test(s) were run
against the mutated source, the failure was confirmed, and the source was
reverted (checked by re-running `flutter test` clean afterward).

1. **Money digit surgery → `minor / pow(10, exponent)`**
   (`lib/src/sheet/money.dart`'s `toDecimalString`, replaced with a float
   division through a hand-rolled `_pow` and `.toString()`).
   `flutter test test/sheet/money_test.dart`: **7 of 9 tests failed, exit
   1** — including the file's own `MUTATION PROOF` test (expected
   `90071992547409.93`, got `90071992547409.92`, a real one-cent precision
   loss at that magnitude) and, incidentally, nearly every other assertion
   in the file, since a float's `.toString()` never produces the fixed
   `"50.00"`-shaped output the tests require either.
2. **Bare `requires_payment_method` made terminal**
   (`lib/src/sheet/outcome.dart`'s `outcomeFor`, dropped the
   `lastPaymentError != null` guard).
   `flutter test test/sheet/checkout_screen_test.dart test/sheet/return_screen_test.dart`:
   **12 of 57 tests failed, exit 1** —
   including both files' own `MUTATION PROOF` tests and, as a side effect,
   every test that reaches a non-terminal `requires_payment_method` intent
   through `stateForContext`/`reduceCheckoutScreen`/`stateForReturn`,
   because the mutated function now reports every one of them as
   `outcome:failed`.
3. **A rail-code branch added** (`lib/src/sheet/rails.dart`'s
   `railChoices`, `if (spec.code == 'mtn_momo') { … }` inserted before the
   loop's final `supported.add`).
   `flutter test test/sheet/no_rail_code_branching_test.dart`: **1 of 2
   tests failed, exit 1** — `lib/src/sheet/rails.dart contains the literal
"mtn_momo"`.

## What this lane did not do

Everything the parent issue's "bar" section and "Also do not lose" list
name that needs a widget, a locale catalogue, or a presentation decision:
the sheet itself, i18n (French default, ~71 keys), the "remember this
number" feature (IndexedDB-equivalent storage, 90-day TTL, the shared-phone
warning), the test-mode banner, focus management/`Semantics`, the
`redirect`-rail hand-off back into the sheet, and wiring the jittered-poll
primitive into an actual controller. D9 iframe/`postMessage` concepts
(`frame.ts`/`origins.ts`/`csp.ts`) remain out of scope by design — they are
web-platform concepts with no native analogue, per the issue's own text —
and are not addressed by an ADR in this lane; that is still owed.
`docs/flows/mobile-checkout.md`'s own Status section was updated with a
narrow, dated note pointing here, not rewritten.
