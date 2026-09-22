// The iOS host (design doc D5, revised 2026-09-16; D8) — Lane C of
// `docs/plans/2026-09-22-tauri-plugin-brief.md`. A port of
// `sdks/flutter/vpay_checkout_flutter/ios/vpay_checkout_flutter/Sources/vpay_checkout_flutter/VpayCheckoutFlutterPlugin.swift`
// onto Tauri v2's Swift plugin API.
//
// This class wires the two commands of the brief's wire contract —
// `plugin:vpay-checkout|show` and `plugin:vpay-checkout|dismiss` — to
// presenting and dismissing a `VpayCheckoutExternalBrowserSession` (D8 —
// `SFSafariViewController`), and forwards its one outcome back over the
// `Channel` the caller passed as `onEvent`. It decides nothing about the
// payment outcome itself (D1): the two outcomes it can report are "the
// payer left the sheet" and "an incoming deep link matched", and `guest-js`
// polls `/v1/browser/payment_intents/{id}` before answering either. See
// `VpayCheckoutExternalBrowserSession.swift`'s header for where the window
// lives and what it can and cannot observe.
//
// # What was verified against upstream, and where
//
// Read on 2026-09-22 at tag `tauri-v2.11.6`, under
// `crates/tauri/mobile/ios-api/Sources/Tauri/`:
//
//   * `Plugin/Plugin.swift` — `open class Plugin: NSObject` with
//     `public let manager: PluginManager = PluginManager.shared`. Its whole
//     overridable surface is `load(webview:)`, `checkPermissions(_:)` and
//     `requestPermissions(_:)`, plus `trigger`/`registerListener`/
//     `removeListener`. Commands are **not** declared on the base class:
//     they are `@objc` methods found by selector, so `show`/`dismiss` below
//     are `@objc public` and nothing overrides anything.
//   * `Tauri.swift` — `PluginManager.invoke(name:invoke:)` looks up
//     `Selector("\(command):completionHandler:")`, then
//     `Selector("\(command):error:")`, then `Selector("\(command):")`.
//     `show(_:) throws` is the second of those (`show:error:`), `dismiss(_:)`
//     the third. It also sets `PluginManager.shared.viewController` from
//     `@_cdecl("on_webview_created")`, which is where `manager.viewController`
//     below comes from — the same property `plugins-workspace@v2`'s
//     `plugins/dialog/ios/Sources/DialogPlugin.swift` (`presentViewController`)
//     and `plugins/opener/ios/Sources/OpenerPlugin.swift` present from.
//   * `Invoke.swift` — `parseArgs<T: Decodable>(_:)` decodes the raw JSON
//     args with a `JSONDecoder` whose `userInfo[channelDataKey]` is the
//     channel sender; `resolve()` answers with no value; `reject(_:code:…)`
//     answers with `{ "message": … }`.
//   * `Channel.swift` — `public class Channel: Decodable`, decoded from the
//     string `"__CHANNEL__:<id>"`, which is why `ShowArgs.onEvent` is simply
//     a `Channel` field and needs no custom decoding. `send<T: Encodable>(_:)
//     throws` JSON-encodes and hands the string to that sender.
//   * `@_cdecl("init_plugin_vpay_checkout")` at the bottom of this file
//     mirrors `@_cdecl("init_plugin_opener")` in `OpenerPlugin.swift`; the
//     suffix is the plugin identifier `vpay-checkout` with `-` → `_`.
//
// # Threading
//
// `PluginManager.invoke` dispatches every command on `ipcDispatchQueue`, a
// background `DispatchQueue` — commands do **not** arrive on the main
// thread. So all UIKit work, and all of this object's mutable state, is
// confined to `DispatchQueue.main.async` blocks, exactly as `DialogPlugin`
// and `OpenerPlugin` do it. The Flutter plugin this is a port of used
// `@MainActor` instead; that would be wrong here, because Tauri reaches
// these methods through the Objective-C runtime (`perform(_:with:)` and
// `unsafeBitCast` of the method IMP), which bypasses actor isolation
// entirely rather than hopping for it. Do not "clean this up" into
// `@MainActor`.
//
// # Universal Link intake (D2) — HONESTLY ABSENT ON TAURI TODAY
//
// The Flutter host reported `stopUrlReached` from
// `application(_:continue:restorationHandler:)`, delivered because
// `FlutterPlugin` lets a plugin register itself as an application delegate
// (`registrar.addApplicationDelegate(self)`). **Tauri v2.11.6 has no
// equivalent.** Three things were checked before concluding that:
//
//   1. `Plugin.swift` (above) exposes no app-lifecycle hook at all.
//   2. A grep for `userActivity`, `NSUserActivity`, `openURL`,
//      `UIApplicationDelegate` and `applicationDelegate` over every file in
//      `crates/tauri/mobile/ios-api/Sources/Tauri/` at `tauri-v2.11.6`
//      returns nothing.
//   3. `tauri-apps/plugins-workspace@v2`'s `deep-link` plugin — the one
//      plugin whose entire job is Universal Links — **ships no `ios/`
//      directory**. `plugins/deep-link/src/lib.rs` handles iOS on the Rust
//      side instead, via `.on_event(…)` matching
//      `tauri::RunEvent::Opened { urls }`. That is the only iOS deep-link
//      seam Tauri has, and it is not reachable from a Swift plugin.
//
// So `stopUrlReached` **cannot be produced on iOS through this plugin
// today, and every iOS checkout ends `dismissed`.** That is a deviation
// from the brief's wire contract as written, and the docs lane must record
// it as a dated ⛔ rather than leave the wire contract reading as though
// three hosts implement it. It is not a correctness bug: D1 and D4 make
// `dismissed` the ordinary end of a *successful* payment too, and
// `guest-js` runs the same poll either way — a payer who paid and then
// tapped Done still resolves `succeeded`. It is a latency and diagnostics
// gap, not a money gap.
//
// The matching logic (`matchesStopUrl(_:)`) and the entry point a future
// hook would call (`handleUniversalLink(_:)`) are implemented below and
// **nothing calls them**. Keeping them, loudly labelled, is the honest
// option: the alternative is either inventing a hook that does not exist,
// or deleting the D2 rule from the only host that will eventually need it.
//
// # D6
//
// No `print`, no `NSLog`, no `os_log` anywhere in this package, and no
// rejection message ever interpolates a value. The URL this plugin is
// handed carries the session secret in its fragment; every rejection below
// is one of four fixed tokens.

import Foundation
import Tauri
import UIKit

/// One stop URL, already normalised the way `guest-js`'s controller is:
/// scheme, host, port and path only. Query and fragment are ignored (D2), so
/// they are not carried across the IPC boundary at all — there is nothing
/// here for this host to compare against them by mistake.
struct StopUrlArgs: Decodable {
  let scheme: String
  let host: String
  let port: Int
  let path: String
}

/// The arguments of `plugin:vpay-checkout|show`, camelCase exactly as the
/// brief's wire contract spells them.
struct ShowArgs: Decodable {
  /// The session's own hosted `url` (D6: carries the session secret in its
  /// fragment). This host loads exactly this and nothing else — it does not
  /// construct a URL of its own.
  let url: String

  /// Optional only so that a caller which omits the key still gets a window
  /// rather than a decode failure; `guest-js` always sends it, possibly
  /// empty. Empty is a real case: a hosted session that forwards only one
  /// way has one stop URL, and one that forwards neither way has none.
  let stopUrls: [StopUrlArgs]?

  /// D6's named insecure opt-in, forwarded so that a host does not have to
  /// re-derive "is this the demo stack" from the URL's scheme itself. This
  /// host **reads it and does nothing with it**, the same as the Flutter iOS
  /// host: the refusal happens in `guest-js` before `show` is ever called,
  /// and a second, subtly different rule here would be a way for the two to
  /// disagree. It is decoded rather than ignored so that the field's
  /// presence in the wire contract stays visible in this file.
  let allowInsecureUrl: Bool?

  /// The Tauri `Channel` the one outcome is sent on. Decodes from the
  /// `"__CHANNEL__:<id>"` string `@tauri-apps/api/core`'s `Channel`
  /// serialises to; `Invoke.parseArgs` supplies the sender through the
  /// decoder's `userInfo`.
  let onEvent: Channel
}

class VpayCheckoutPlugin: Plugin {
  /// Strong: this class wraps a presented `SFSafariViewController` rather
  /// than being one itself (D8), so nothing else keeps it alive between
  /// `show` and its one event.
  ///
  /// Main queue only — see this file's header on threading.
  private var currentSession: VpayCheckoutExternalBrowserSession?

  /// The current request's stop URLs, kept only for as long as a session is
  /// open. On the Flutter host this fed
  /// `application(_:continue:restorationHandler:)`; here it feeds
  /// `handleUniversalLink(_:)`, which nothing calls. Kept because the day
  /// Tauri grows the hook, the wrong thing to have to reconstruct is which
  /// request's rules were in force.
  ///
  /// Main queue only.
  private var currentStopUrls: [StopUrlArgs] = []

  // MARK: - Commands

  /// `plugin:vpay-checkout|show`.
  ///
  /// Resolves with no value once the sheet is being presented; the outcome
  /// arrives later, exactly once, on `args.onEvent`.
  ///
  /// `throws` is declared because it fixes the Objective-C selector Tauri
  /// looks for (`show:error:`, see the header) — but this method never
  /// actually throws. A thrown Swift error would be stringified by
  /// `PluginManager.invoke` as `invoke.reject("\(error)")`, and a
  /// `DecodingError` can quote the value it choked on. One of those values
  /// is `url`. So every failure below rejects explicitly, with a fixed
  /// token (D6).
  @objc public func show(_ invoke: Invoke) throws {
    let args: ShowArgs
    do {
      args = try invoke.parseArgs(ShowArgs.self)
    } catch {
      invoke.reject("invalid_arguments")
      return
    }

    guard let url = URL(string: args.url) else {
      invoke.reject("invalid_url")
      return
    }

    DispatchQueue.main.async { [self] in
      guard currentSession == nil else {
        invoke.reject("already_open")
        return
      }
      guard let presenter = manager.viewController else {
        invoke.reject("no_presenter")
        return
      }

      let session = VpayCheckoutExternalBrowserSession(url: url)
      let channel = args.onEvent
      session.onEvent = { [weak self] event in
        // Clear first: the plugin holds the session and the session holds
        // this closure, so releasing the session here is what breaks the
        // cycle, and doing it before the send means a `show` issued from
        // the event handler is not spuriously `already_open`.
        self?.currentSession = nil
        self?.currentStopUrls = []
        // `try?`: `Channel.send` can only fail if `JSONEncoder` fails on
        // `CheckoutWindowEvent`, which is one enum and one optional string.
        // There is nowhere to report such a failure to — D6 forbids logging
        // and the invoke has already resolved — and `guest-js`'s rule 6
        // ("the event cannot be lost") answers `unresolved` for a host that
        // never reports, rather than hanging.
        try? channel.send(event)
      }
      currentSession = session
      currentStopUrls = args.stopUrls ?? []
      // If `presenter` is already presenting something, UIKit refuses this
      // and no event will ever follow. That is the same exposure the
      // Flutter host has, and the same one `guest-js`'s rule 6 backstops;
      // it is written down here rather than papered over with a retry that
      // could present the payer's checkout twice.
      session.present(from: presenter)
      invoke.resolve()
    }
  }

  /// `plugin:vpay-checkout|dismiss`. Closes the sheet if one is open, and a
  /// no-op otherwise. Closing it reports `dismissed` on the open session's
  /// channel, through the same one-event-per-`show` guard as every other
  /// way out.
  @objc public func dismiss(_ invoke: Invoke) {
    DispatchQueue.main.async { [self] in
      currentSession?.dismiss()
      invoke.resolve()
    }
  }

  // MARK: - Universal Link intake (D2) — implemented, unreachable

  /// What `application(_:continue:restorationHandler:)` would call if Tauri
  /// forwarded it. **Nothing calls this.** See this file's header for the
  /// three upstream sources that establish there is no such hook in
  /// `tauri-v2.11.6`, and for why `dismissed` is nonetheless
  /// correctness-complete (D1/D4).
  ///
  /// If a future Tauri exposes an app-delegate seam — or if a Rust-side
  /// forwarder off `tauri::RunEvent::Opened { urls }` is ever added to this
  /// plugin's crate — this is the one function it should call, from the
  /// main queue. It returns `true` only when a session is open **and** the
  /// incoming link matched one of its `stopUrls`, so that an unrelated
  /// link is left for whoever else wants it.
  func handleUniversalLink(_ url: URL) -> Bool {
    guard let session = currentSession, matchesStopUrl(url) else {
      return false
    }
    session.reportStopUrlReached(url: url)
    return true
  }

  /// `true` when `url`'s scheme, host, port and path match one of
  /// `currentStopUrls` — query and fragment ignored (D2), mirroring
  /// `guest-js`'s `StopUrlSpec` match and the Flutter host's
  /// `matchesStopUrl`.
  private func matchesStopUrl(_ url: URL) -> Bool {
    guard let scheme = url.scheme, let host = url.host else { return false }
    let port = url.port ?? defaultPort(forScheme: scheme)
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

/// The C entry point `tauri-cli`'s generated Xcode project calls to register
/// this plugin, named after the plugin identifier `vpay-checkout` with `-`
/// replaced by `_`. Mirrors `@_cdecl("init_plugin_opener")` in
/// `plugins-workspace@v2`'s `plugins/opener/ios/Sources/OpenerPlugin.swift`.
@_cdecl("init_plugin_vpay_checkout")
func initPlugin() -> Plugin {
  return VpayCheckoutPlugin()
}
