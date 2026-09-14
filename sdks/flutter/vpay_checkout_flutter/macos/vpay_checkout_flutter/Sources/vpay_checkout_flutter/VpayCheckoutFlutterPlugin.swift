// The macOS host (design doc D5, D8) — Lane C
// (docs/plans/2026-09-13-flutter-plugin-brief.md). This class only wires
// Dart's `VpayCheckoutHostApi` calls to presenting/dismissing either
// `VpayCheckoutViewController` as a sheet (mode `.inApp`) or a
// `VpayCheckoutExternalBrowserSession` (mode `.externalBrowser`, D8 — the
// payer's default browser via `NSWorkspace`, wired 2026-09-14), and
// forwards whichever one's result back over
// `VpayCheckoutFlutterApi.onWindowEvent`. It decides nothing about the
// outcome itself (D1) — see those two classes' own headers for where the
// actual window lives in each mode.
//
// **Compiled by nobody** (this repository has no macOS/iOS toolchain — see
// `docs/plans/2026-09-13-flutter-plugin-brief.md`, "What this host can
// actually verify"). Reviewed by reading only.
import Cocoa
import FlutterMacOS

public final class VpayCheckoutFlutterPlugin: NSObject, FlutterPlugin, VpayCheckoutHostApi {

  public static func register(with registrar: FlutterPluginRegistrar) {
    let flutterApi = VpayCheckoutFlutterApi(binaryMessenger: registrar.messenger)
    let plugin = VpayCheckoutFlutterPlugin(
      flutterApi: flutterApi,
      presenterProvider: { [weak registrar] in registrar?.viewController }
    )
    VpayCheckoutHostApiSetup.setUp(binaryMessenger: registrar.messenger, api: plugin)
    registrar.publish(plugin)
  }

  private let flutterApi: VpayCheckoutFlutterApiProtocol
  private let presenterProvider: () -> NSViewController?
  private weak var current: VpayCheckoutViewController?
  /// Strong, unlike `current` above: this class wraps `NSWorkspace.shared
  /// .open` and a notification observer rather than a presented view
  /// controller (D8), so nothing else keeps it alive between `show` and
  /// its one `onEvent`.
  private var currentExternalBrowser: VpayCheckoutExternalBrowserSession?

  init(
    flutterApi: VpayCheckoutFlutterApiProtocol,
    presenterProvider: @escaping () -> NSViewController?
  ) {
    self.flutterApi = flutterApi
    self.presenterProvider = presenterProvider
  }

  // VpayCheckoutHostApi — Dart calling into this host. `VpayCheckoutHostApiSetup`
  // (generated) already dispatches both methods inside `Task { @MainActor in … }`,
  // so AppKit calls below run on the main actor without an extra hop.

  public func show(request: ShowCheckoutRequest) async throws {
    switch request.mode {
    case .inApp:
      try showInApp(request: request)
    case .externalBrowser:
      try showExternalBrowser(request: request)
    }
  }

  private func showInApp(request: ShowCheckoutRequest) throws {
    guard current == nil else {
      throw PigeonError(
        code: "already_open",
        message: "vpay_checkout_flutter: a checkout window is already open.",
        details: nil
      )
    }
    let url = try parsedUrl(request.url)
    guard let presenter = presenterProvider() else {
      throw PigeonError(
        code: "no_presenter",
        message: "vpay_checkout_flutter: no view controller to present the checkout window from.",
        details: nil
      )
    }

    let controller = VpayCheckoutViewController(
      checkoutUrl: url,
      stopUrls: request.stopUrls.compactMap { $0 }
    )
    controller.onEvent = { [weak self, weak controller] event in
      self?.current = nil
      let api = self?.flutterApi
      Task { @MainActor in
        try? await api?.onWindowEvent(event: event)
      }
      _ = controller
    }
    current = controller
    presenter.presentAsSheet(controller)
  }

  /// D8: the payer's default browser via `NSWorkspace` — see
  /// `VpayCheckoutExternalBrowserSession`'s own header for why this is the
  /// maintainer's call rather than a design-doc-mandated API. No presenter
  /// is needed: `NSWorkspace.shared.open` does not present anything inside
  /// this app at all.
  private func showExternalBrowser(request: ShowCheckoutRequest) throws {
    guard current == nil, currentExternalBrowser == nil else {
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
      let api = self?.flutterApi
      Task { @MainActor in
        try? await api?.onWindowEvent(event: event)
      }
    }
    currentExternalBrowser = session
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

  public func dismiss() async throws {
    if let controller = current {
      controller.reportDismissedIfNeeded()
      controller.dismiss(controller)
    }
    currentExternalBrowser?.dismiss()
  }
}
