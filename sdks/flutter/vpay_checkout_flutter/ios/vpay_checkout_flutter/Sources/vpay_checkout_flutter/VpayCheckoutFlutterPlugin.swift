// The iOS host (design doc D5, revised 2026-09-16; D8) — Lane C
// (docs/plans/2026-09-13-flutter-plugin-brief.md). This class wires Dart's
// `VpayCheckoutHostApi` calls to presenting/dismissing a
// `VpayCheckoutExternalBrowserSession` (D8 — `SFSafariViewController`), and
// forwards its result back over `VpayCheckoutFlutterApi.onWindowEvent`. It
// also owns the one piece of platform plumbing `VpayCheckoutExternalBrowserSession`
// itself cannot reach: intercepting an incoming **Universal Link** and
// matching it against the current request's `stopUrls` (D2). It decides
// nothing about the payment outcome itself (D1) — see
// `VpayCheckoutExternalBrowserSession`'s own header for where the actual
// window lives and what it can and cannot observe.
//
// There used to be a second mode here (`.inApp`, a `WKWebView` wrapped by
// the now-deleted `VpayCheckoutViewController`), selected by
// `ShowCheckoutRequest.mode`. D5's 2026-09-16 revision removed that field
// from the frozen pigeon contract entirely — the browser is now the payer's
// own, unconditionally, on every platform. There is exactly one `show` path
// below, not a switch.
//
// This file **compiles and runs on a real iOS Simulator** as of 2026-09-16
// (`flutter build ios --simulator --debug` from `example/`) — see
// `VpayCheckoutExternalBrowserSession.swift`'s header for what that claim
// does and does not cover.
import Flutter
import UIKit

public final class VpayCheckoutFlutterPlugin: NSObject, FlutterPlugin, VpayCheckoutHostApi {

  public static func register(with registrar: FlutterPluginRegistrar) {
    let flutterApi = VpayCheckoutFlutterApi(binaryMessenger: registrar.messenger())
    let plugin = VpayCheckoutFlutterPlugin(
      flutterApi: flutterApi,
      presenterProvider: { [weak registrar] in registrar?.viewController }
    )
    VpayCheckoutHostApiSetup.setUp(binaryMessenger: registrar.messenger(), api: plugin)
    registrar.publish(plugin)
    // Required for `application(_:continue:restorationHandler:)` below to
    // ever be called at all — `FlutterPlugin`'s app-lifecycle callbacks are
    // opt-in per plugin instance, not delivered automatically just because
    // the protocol method is implemented.
    registrar.addApplicationDelegate(plugin)
  }

  private let flutterApi: VpayCheckoutFlutterApiProtocol
  private let presenterProvider: () -> UIViewController?
  /// Strong: this class wraps a presented `SFSafariViewController` rather
  /// than being one itself (D8), so nothing else keeps it alive between
  /// `show` and its one `onEvent`.
  private var currentExternalBrowser: VpayCheckoutExternalBrowserSession?
  /// The current request's stop URLs, kept only for as long as a session is
  /// open, so `application(_:continue:restorationHandler:)` has something
  /// to match an incoming Universal Link against (D2:
  /// scheme+host+port+path, query and fragment ignored).
  private var currentStopUrls: [CheckoutStopUrl] = []

  init(
    flutterApi: VpayCheckoutFlutterApiProtocol,
    presenterProvider: @escaping () -> UIViewController?
  ) {
    self.flutterApi = flutterApi
    self.presenterProvider = presenterProvider
  }

  // VpayCheckoutHostApi — Dart calling into this host. `VpayCheckoutHostApiSetup`
  // (generated) already dispatches both methods inside `Task { @MainActor in … }`,
  // so UIKit calls below run on the main actor without an extra hop.
  //
  // `@MainActor` and non-`public` are both load-bearing, not a style choice:
  // without `@MainActor` this hard-crashes with
  // `NSInternalInconsistencyException: Call must be made on main thread`,
  // because the pigeon-generated `Task { @MainActor in … }` calls a
  // *nonisolated* async witness, which hops straight back off the main
  // actor before touching UIKit. Do not "clean this up".

  @MainActor func show(request: ShowCheckoutRequest) async throws {
    guard currentExternalBrowser == nil else {
      throw PigeonError(
        code: "already_open",
        message: "vpay_checkout_flutter: a checkout window is already open.",
        details: nil
      )
    }
    let url = try parsedUrl(request.url)
    let presenter = try requirePresenter()

    let session = VpayCheckoutExternalBrowserSession(url: url)
    session.onEvent = { [weak self] event in
      self?.currentExternalBrowser = nil
      self?.currentStopUrls = []
      let api = self?.flutterApi
      Task { @MainActor in
        try? await api?.onWindowEvent(event: event)
      }
    }
    currentExternalBrowser = session
    currentStopUrls = request.stopUrls.compactMap { $0 }
    session.present(from: presenter)
  }

  private func parsedUrl(_ raw: String) throws -> URL {
    guard let url = URL(string: raw) else {
      throw PigeonError(
        code: "invalid_url",
        message: "vpay_checkout_flutter: ShowCheckoutRequest.url is not a valid URL.",
        details: nil
      )
    }
    return url
  }

  private func requirePresenter() throws -> UIViewController {
    guard let presenter = presenterProvider() else {
      throw PigeonError(
        code: "no_presenter",
        message: "vpay_checkout_flutter: no view controller to present the checkout window from.",
        details: nil
      )
    }
    return presenter
  }

  @MainActor func dismiss() async throws {
    currentExternalBrowser?.dismiss()
  }

  // MARK: - Universal Link intake (D2/D8)

  /// `FlutterPlugin`'s app-lifecycle callback for
  /// `UIApplicationDelegate.application(_:continue:restorationHandler:)`,
  /// forwarded here because `registrar.addApplicationDelegate(self)` was
  /// called in `register(with:)`. This is the ONLY way a stop URL can ever
  /// be reported now that the payer's page runs in the browser's own
  /// process (D5, revised 2026-09-16) — see
  /// `VpayCheckoutExternalBrowserSession.swift`'s header.
  ///
  /// **Wired but UNVERIFIED as of 2026-09-16** — see that same header for
  /// why: this repository has no HTTPS origin serving
  /// `apple-app-site-association`, so no Universal Link has ever actually
  /// reached this method. The matching logic below has been read, not
  /// exercised by a real incoming link.
  ///
  /// Returns `true` only when a session is open AND the incoming link
  /// matched one of its `stopUrls` — an unmatched or unrelated
  /// `NSUserActivity` is left for any other delegate to handle (`false`),
  /// per `FlutterPlugin`'s own "delegates are called in order of
  /// registration" contract.
  public func application(
    _ application: UIApplication,
    continue userActivity: NSUserActivity,
    restorationHandler: @escaping ([any UIUserActivityRestoring]?) -> Void
  ) -> Bool {
    guard userActivity.activityType == NSUserActivityTypeBrowsingWeb,
      let url = userActivity.webpageURL,
      let session = currentExternalBrowser,
      matchesStopUrl(url)
    else {
      return false
    }
    session.reportStopUrlReached(url: url)
    return true
  }

  /// `true` when `url`'s scheme, host, port and path match one of
  /// `currentStopUrls` — query and fragment ignored (D2), mirroring
  /// `checkout_controller.dart`'s `StopUrlSpec.matches`.
  private func matchesStopUrl(_ url: URL) -> Bool {
    guard let scheme = url.scheme, let host = url.host else { return false }
    let port = Int64(url.port ?? defaultPort(forScheme: scheme))
    let path = url.path
    return currentStopUrls.contains {
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
