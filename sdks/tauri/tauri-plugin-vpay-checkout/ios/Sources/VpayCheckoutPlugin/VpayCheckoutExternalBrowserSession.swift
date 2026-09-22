// The iOS host's ONE window (design doc D5, revised 2026-09-16 — "the payer's
// browser, not an in-app WKWebView"; D8). This is a port of
// `sdks/flutter/vpay_checkout_flutter/ios/vpay_checkout_flutter/Sources/vpay_checkout_flutter/VpayCheckoutExternalBrowserSession.swift`
// onto Tauri v2's Swift plugin API; the reasoning below is that file's and
// survives the port unchanged, because none of it is about Flutter.
//
// `SFSafariViewController`, not `ASWebAuthenticationSession` — design doc
// D8 explains why: with no custom URL scheme (`checked_forward_url` accepts
// only `http(s)`, and a scheme is refused by design — schemes are
// first-come-first-served on Android and this plugin does not add one on
// any platform for consistency), `ASWebAuthenticationSession`'s
// `callbackURLScheme` would never fire below iOS 17.4, and it costs a
// system "… wants to use … to sign in" consent alert that is the wrong
// sentence on a payment. `SFSafariViewController` needs no callback at all:
// it is presented in-process, like a modal view controller, and its
// delegate tells us the moment the payer taps "Done".
//
// Nor a `WKWebView`, and nor a Tauri `WebviewWindow` (D5 rev. 2026-09-16,
// restated for Tauri in the brief's "What no lane may do"). The Flutter
// plugin deleted its in-app `WKWebView` host for this reason and the reason
// applies verbatim to a Tauri app: rendering vpay's payment form inside the
// merchant app's own process puts `evaluateJavaScript`, the cookie store and
// a navigation delegate all within reach of a compromised merchant app —
// i.e. somewhere a payer's PAN and OTP could be read without the payer being
// able to tell. A Tauri app makes that worse, not better: its front end is
// already a `WKWebView` the merchant controls, and `WebviewWindow` would put
// checkout in a sibling of it. The browser's own process cannot be inspected
// that way, and the payer gets a real URL bar.
//
// The cost is stated rather than hidden: no host on any platform can see a
// navigation any more (Apple's own isolation — an `SFSafariViewController`
// exposes no navigation delegate at all, for exactly the same reason this
// plugin wants one). A stop URL can therefore only ever arrive as an
// incoming **Universal Link**, reported here via `reportStopUrlReached(url:)`
// by whoever received it. On Tauri today nobody does — see
// `VpayCheckoutPlugin.swift`'s header, "Universal Link intake", for the
// upstream sources that were read to establish that, and for why D1/D4 make
// this correctness-complete anyway. The only signal this class has of its
// own is the payer leaving the sheet, reported as `.dismissed`.
//
// D1/D4: `guest-js`'s controller polls the real payment intent before
// answering either outcome, so a payer who paid and then tapped Done (no
// Universal Link ever arriving) still resolves `succeeded`, not `canceled`.
// Nothing in this file decides anything about money.
//
// D6: there is no `print`, no `os_log` and no `NSLog` anywhere in this
// package. The URL this class is handed carries the session secret in its
// fragment.

import SafariServices
import UIKit

/// Which of the two signals `guest-js` polls to resolve actually happened.
/// Never a `succeeded`/`canceled`/`failed` member — the design's whole point
/// (D1) is that this interface cannot say that; only the poll of
/// `/v1/browser/payment_intents/{id}` can. The raw values are the brief's
/// wire strings and are what crosses the Tauri `Channel`.
enum CheckoutWindowOutcome: String, Encodable {
  case dismissed
  case stopUrlReached
}

/// The one event per `show`, exactly as the brief's wire contract spells it:
/// `{ "outcome": "dismissed" | "stopUrlReached", "reachedUrl": "https://…" | null }`.
struct CheckoutWindowEvent: Encodable {
  let outcome: CheckoutWindowOutcome

  /// The full URL of the incoming deep link, present only for
  /// `.stopUrlReached` — carried for diagnostics only; `guest-js` reads
  /// nothing off it (D1).
  let reachedUrl: String?

  private enum CodingKeys: String, CodingKey {
    case outcome
    case reachedUrl
  }

  /// Hand-written rather than synthesised on purpose. Swift's synthesised
  /// `Encodable` uses `encodeIfPresent` for an `Optional` property, which
  /// **omits** the key; the wire contract says `reachedUrl` is
  /// `string | null`, and `guest-js` reads the field rather than testing for
  /// its presence. An explicit `encodeNil` keeps the JSON the two other
  /// hosts send.
  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(outcome, forKey: .outcome)
    if let reachedUrl {
      try container.encode(reachedUrl, forKey: .reachedUrl)
    } else {
      try container.encodeNil(forKey: .reachedUrl)
    }
  }
}

/// Wraps the presented `SFSafariViewController` and reports exactly one
/// outcome for it.
///
/// **Main queue only.** Every member is touched from the main queue and
/// nothing here is thread-safe: `VpayCheckoutPlugin` creates, presents and
/// releases this object from inside `DispatchQueue.main.async`, because
/// Tauri dispatches plugin commands on a background queue (see that file's
/// header). UIKit requires it regardless.
final class VpayCheckoutExternalBrowserSession: NSObject {
  private let safari: SFSafariViewController
  private var reported = false

  /// Fired exactly once, mirroring the brief's "the one event, sent on
  /// `onEvent` exactly once per `show`".
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
    // Measured on an iPhone 17 Pro simulator on 2026-09-17, against the
    // Flutter plugin this is a port of: the caller awaits this event with
    // no timeout, so a swipe-away left the checkout sheet suspended
    // forever, spinning on "Redirection vers Orange Money" indefinitely —
    // the payer's money could already have moved and the app would never
    // say so. `guest-js`'s rule 6 ("the event cannot be lost") is the
    // belt to this file's braces, not a replacement for it.
    //
    // `presentationController` exists as soon as the presentation is
    // requested, which is the same reasoning the detent configuration
    // below relies on, so it is safe to read on the line after `present`.
    safari.presentationController?.delegate = self
    // The maintainer's explicit "large detent, draggable to full height"
    // decision (D5, revised 2026-09-16). `SFSafariViewController` has no
    // `viewDidLoad` hook of its own to override, but
    // `sheetPresentationController` is built by UIKit as soon as the
    // presentation is requested, so it is safe to configure immediately
    // after this `present` call rather than from some later callback. The
    // `if let` is not defensive padding: the property is `nil` whenever
    // UIKit chose a plain `UIPresentationController` instead of the sheet
    // subclass, and the fallback is `.pageSheet`'s already-card-like
    // default, unchanged.
    //
    // **`if #available(iOS 15.0, *)` is load-bearing even though
    // `Package.swift` declares `.iOS(.v15)`, and removing it breaks
    // `cargo check`.** `sheetPresentationController`, `.large()` and
    // `prefersGrabberVisible` are all iOS 15.0 APIs, and this package is
    // compiled by two different things with two different floors:
    //
    //   * `swift build`/`xcodebuild` on this SwiftPM package, and the
    //     Xcode project `tauri-cli` generates, honour `Package.swift`'s
    //     `.iOS(.v15)`;
    //   * `cargo check --target aarch64-apple-ios` on the plugin crate
    //     does not read `Package.swift` at all. It goes through
    //     `tauri_utils::build::link_apple_library` →
    //     `swift_rs::SwiftLinker::with_ios(…)`, which reads
    //     `IPHONEOS_DEPLOYMENT_TARGET` from the environment and **defaults
    //     to "13.0"**, then invokes swiftc with `-target
    //     arm64-apple-ios13.0`. Measured by Lane A1 on 2026-09-22: without
    //     the guard that build fails here with "sheetPresentationController
    //     is only available in iOS 15.0 or newer".
    //
    // The guard makes both floors compile from one source, which is why
    // the Flutter session this is a port of carries the same one. On a
    // 13/14 runtime the detents are simply not applied and `.pageSheet`'s
    // default card presentation stands — the same fallback the `if let`
    // above describes. Do not "simplify" either condition away.
    if #available(iOS 15.0, *), let sheet = safari.sheetPresentationController {
      sheet.detents = [.large()]
      sheet.prefersGrabberVisible = true
    }
  }

  /// Reports that an incoming Universal Link matched one of the current
  /// request's `stopUrls` (D2: scheme+host+port+path, query and fragment
  /// ignored) while this session's sheet is showing.
  ///
  /// **Nothing calls this on Tauri today** and it is not reachable from
  /// JavaScript: Tauri v2.11.6's Swift `Plugin` base class forwards no
  /// `UIApplicationDelegate` callback of any kind. See
  /// `VpayCheckoutPlugin.swift`'s header for the exact sources that were
  /// read, and `VpayCheckoutPlugin.handleUniversalLink(_:)` for the seam a
  /// future Tauri version — or a Rust-side forwarder off
  /// `tauri::RunEvent::Opened` — would call. It is kept implemented, and
  /// this comment is kept loud, because a silently missing branch is the
  /// failure this repository cares most about.
  func reportStopUrlReached(url: URL) {
    guard !reported else { return }
    report(CheckoutWindowEvent(outcome: .stopUrlReached, reachedUrl: url.absoluteString))
    safari.presentingViewController?.dismiss(animated: true)
  }

  /// Called by `VpayCheckoutPlugin` when the `dismiss` command is invoked
  /// from JavaScript. A no-op the second time (`show` fires its event
  /// exactly once).
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
    // other is a swipe, which arrives at `presentationControllerDidDismiss`
    // below. No navigation delegate exists to watch a stop URL against;
    // that only ever arrives as a Universal Link, above.
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
