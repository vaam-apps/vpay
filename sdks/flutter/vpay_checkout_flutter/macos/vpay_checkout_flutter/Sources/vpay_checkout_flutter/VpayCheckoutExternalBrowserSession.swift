// The macOS host's ONE window (design doc D5, revised 2026-09-16; D8) — the
// maintainer's own call, recorded here rather than picked silently: **there
// is no `SFSafariViewController` on macOS** (it is a UIKit type, iOS/iPadOS
// only) and no Custom-Tabs equivalent either. The only "external browser" a
// Mac has is the payer's own default browser, a separate application. So
// this opens `NSWorkspace.shared.open(url)` and nothing else — no window,
// sheet or view controller of this app's own hosts the payment page at all.
//
// This used to be one of two hosts (the other, `VpayCheckoutViewController`,
// wrapped a `WKWebView` presented as a sheet). D5's 2026-09-16 revision
// deleted that class entirely, for the same reason iOS's in-app WKWebView
// host was deleted: rendering vpay's payment form inside the merchant
// app's own process put `evaluateJavascript`, the cookie store and a
// navigation delegate all within reach of a compromised merchant app.
//
// # The honest limit of this design: NSWorkspace.open has NO return signal
//
// `NSWorkspace.shared.open(url)` hands the URL to the default browser and
// returns immediately — it does not open a window this app owns, does not
// return a handle to the browser's window, and does not call back when
// that window closes. There is no AppKit or system API on macOS that
// reports "the user closed a window belonging to a DIFFERENT application."
// Concretely, that means this class has exactly two ways to ever fire
// `onEvent`, and no others:
//
// 1. An incoming **Universal Link** matches one of the request's
//    `stopUrls` (D2) — `VpayCheckoutFlutterPlugin.handleUserActivity(_:)`
//    calls `reportStopUrlReached(url:)` below. Wired but UNVERIFIED (see
//    that method's own header) — this repository has no HTTPS origin
//    serving `apple-app-site-association` to prove it against, and macOS
//    Flutter plugins have no automatic forwarding for this at all (see
//    `VpayCheckoutFlutterPlugin.swift`'s header).
// 2. Dart calls `VpayCheckoutHostApi.dismiss()` explicitly —
//    `VpayCheckoutFlutterPlugin.dismiss()` calls `dismiss()` below, which
//    reports `.dismissed`. This is NOT this class observing that the
//    browser window closed (it has no way to observe that); it is this
//    class honoring an explicit instruction from Dart to end the pending
//    `show()` call. `VpayCheckoutFlutterApi.onWindowEvent`'s contract is
//    "fired exactly once per show," and Dart's own `dismiss()` future would
//    hang forever if this class refused to resolve it just because it
//    cannot verify what the browser is actually doing.
//
// **If neither of those happens, `onEvent` never fires.** A payer who pays
// in the browser, closes the browser tab, and does nothing else produces
// NO signal to this app at all — not a delayed one, not an inferred one,
// none. An earlier version of this class filled that gap by treating
// `NSApplication.didBecomeActiveNotification` (this app regaining focus) as
// evidence of a dismissal. That was removed: reactivation only means the
// payer switched back to this app — by Cmd-Tab, by clicking the Dock icon,
// because the OS raised it for an unrelated reason — and says nothing about
// whether the browser window is still open. Reporting `.dismissed` off
// that notification was this codebase's cardinal sin
// (`CLAUDE.md`, "The failure mode to avoid"): a plausible-looking event
// this class never actually observed.
//
// # What this means for D4's poll in practice, on macOS specifically
//
// D1/D4 (`docs/adr/0021-flutter-checkout-plugin.md`) make every OTHER
// host's dismissal correctness-complete: `checkout_controller.dart` polls
// the real payment intent before answering a dismissal, so a payer who
// paid and then closed the window still resolves `succeeded`, not
// `canceled`. That machinery is unchanged here and still correct WHEN it
// runs. The macOS-specific gap is upstream of it: the poll only ever
// starts once `onWindowEvent` fires, and on macOS that requires either a
// Universal Link this repository cannot verify, or the merchant's own
// Flutter app code calling `dismiss()` on some signal of its own (a
// "check payment status" button, a timeout, polling its own backend
// independently of this plugin). Nothing in this plugin can currently give
// a macOS app "the browser closed" on its own. That is stated as a real,
// current gap — not solved by this pass — rather than hidden behind a
// synthetic event.
import AppKit

final class VpayCheckoutExternalBrowserSession: NSObject {
  private var reported = false

  /// Fired exactly once, mirroring `VpayCheckoutFlutterApi.onWindowEvent`'s
  /// own "exactly once per show" contract — see this file's header for the
  /// exactly two ways that can happen on macOS.
  var onEvent: ((CheckoutWindowEvent) -> Void)?

  /// Opens `url` in the payer's default browser. Returns immediately;
  /// nothing about this call itself signals anything back (see this file's
  /// header).
  func open(url: URL) {
    NSWorkspace.shared.open(url)
  }

  /// Called by `VpayCheckoutFlutterPlugin.handleUserActivity(_:)` when an
  /// incoming Universal Link matched one of `ShowCheckoutRequest.stopUrls`
  /// (D2). Wired but UNVERIFIED — see this file's header.
  func reportStopUrlReached(url: URL) {
    report(CheckoutWindowEvent(outcome: .stopUrlReached, reachedUrl: url.absoluteString))
  }

  /// Called by `VpayCheckoutFlutterPlugin.dismiss()` when Dart explicitly
  /// asks to close the session. Does NOT close, and cannot close, the
  /// browser window itself — `NSWorkspace.open` gives no handle to do that
  /// (see this file's header) — this only ends this plugin's own
  /// bookkeeping and resolves the pending `onWindowEvent` as `.dismissed`,
  /// because Dart asked it to, not because anything was observed.
  func dismiss() {
    reportDismissedIfNeeded()
  }

  func reportDismissedIfNeeded() {
    report(CheckoutWindowEvent(outcome: .dismissed, reachedUrl: nil))
  }

  private func report(_ event: CheckoutWindowEvent) {
    guard !reported else { return }
    reported = true
    onEvent?(event)
  }
}
