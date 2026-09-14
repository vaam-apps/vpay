// D8's macOS external-browser window, wired 2026-09-14 — the maintainer's
// call, recorded here rather than picked silently: **there is no
// `SFSafariViewController` on macOS** (it is a UIKit type, iOS/iPadOS
// only) and no Custom-Tabs equivalent either. The only "external browser"
// a Mac has is the payer's own default browser, a separate application —
// closer in shape to Android's Custom Tabs (another app's window, D8's
// tier 0) than to iOS's in-process `SFSafariViewController`.
//
// So this mirrors Android's own tier-0 design exactly: `NSWorkspace.shared
// .open(url)` hands the URL to the default browser, and this class watches
// for `NSApplication.didBecomeActiveNotification` — the moment THIS app
// (not the browser) regains focus — as the one return signal, reported as
// `CheckoutWindowOutcome.dismissed`, never `stopUrlReached` (there is no
// navigation here to watch a stop URL against). D1/D4 make this
// correctness-complete: `checkout_controller.dart` polls the real payment
// intent before answering a dismissal, so a payer who paid and then
// switched back to this app still resolves `succeeded`, not `canceled`.
//
// No custom URL scheme anywhere (D8), consistent with every other platform
// in this plugin.
//
// **Compiled by nobody** (this repository has no macOS/iOS toolchain — see
// `docs/plans/2026-09-13-flutter-plugin-brief.md`, "What this host can
// actually verify"). Reviewed by reading only.
import AppKit

final class VpayCheckoutExternalBrowserSession: NSObject {
  private var reported = false
  private var observer: NSObjectProtocol?

  /// Fired exactly once, mirroring `VpayCheckoutFlutterApi.onWindowEvent`'s
  /// own "exactly once per show" contract.
  var onEvent: ((CheckoutWindowEvent) -> Void)?

  /// Opens [url] in the payer's default browser and starts watching for
  /// this app's own reactivation. The observer is registered BEFORE
  /// `NSWorkspace.shared.open` is called, so a reactivation racing the
  /// open call itself is never missed.
  func open(url: URL) {
    observer = NotificationCenter.default.addObserver(
      forName: NSApplication.didBecomeActiveNotification,
      object: nil,
      queue: .main
    ) { [weak self] _ in
      self?.reportDismissedIfNeeded()
    }
    NSWorkspace.shared.open(url)
  }

  /// Best-effort only, and in practice a no-op: the default browser is a
  /// separate application, and there is no API to close its window from
  /// here. This exists only so `VpayCheckoutFlutterPlugin.dismiss` has
  /// something to call; it does not stop watching for this app's own
  /// reactivation, which remains the only real return signal.
  func dismiss() {
    // Deliberately empty — see the doc comment above.
  }

  func reportDismissedIfNeeded() {
    report(CheckoutWindowEvent(outcome: .dismissed, reachedUrl: nil))
  }

  private func report(_ event: CheckoutWindowEvent) {
    guard !reported else { return }
    reported = true
    if let observer {
      NotificationCenter.default.removeObserver(observer)
    }
    observer = nil
    onEvent?(event)
  }
}
