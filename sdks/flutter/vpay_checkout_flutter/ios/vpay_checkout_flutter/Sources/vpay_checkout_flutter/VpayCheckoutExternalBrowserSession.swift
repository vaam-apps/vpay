// D8's iOS external-browser window, wired 2026-09-14:
// `SFSafariViewController`, not `ASWebAuthenticationSession` — design doc
// D8 explains why: with no custom URL scheme (`checked_forward_url`
// accepts only `http(s)`, and a scheme is refused by design — schemes are
// first-come-first-served on Android and this plugin does not add one on
// any platform for consistency), `ASWebAuthenticationSession`'s
// `callbackURLScheme` would never fire below iOS 17.4, and it costs a
// system "… wants to use … to sign in" consent alert that is the wrong
// sentence on a payment. `SFSafariViewController` needs no callback at
// all: it is presented in-process, like a modal view controller, and its
// delegate tells us the moment the payer dismisses it.
//
// **Tier 1 (iOS 17.4+ Associated Domains, `ASWebAuthenticationSession`'s
// https callback) is NOT implemented here.** It needs a merchant-hosted
// `apple-app-site-association` file this repository cannot deploy or
// prove against — see `docs/sdks/parity.md`'s dated gap. This class only
// implements D8's tier 0, which the design doc states explicitly is
// correctness-complete on its own (D1: the outcome always comes from
// polling the payment intent, never from the window).
//
// Unlike `VpayCheckoutViewController`'s `WKWebView`, `SFSafariViewController`
// exposes no navigation delegate at all — Apple's own isolation, for
// exactly the reason D8 wants an external-browser mode to begin with. So
// there is no stop-URL interception here: the only signal is the payer
// tapping "Done", reported as `CheckoutWindowOutcome.dismissed`. D1/D4
// make this correctness-complete: `checkout_controller.dart` polls the
// real payment intent before answering a dismissal, so a payer who paid
// and then tapped Done still resolves `succeeded`, not `canceled`.
//
// **Compiled by nobody** (this repository has no macOS/iOS toolchain — see
// `docs/plans/2026-09-13-flutter-plugin-brief.md`, "What this host can
// actually verify"). Reviewed by reading only.
import SafariServices
import UIKit

final class VpayCheckoutExternalBrowserSession: NSObject {
  private let safari: SFSafariViewController
  private var reported = false

  /// Fired exactly once, mirroring `VpayCheckoutFlutterApi.onWindowEvent`'s
  /// own "exactly once per show" contract.
  var onEvent: ((CheckoutWindowEvent) -> Void)?

  init(url: URL) {
    safari = SFSafariViewController(url: url)
    super.init()
    safari.modalPresentationStyle = .pageSheet
    safari.delegate = self
  }

  func present(from presenter: UIViewController) {
    presenter.present(safari, animated: true)
  }

  /// Called by `VpayCheckoutFlutterPlugin` when `dismiss()` is invoked from
  /// Dart. A no-op the second time (`show` fires its event exactly once).
  func dismiss() {
    reportDismissedIfNeeded()
    safari.presentingViewController?.dismiss(animated: true)
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

extension VpayCheckoutExternalBrowserSession: SFSafariViewControllerDelegate {
  func safariViewControllerDidFinish(_ controller: SFSafariViewController) {
    // The payer tapped "Done" — the only dismissal signal this class has
    // (see this file's header).
    reportDismissedIfNeeded()
  }
}
