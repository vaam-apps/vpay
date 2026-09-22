# ADR-0023: A Tauri v2 checkout plugin — one state machine, three hosts

- **Status:** Proposed — needs maintainer acceptance. Every decision below
  (T1–T7) was taken by the implementing agent, not by the maintainer; the
  maintainer asked for the surface and for the fan-out, and nothing else
  here has been put to them. § "Left to the maintainer" carries four
  questions this ADR deliberately does not answer.
- **Date:** 2026-09-22
- **Deciders:** the implementing agent (T1–T7, 2026-09-22), pending the vpay
  maintainers
- **Extends:** [ADR-0021](0021-flutter-checkout-plugin.md) — the payer-surface
  decision, its D1–D9 and D-M1–D-M6, **inherited unchanged**. This ADR
  revisits none of them; it records only what is about Tauri. Design record:
  [`docs/plans/2026-09-22-tauri-plugin.md`](../plans/2026-09-22-tauri-plugin.md).
  Process: [`docs/flows/tauri-checkout.md`](../flows/tauri-checkout.md).
- **Number checked at branch time, not assumed:** `ls docs/adr` on
  `claude/tauri-v2-sdk-multiplatform-ca900b` (base `master` `7134ecb`)
  returns `0001`…`0022`, **22 files for 22 numbers — no gap and no
  duplicate** (the duplicate `0018` ADR-0021 and ADR-0022 both record was
  resolved by [#172](https://github.com/vaam-apps/vpay/pull/172) on
  2026-09-16). `gh pr list --state open --json number,title,files` on
  2026-09-22 returns **exactly one** open pull request — **#236**, "chore:
  release master" — whose diff touches twelve files and **none** under
  `docs/adr/`. So `0023` is the next number free of both the tree and every
  open pull request's diff.

## Context

The maintainer asked, verbatim, on 2026-09-22: _"Please create a SDK for
andoid+ios+web for tauri v2"_ [sic].

Three facts already in this repository make most of the answer a foregone
conclusion rather than a fresh design, and they are why this ADR is short:

1. **ADR-0021 already decided what a vpay payer surface is.** It holds a
   publishable key and one session's credentials, never a merchant token; it
   opens vpay's hosted page in the payer's **own browser**; and it reads the
   outcome by polling `GET /v1/browser/payment_intents/{id}`, never off a
   URL. A second payer surface that answered any of those differently would
   be a divergence, not a design.
2. **The browser client already exists.** `@vaam-apps/vpay-stripe-js` speaks
   the two `/v1/browser` reads this surface needs.
3. **The hosted page needs no change to run in a Custom Tab, an
   `SFSafariViewController` or a desktop browser tab.** It already runs
   top-level with no peer.

What was genuinely open is everything in the second column: where a Tauri
plugin's state machine lives when one of its three targets has no Rust, what
a plugin crate does to this repository's Cargo graph, and what Tauri v2.11.6
can and cannot actually do on iOS.

## Decision

Ship `tauri-plugin-vpay-checkout` (the Rust crate, `publish = false`) and
`@vaam-apps/vpay-tauri-checkout` (the guest-JS npm package) from
`sdks/tauri/tauri-plugin-vpay-checkout/`, with the following seven decisions.
The design record argues each at length; here each is one sentence and its
one reason.

| #      | Decision                                                                                                                               | Because                                                                                                                                                            |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **T1** | The state machine — parse, pre-flight, stop URLs, poll, terminal rule — is guest-JS; the Rust, Kotlin and Swift hosts decide nothing.  | The maintainer asked for **web**, and on the web there is no Rust; one implementation of D1/D4 tested on Node beats two implementations or a wasm build.           |
| **T2** | The crate carries its own empty `[workspace]` table and is a member of no workspace.                                                   | `tauri`'s graph (wry, tao, objc2, webkit2gtk) must not enter `Cargo.lock`, `cargo deny`, `verify-no-mocks`'s `cargo metadata`, or `cargo nextest run --workspace`. |
| **T3** | The two reads reuse `@vaam-apps/vpay-stripe-js` rather than porting `browser_client.dart`.                                             | One wire, one client: a divergence between two TypeScript clients on the same two routes would be invisible until a payer hit it.                                  |
| **T4** | The desktop host opens the default browser via `open::that_detached` and has **no** dismissal signal; `dismiss()` is the only trigger. | The Flutter macOS host already shipped a focus-regained signal once and removed it on 2026-09-16; "the app regained focus" is not "the payer finished".            |
| **T5** | Outside Tauri the same package is the popup host, accepting `vpay:complete` only from its own origin **and** its own popup `Window`.   | "web" is a third host, not a second package; and origin alone let a same-origin window spoof completion, which the Flutter web suite found on 2026-09-15.          |
| **T6** | iOS cannot report `stopUrlReached`; every iOS checkout ends `dismissed`, and the matching code stays, unreachable and labelled.        | tauri-v2.11.6's Swift `Plugin` exposes no app-delegate hook and its own `deep-link` plugin ships no `ios/` at all, handling iOS in Rust via `RunEvent::Opened`.    |
| **T7** | None of the Rust/Kotlin/Swift halves is in `just ci` or `just verify`; the TypeScript half already is, through `pnpm -r`.              | The CI image has no webkit2gtk, no Android SDK/NDK and no Xcode; adding them is a runner-image change, i.e. the maintainer's call — D-M3's reasoning, second time. |

The wire between guest-JS and every host is fixed by
[`docs/plans/2026-09-22-tauri-plugin-brief.md`](../plans/2026-09-22-tauri-plugin-brief.md)
and is not restated here. Its one load-bearing property: a host's entire
vocabulary is `{ outcome: "dismissed" | "stopUrlReached", reachedUrl }`, sent
**exactly once per `show`**. There is no `succeeded`, `canceled` or `failed`
outcome anywhere in it, because no window may decide what a payment did
(D1).

## Alternatives considered

**A Tauri `WebviewWindow` for the checkout page.** Rejected by D5 as revised
2026-09-16, which is inherited rather than re-argued: rendering vpay's
payment form inside the merchant app's own process puts `evaluateJavascript`,
the cookie store and a navigation delegate within that app's reach — i.e.
lets a compromised merchant app read the payer's PAN and OTP undetectably.
The browser's process cannot be inspected that way and the payer gets a real
URL bar. The cost is stated rather than hidden: no host can observe a
navigation any more, so a stop URL only ever arrives as a deep link, which is
why T6's gap is a latency gap rather than a correctness one.

**A Rust-side state machine, with guest-JS as thin bindings.** Rejected
because the web has no Rust. The options were a wasm build of the machine
shipped to browsers (a second artefact and a second set of failure modes) or
two implementations of D1/D4 — the exact divergence ADR-0015 exists to catch,
on the two rules a payer's money depends on.

**Depending on `tauri-plugin-opener` for the desktop browser launch.**
Rejected in favour of the `open` crate directly. `tauri-plugin-opener` is
itself a thin wrapper over `open`, so the dependency buys nothing this crate
needs and costs something specific: a merchant registering
`tauri-plugin-vpay-checkout` would transitively acquire a second plugin, its
own permission namespace and its own capability entries, for a payments SDK
whose whole permission surface is deliberately two commands. Reversible, and
named in "Left to the maintainer" because it is a taste call as much as a
technical one.

**`addPluginListener` events instead of a per-`show` `Channel`.** Rejected.
`addPluginListener` gives a plugin-wide event stream, which makes
"exactly one event per `show`" something guest-JS must enforce by
correlation rather than something the wire gives it. A
`tauri::ipc::Channel<CheckoutWindowEvent>` passed as the `onEvent` argument
of the `show` call is bound to that call, so the event cannot arrive on the
wrong checkout and a second one is ignorable at the host boundary. It also
made `show`'s arguments flat rather than one `request` object, because
`Channel` implements `CommandArg` and not `Deserialize` — which is recorded
in `src/commands.rs`'s own module header, with the upstream line citations,
rather than here.

## Consequences

**A fourth toolchain family enters a repository that had three.** Until
today this tree needed Rust, Node/pnpm and (for one payer surface) Flutter.
It now also needs Kotlin/Gradle and Swift/SwiftPM **reached through Tauri's
own CLI** — `cargo tauri android build`, `cargo tauri ios build` — which is
not the same tooling the Flutter plugin's Kotlin and Swift go through. Nobody
has to install any of it to work on the rest of the repository, and that is
T2 and T7 doing their job; but anyone changing `android/` or `ios/` here
needs a toolchain no CI job has.

**Nothing in `.github/workflows/ci.yml` changed, and nothing needed to.**
The `web` job's `paths-filter` already names `**/*.ts`, `**/*.json` and
`**/*.md`, so a change under `sdks/tauri/` fires it; `pnpm-workspace.yaml`
gained `"sdks/tauri/*"`, so `just lint-web` (`pnpm -r typecheck`,
`pnpm -r lint`), `just test-web` (`pnpm -r test`) and `just fmt-check-web`
(`prettier --check .`) all reach the new package with no recipe change. What
fires no job is the Rust, Kotlin and Swift.

**Four new `just` recipes, none of them a gate.**
`just test-tauri-rust`, `just clippy-tauri-rust`, `just test-tauri-js` and
`just check-tauri-mobile` exist so the commands are written down once and
correctly; `just ci` and `just verify` are untouched, and the justfile's own
gate tally is unchanged at fifteen. The honest form of "later" is the dated
⛔ rows in [`../sdks/parity.md`](../sdks/parity.md), which is where a reader
who trusts a green build will actually look.

**`release-please` gains two more files to bump.**
`sdks/tauri/tauri-plugin-vpay-checkout/Cargo.toml` (a `generic` entry, whose
`version` line carries `# x-release-please-version`) and its `package.json`
(a `json` entry at `$.version`). Both are `{"type": …}` objects and neither
is a bare string, for the reason #203/#204 wrote down: a bare string gets an
updater inferred from the file extension, and that reparses and re-serialises
the document. `cargo xtask verify-versions` reads the config and now counts
two more references that must all say the same version.

**The iOS 15.0 floor is a declaration and a guard, not a build constraint.**
`ios/Package.swift` declares `platforms: [.iOS(.v15)]`, and **nothing in the
plugin's own build reads it**: `cargo check --target aarch64-apple-ios` goes
through `tauri_utils::build::link_apple_library` →
`swift_rs::SwiftLinker::with_ios(…)`, which honours
`IPHONEOS_DEPLOYMENT_TARGET` and defaults to `"13.0"`. The Swift therefore
has to keep compiling at `-target arm64-apple-ios13.0`, and the one
iOS-15-only API it uses — `sheetPresentationController`'s detents, which the
`.large()` detent decision needs — sits behind an `if #available(iOS 15.0,
*)` guard. Two things follow for anyone editing `ios/`: raising the floor in
`Package.swift` buys nothing on its own, and `cargo check --target
aarch64-apple-ios` exiting 101 with a Swift availability error means that
guard regressed, not that the environment is wrong. (It did exit 101, before
the guard existed.)

**The generated permission files are tracked.**
`permissions/autogenerated/**` and `permissions/schemas/schema.json` are
written by `tauri_plugin::Builder` on every `cargo build` of the crate, and
they are committed the way every plugin in `tauri-apps/plugins-workspace`
commits them — a consuming app's build script discovers a plugin's
permissions from them. They are prettier-ignored (`.prettierignore`'s own
entry says why), because a formatted copy is un-formatted again by the next
`cargo build`.

**A capability a merchant must grant, or nothing works and the failure is
confusing.** `vpay-checkout:default` grants `allow-show` and `allow-dismiss`
and nothing else. Without it Tauri's capability system denies both commands
and `start` answers `unresolved` with `platform_window_failed` — which is the
correct answer and an opaque one; the package README says so at the point a
merchant would hit it.

## Left to the maintainer

None of these is decided here, and none of them blocks the code as it stands.

1. **Publishing.** The crate is `publish = false` with a stated reason (the
   merchant-facing artefact is the npm package; the Rust half is consumed by
   path or by git), and the npm package is publish-ready but published by
   nothing. Whether either goes to a registry, and under what release
   cadence, is a distribution decision.
2. **Tauri toolchains in the CI image.** Adding `libwebkit2gtk-4.1-dev`
   alone would let `just clippy-tauri-rust` and `just test-tauri-rust` run in
   CI; Android and iOS need far more. T7 is a consequence of not having them,
   not an argument against ever having them.
3. **`open` versus `tauri-plugin-opener`.** Argued above; genuinely a taste
   call about whether a payments plugin should pull in another plugin.
4. **Whether a desktop host should exist at all.** A desktop checkout with no
   dismissal signal (T4) always waits out D4's 5-second budget and answers
   `pending` unless the application calls `dismiss()`. That is correct and it
   is a poor experience. The alternatives — refusing on desktop, or requiring
   the merchant to wire `tauri-plugin-deep-link` and call `dismiss()`
   themselves — are both defensible and neither was chosen unilaterally.
