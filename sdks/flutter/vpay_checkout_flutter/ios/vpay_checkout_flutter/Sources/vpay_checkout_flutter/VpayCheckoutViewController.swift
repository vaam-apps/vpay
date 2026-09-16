// The iOS native window itself (design doc D5). A `UIViewController` wrapping
// a full-bleed `WKWebView` — nothing else. This class decides nothing about
// the payer's payment outcome (D1): it only watches for a navigation that
// matches one of `ShowCheckoutRequest.stopUrls`, or a dismissal (including
// the interactive swipe-to-dismiss gesture), and reports which.
// `checkout_controller.dart`'s poll of
// `GET /v1/browser/payment_intents/{id}` is the only thing that ever says
// `succeeded`.
//
// Non-negotiables (docs/plans/2026-09-13-flutter-plugin-brief.md, design D5):
// - `webView(_:decidePolicyFor:decisionHandler:)` is the stop check; this
//   file does **not** implement
//   `webView(_:didReceive:completionHandler:)` (the `URLAuthenticationChallenge`
//   hook a WKNavigationDelegate can use to bypass TLS trust evaluation) at
//   all — the platform's own certificate validation runs unmodified. A
//   payment SDK that proceeded through a certificate error would be the
//   single worst thing this plugin could ship, so it is easier to audit
//   this file for the absence of that method than to trust a correct
//   implementation of one.
// - No `WKScriptMessageHandler` and no `evaluateJavaScript` bridge anywhere
//   in this file (design doc D3): the page loaded here is third-party rail
//   content, not vpay's own.
// - `WKWebViewConfiguration.websiteDataStore` is the **default, persistent**
//   store (D-M5) — never `.nonPersistent()`.
import UIKit
import WebKit

/// **Compiled by nobody** (this repository has no macOS/iOS toolchain — see
/// `docs/plans/2026-09-13-flutter-plugin-brief.md`, "What this host can
/// actually verify"). Reviewed by reading only.
final class VpayCheckoutViewController: UIViewController {
  private let checkoutUrl: URL
  private let stopUrls: [CheckoutStopUrl]

  /// Fired exactly once, mirroring `VpayCheckoutFlutterApi.onWindowEvent`'s
  /// own "exactly once per show" contract.
  var onEvent: ((CheckoutWindowEvent) -> Void)?

  private var webView: WKWebView?
  private var reported = false

  init(checkoutUrl: URL, stopUrls: [CheckoutStopUrl]) {
    self.checkoutUrl = checkoutUrl
    self.stopUrls = stopUrls
    super.init(nibName: nil, bundle: nil)
    modalPresentationStyle = .pageSheet
    isModalInPresentation = false  // allows the interactive swipe (D4/D5).
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("VpayCheckoutViewController does not support storyboards.")
  }

  override func loadView() {
    let configuration = WKWebViewConfiguration()
    // D-M5, decided 2026-09-13: the default persistent data store — never
    // `WKWebsiteDataStore.nonPersistent()`.
    configuration.websiteDataStore = .default()
    let webView = WKWebView(frame: .zero, configuration: configuration)
    webView.navigationDelegate = self
    self.webView = webView
    view = webView
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    // Available by `viewDidLoad` even though the presentation animation has
    // not run yet — UIKit creates the adaptive presentation controller
    // before loading the presented controller's view.
    presentationController?.delegate = self
    webView?.load(URLRequest(url: checkoutUrl))
  }

  /// Called by `VpayCheckoutFlutterPlugin` when `dismiss()` is invoked from
  /// Dart, and by this controller's own `UIAdaptivePresentationController`
  /// delegate on an interactive swipe. A no-op the second time (`show`
  /// fires its event exactly once).
  func reportDismissedIfNeeded() {
    report(CheckoutWindowEvent(outcome: .dismissed, reachedUrl: nil))
  }

  private func report(_ event: CheckoutWindowEvent) {
    guard !reported else { return }
    reported = true
    onEvent?(event)
  }

  /// `true` when `url`'s scheme, host, port and path match one of
  /// `stopUrls` — query and fragment ignored (D2), mirroring
  /// `checkout_controller.dart`'s `StopUrlSpec.matches`.
  private func matchesStopUrl(_ url: URL) -> CheckoutStopUrl? {
    guard let scheme = url.scheme, let host = url.host else { return nil }
    let port = Int64(url.port ?? defaultPort(forScheme: scheme))
    let path = url.path
    return stopUrls.first {
      $0.scheme == scheme && $0.host == host && $0.port == port && $0.path == path
    }
  }

  private func defaultPort(forScheme scheme: String) -> Int {
    switch scheme {
    case "https": return 443
    case "http": return 80
    default: return 0
    }
  }
}

extension VpayCheckoutViewController: WKNavigationDelegate {
  func webView(
    _ webView: WKWebView,
    decidePolicyFor navigationAction: WKNavigationAction,
    decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
  ) {
    guard let url = navigationAction.request.url, matchesStopUrl(url) != nil else {
      decisionHandler(.allow)
      return
    }
    decisionHandler(.cancel)
    report(CheckoutWindowEvent(outcome: .stopUrlReached, reachedUrl: url.absoluteString))
    presentingViewController?.dismiss(animated: true)
  }

  // `webView(_:didReceive:completionHandler:)` is deliberately not
  // implemented — see this file's header.
}

extension VpayCheckoutViewController: UIAdaptivePresentationControllerDelegate {
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    // The payer swiped the sheet down (design doc D5, "the modal's
    // dismissal, interactive swipe included, is D4's dismissal").
    reportDismissedIfNeeded()
  }
}
