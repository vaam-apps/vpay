// The macOS host (design doc D5, revised 2026-09-16; D8) — Lane C
// (docs/plans/2026-09-13-flutter-plugin-brief.md). This class wires Dart's
// `VpayCheckoutHostApi` calls to opening/"dismissing" a
// `VpayCheckoutExternalBrowserSession` (D8 — the payer's default browser
// via `NSWorkspace`), and forwards its result back over
// `VpayCheckoutFlutterApi.onWindowEvent`. It also owns the one piece of
// platform plumbing `VpayCheckoutExternalBrowserSession` itself cannot
// reach: matching an incoming **Universal Link** against the current
// request's `stopUrls` (D2). It decides nothing about the payment outcome
// itself (D1) — see `VpayCheckoutExternalBrowserSession`'s own header for
// the full account of what this host can and cannot observe on macOS.
//
// There used to be a second mode here (`.inApp`, a `WKWebView` sheet
// wrapped by the now-deleted `VpayCheckoutViewController`), selected by
// `ShowCheckoutRequest.mode`. D5's 2026-09-16 revision removed that field
// from the frozen pigeon contract entirely — the browser is now the
// payer's own, unconditionally. There is exactly one `show` path below,
// not a switch, and no presenter is needed any more: `NSWorkspace.open`
// does not present anything inside this app at all.
//
// `show`/`dismiss` are `@MainActor func` and NOT `public` — load-bearing,
// not a style choice, mirroring the iOS host: `VpayCheckoutHostApiSetup`
// (generated) already dispatches both inside `Task { @MainActor in … }`,
// so a *nonisolated* witness (which `public func` without `@MainActor`
// would produce) hops straight back off the main actor before this body
// runs, and on iOS that is exactly what produced a hard
// `NSInternalInconsistencyException` crash. This file was already wrong in
// exactly that way (`public func show`, no `@MainActor`) before this pass;
// fixed here the same way iOS was fixed. Do not "clean this up" back to
// `public`.
import Cocoa
import FlutterMacOS

public final class VpayCheckoutFlutterPlugin: NSObject, FlutterPlugin, VpayCheckoutHostApi {

  public static func register(with registrar: FlutterPluginRegistrar) {
    let flutterApi = VpayCheckoutFlutterApi(binaryMessenger: registrar.messenger)
    let plugin = VpayCheckoutFlutterPlugin(flutterApi: flutterApi)
    VpayCheckoutHostApiSetup.setUp(binaryMessenger: registrar.messenger, api: plugin)
    registrar.publish(plugin)
    // NOTE what this does NOT buy, unlike the equivalent call on iOS: see
    // `handleUserActivity(_:)`'s header below. macOS's
    // `FlutterAppLifecycleDelegate` has no Universal-Link forwarding hook
    // at all, so this registration only wires ordinary app-lifecycle
    // notifications (activation, hide/unhide, `openURLs`, launch,
    // terminate) — none of which is a Universal Link.
    registrar.addApplicationDelegate(plugin)
  }

  private let flutterApi: VpayCheckoutFlutterApiProtocol
  /// Strong: this class wraps `NSWorkspace.shared.open` rather than being a
  /// presented view controller itself (D8), so nothing else keeps it alive
  /// between `show` and its one `onEvent`.
  private var currentExternalBrowser: VpayCheckoutExternalBrowserSession?
  /// The current request's stop URLs, kept only for as long as a session is
  /// open, so `handleUserActivity(_:)` has something to match an incoming
  /// Universal Link against (D2: scheme+host+port+path, query and fragment
  /// ignored).
  private var currentStopUrls: [CheckoutStopUrl] = []

  init(flutterApi: VpayCheckoutFlutterApiProtocol) {
    self.flutterApi = flutterApi
  }

  // VpayCheckoutHostApi — Dart calling into this host. `VpayCheckoutHostApiSetup`
  // (generated) already dispatches both methods inside `Task { @MainActor in … }`,
  // so AppKit calls below run on the main actor without an extra hop.

  @MainActor func show(request: ShowCheckoutRequest) async throws {
    guard currentExternalBrowser == nil else {
      throw PigeonError(
        code: "already_open",
        message: "vpay_checkout_flutter: a checkout window is already open.",
        details: nil
      )
    }
    let url = try parsedUrl(request.url)

    let session = VpayCheckoutExternalBrowserSession()
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
    session.open(url: url)
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

  @MainActor func dismiss() async throws {
    currentExternalBrowser?.dismiss()
  }

  // MARK: - Universal Link intake (D2/D8) — see this file's own header, and
  // VpayCheckoutExternalBrowserSession.swift's, for the full account.

  /// Matches an incoming Universal Link (an `NSUserActivity` of type
  /// `NSUserActivityTypeBrowsingWeb`) against the currently open session's
  /// `stopUrls`, and reports `.stopUrlReached` if it matches.
  ///
  /// # Why this is a plain method the app must call, not an automatic hook
  ///
  /// On iOS, `FlutterPlugin` has an `application(_:continue:restorationHandler:)`
  /// optional method that `FlutterAppDelegate` forwards to every plugin
  /// registered via `addApplicationDelegate`, so a Universal Link reaches
  /// the plugin with zero app-level code. **macOS has no such hook.**
  /// `FlutterMacOS`'s `FlutterAppLifecycleDelegate` protocol (which
  /// `FlutterPlugin` on macOS extends) only forwards a fixed, enumerated
  /// list of `NSApplicationDelegate` notifications — launch, activation,
  /// hide/unhide, screen/occlusion changes, `application:openURLs:`,
  /// termination — read directly from `FlutterAppLifecycleDelegate.h` in
  /// the macOS engine framework. `application(_:continue:restorationHandler:)`
  /// (the macOS `NSApplicationDelegate` method Universal Links arrive
  /// through) is simply not one of them, and `FlutterAppDelegate.mm` does
  /// not call it on any registered delegate.
  ///
  /// So on macOS, a host app that wants Universal Link intake MUST override
  /// `application(_:continue:restorationHandler:)` in its own
  /// `NSApplicationDelegate` (typically `MainFlutterWindow`'s
  /// `AppDelegate.swift`) and call this method itself, e.g.:
  /// ```swift
  /// override func application(
  ///   _ application: NSApplication,
  ///   continue userActivity: NSUserActivity,
  ///   restorationHandler: @escaping ([NSUserActivityRestoring]) -> Void
  /// ) -> Bool {
  ///   // vpayCheckoutFlutterPlugin obtained from the plugin registry, e.g.
  ///   // via `registrar(forPlugin:)` at launch.
  ///   return vpayCheckoutFlutterPlugin?.handleUserActivity(userActivity) ?? false
  /// }
  /// ```
  /// This repository's own `example/` app does not do this — it was
  /// scaffolded only to prove `flutter build macos --debug` compiles this
  /// plugin (see this package's README/CLAUDE.md instructions), not to
  /// exercise Universal Link intake, and the scaffold is removed after that
  /// build. **Wired but UNVERIFIED**, doubly so on macOS: neither the
  /// `apple-app-site-association` deployment iOS also lacks, NOR the
  /// host-app wiring this method depends on, exist anywhere in this
  /// repository.
  @discardableResult
  @MainActor public func handleUserActivity(_ userActivity: NSUserActivity) -> Bool {
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
