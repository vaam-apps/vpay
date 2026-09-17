// The iOS host's ONE window (design doc D5, revised 2026-09-16 — "the payer's
// browser, not an in-app WKWebView"; D8): `SFSafariViewController`, not
// `ASWebAuthenticationSession` — design doc D8 explains why: with no custom
// URL scheme (`checked_forward_url` accepts only `http(s)`, and a scheme is
// refused by design — schemes are first-come-first-served on Android and
// this plugin does not add one on any platform for consistency),
// `ASWebAuthenticationSession`'s `callbackURLScheme` would never fire below
// iOS 17.4, and it costs a system "… wants to use … to sign in" consent
// alert that is the wrong sentence on a payment. `SFSafariViewController`
// needs no callback at all: it is presented in-process, like a modal view
// controller, and its delegate tells us the moment the payer taps "Done".
//
// This used to be one of two hosts (the other, `VpayCheckoutViewController`,
// wrapped an in-app `WKWebView`). D5's 2026-09-16 revision deleted that
// class entirely: rendering vpay's payment form inside the merchant app's
// own process put `evaluateJavascript`, the cookie store and a navigation
// delegate all within reach of a compromised merchant app — i.e. somewhere
// a payer's PAN and OTP could be read without the payer being able to tell.
// The browser's own process cannot be inspected that way, and the payer
// gets a real URL bar. The cost, stated rather than hidden: no host on any
// platform can see a navigation any more (Apple's own isolation — a
// `SFSafariViewController` exposes no navigation delegate at all, for
// exactly the same reason this plugin wants one), so a stop URL only ever
// arrives as an incoming **Universal Link**, handled by
// `VpayCheckoutFlutterPlugin.application(_:continue:restorationHandler:)`
// and reported here via `reportStopUrlReached(url:)`. The only other signal
// this class has of its own is the payer tapping "Done", reported as
// `CheckoutWindowOutcome.dismissed`. D1/D4 make both cases
// correctness-complete: `checkout_controller.dart` polls the real payment
// intent before answering either one, so a payer who paid and then tapped
// Done (no Universal Link ever arriving) still resolves `succeeded`, not
// `canceled`.
//
// **Universal Link intake is wired but UNVERIFIED as of 2026-09-16.** A
// verified Universal Link needs an HTTPS origin serving
// `apple-app-site-association` for the merchant's own `success_url`/
// `cancel_url` host, an associated-domains entitlement on a signed app, and
// a real device or a simulator with that entitlement provisioned — none of
// which this repository can deploy or prove against. The matching code
// below has been read, not driven by an actual incoming Universal Link.
// Nothing in this file or `VpayCheckoutFlutterPlugin` pretends otherwise.
//
// This file **compiles and runs on a real iOS Simulator** as of 2026-09-16
// (`flutter build ios --simulator --debug` from `example/`) — unlike the
// "Compiled by nobody" claim earlier files in this plugin's history carried,
// which was true only while this repository had no macOS/iOS toolchain at
// all. It has not been driven against a real device, a real MTN/Orange
// endpoint or a real Universal Link, which is a different, narrower claim
// than "compiled by nobody" and is stated as such above.
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
    // **The swipe-away signal, and why it is not the delegate below.**
    //
    // `SFSafariViewControllerDelegate.safariViewControllerDidFinish` fires
    // when the payer taps *Done*. It does NOT fire when they drag the
    // sheet down — an interactive dismissal is reported by UIKit's
    // presentation machinery, not by Safari's own delegate. With
    // `.pageSheet` and a grabber (set below, D5), dragging it away is the
    // obvious gesture, so the common case was the unreported one.
    //
    // Measured on an iPhone 17 Pro simulator on 2026-09-17: the Dart side
    // awaits this event with no timeout, so a swipe-away left
    // `SheetController._handOffToBrowser` suspended forever and the sheet
    // spun on "Redirection vers Orange Money" indefinitely — the payer's
    // money could already have moved and the app would never say so.
    //
    // `presentationController` exists as soon as the presentation is
    // requested, which is the same reasoning the detent configuration
    // below relies on, so it is safe to read on the line after `present`.
    safari.presentationController?.delegate = self
    // The maintainer's explicit "large detent, draggable to full height"
    // decision (D5, revised 2026-09-16) — the same one the deleted
    // `VpayCheckoutViewController` applied to its own `.pageSheet` via
    // `viewDidLoad`. `SFSafariViewController` has no `viewDidLoad` hook of
    // its own to override, but `sheetPresentationController` is built by
    // UIKit as soon as the presentation is requested (the same reasoning
    // the deleted controller's header gave for reading it in `viewDidLoad`
    // before the presentation animation had run), so it is safe to
    // configure immediately after this `present` call rather than from
    // some later callback. `nil` on iOS 13/14 (a plain
    // `UIPresentationController`, not that subclass) and on iOS 12 (no
    // non-full-screen modal presentation API at all) — both fall back to
    // `.pageSheet`'s already-card-like default or `.fullScreen`
    // unchanged, exactly as the deleted controller's header explained.
    if #available(iOS 15.0, *), let sheet = safari.sheetPresentationController {
      sheet.detents = [.large()]
      sheet.prefersGrabberVisible = true
    }
  }

  /// Called by `VpayCheckoutFlutterPlugin.application(_:continue:restorationHandler:)`
  /// when an incoming Universal Link matched one of
  /// `ShowCheckoutRequest.stopUrls` (D2: scheme+host+port+path, query and
  /// fragment ignored) while this session's sheet is showing. Wired but
  /// UNVERIFIED — see this file's header.
  func reportStopUrlReached(url: URL) {
    guard !reported else { return }
    report(CheckoutWindowEvent(outcome: .stopUrlReached, reachedUrl: url.absoluteString))
    safari.presentingViewController?.dismiss(animated: true)
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
    // The payer tapped "Done". This covers ONE of the two ways out; the
    // other is a swipe, which arrives at
    // `presentationControllerDidDismiss` below. No navigation delegate
    // exists to watch a stop URL against; that only ever arrives as a
    // Universal Link, above.
    reportDismissedIfNeeded()
  }
}

extension VpayCheckoutExternalBrowserSession: UIAdaptivePresentationControllerDelegate {
  /// The payer dragged the sheet away rather than tapping *Done*.
  ///
  /// UIKit calls this only for an *interactive* dismissal that has already
  /// completed, so it cannot double-report against a programmatic
  /// `dismiss()` — and `report` is idempotent regardless (`reported`).
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    reportDismissedIfNeeded()
  }
}
