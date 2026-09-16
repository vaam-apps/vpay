// The macOS native window itself (design doc D5). An `NSViewController`
// wrapping a full-bleed `WKWebView`, presented as a sheet — nothing else.
// This class decides nothing about the payer's payment outcome (D1): it
// only watches for a navigation that matches one of
// `ShowCheckoutRequest.stopUrls`, or a dismissal, and reports which.
// `checkout_controller.dart`'s poll of
// `GET /v1/browser/payment_intents/{id}` is the only thing that ever says
// `succeeded`.
//
// Non-negotiables (docs/plans/2026-09-13-flutter-plugin-brief.md, design D5):
// - `webView(_:decidePolicyFor:decisionHandler:)` is the stop check; this
//   file does **not** implement
//   `webView(_:didReceive:completionHandler:)` (the `URLAuthenticationChallenge`
//   hook a WKNavigationDelegate can use to bypass TLS trust evaluation) at
//   all — the platform's own certificate validation runs unmodified.
// - No `WKScriptMessageHandler` and no `evaluateJavaScript` bridge anywhere
//   in this file (design doc D3).
// - `WKWebViewConfiguration.websiteDataStore` is the **default, persistent**
//   store (D-M5) — never `.nonPersistent()`.
import Cocoa
import WebKit

/// **Compiled by nobody** (this repository has no macOS/iOS toolchain — see
/// `docs/plans/2026-09-13-flutter-plugin-brief.md`, "What this host can
/// actually verify"). Reviewed by reading only.
final class VpayCheckoutViewController: NSViewController {
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
    let frame = NSRect(x: 0, y: 0, width: 480, height: 720)
    let webView = WKWebView(frame: frame, configuration: configuration)
    webView.navigationDelegate = self
    self.webView = webView
    view = webView
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    webView?.load(URLRequest(url: checkoutUrl))
  }

  /// The Escape key — the conventional way a payer dismisses a macOS sheet
  /// with no title bar of its own.
  override func cancelOperation(_ sender: Any?) {
    reportDismissedIfNeeded()
    dismiss(self)
  }

  /// Called by `VpayCheckoutFlutterPlugin` when `dismiss()` is invoked from
  /// Dart. A no-op the second time (`show` fires its event exactly once).
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
    dismiss(self)
  }

  // `webView(_:didReceive:completionHandler:)` is deliberately not
  // implemented — see this file's header.
}
