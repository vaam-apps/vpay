// The Android host for `tauri-plugin-vpay-checkout` (design doc D5, D8),
// ported from
// `sdks/flutter/vpay_checkout_flutter/android/src/main/kotlin/dev/vpay/checkout_flutter/VpayCheckoutFlutterPlugin.kt`
// onto Tauri v2's Android plugin API.
//
// This class only wires the two guest-JS commands to launching/closing
// `VpayCheckoutActivity`, and forwards that Activity's one result back on
// the invoke's `Channel`. It decides nothing about the outcome itself (D1)
// — see `VpayCheckoutActivity.kt`'s own header for where the window (and
// the App Link forwarder, `VpayCheckoutAppLinkActivity`) actually live, and
// why the window is the payer's browser and never a `WebView` or a second
// Tauri webview.
//
// # What the Tauri API actually does here, verified against the sources
//
// Every upstream behaviour this class depends on was read in
// tauri-apps/tauri at tag `tauri-v2.11.6`, under
// `crates/tauri/mobile/android/src/main/java/app/tauri/`:
//
//   - `Plugin.startActivityForResult(invoke, intent, callbackName)`
//     (plugin/Plugin.kt:131) delegates to
//     `PluginHandle.startActivityForResult` (plugin/PluginHandle.kt:40),
//     which calls `PluginManager.startActivityForResult`
//     (plugin/PluginManager.kt:131) — that sets a single callback field and
//     calls `launcher.launch(intent)` on the app Activity's
//     `ActivityResultLauncher`. **It neither resolves nor rejects the
//     invoke**, at launch or at result: the stored `Invoke` is simply
//     handed to the `@ActivityCallback` method by reflection
//     (PluginHandle.kt:41-47). So Tauri does not hold `show` open, and this
//     class is free to `invoke.resolve()` immediately after the launch,
//     which is exactly what the wire contract asks for — `show` resolves
//     once the window is showing, and the outcome arrives later on the
//     `Channel`. No `ActivityResultLauncher` of our own is needed; the
//     second pattern the brief allowed for is not required.
//
//   - Because `PluginManager.startActivityForResultCallback`
//     (PluginManager.kt:43) is a single field shared by every plugin in the
//     app, only one activity-for-result may be in flight at a time. That is
//     already true of this plugin by construction — one checkout window,
//     enforced below — but it is why `already_open` is a real guard and not
//     defensive decoration.
//
//   - `@ActivityCallback` methods are indexed by name
//     (PluginHandle.kt:159-161) and invoked with `isAccessible = true`
//     (PluginHandle.kt:43-45), so `private` is fine; `:tauri-android`'s own
//     consumer ProGuard rules keep `@ActivityCallback <methods>` at any
//     visibility.
//
//   - `invoke.parseArgs(cls)` (plugin/Invoke.kt:28) runs Jackson's
//     `ObjectMapper.readValue`. The mapper (PluginManager.kt:45-56) has
//     `FAIL_ON_UNKNOWN_PROPERTIES` disabled, `FAIL_ON_NULL_FOR_PRIMITIVES`
//     enabled, field visibility ANY, and a registered deserializer that
//     turns the string `"__CHANNEL__:<id>"` into a `Channel`
//     (plugin/Channel.kt:14-23). Nested `@InvokeArg` classes and
//     `List<Nested>` fields are ordinary Jackson databind, resolved from
//     the declared generic field type — which is why `ShowArgs.stopUrls`
//     below can be a `List<StopUrlArgs>`.
//
//   - `Channel.send(JSObject)` (plugin/Channel.kt:26) is the one-way path
//     back to the `tauri::ipc::Channel` the guest passed in.
//     `invoke.resolve()` with no argument sends `null` (Invoke.kt:44-46);
//     `invoke.reject(msg)` sends `{ "message": msg }` (Invoke.kt:48-64).
//
// # D6, in this file specifically
//
// There is no `Log`, no `println` and no `toString()` of a request
// anywhere here. Every rejection message below is a fixed constant that
// interpolates nothing — not the URL, not a stop URL, not a caught
// exception's text. The session URL's fragment is the session's own
// credential, and a rejection string crosses back into JavaScript where
// guest-JS maps it to the fixed `platform_window_failed` anyway.
package dev.vpay.tauri.checkout

import android.app.Activity
import android.net.Uri
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONObject

/**
 * One entry of `show`'s `stopUrls`, already normalised by guest-JS the way
 * `checkout_controller.dart`'s `StopUrlSpec` is: scheme, host, port and
 * path only, with default ports filled in. Query and fragment are ignored
 * (D2) and are not carried at all.
 *
 * Deliberately a separate, mutable, Jackson-populated class rather than a
 * reuse of [CheckoutStopUrl]: `@InvokeArg` classes are written by
 * reflection into fields, and `:tauri-android`'s ProGuard rules keep
 * `@InvokeArg public class *` whole. [CheckoutStopUrl] stays the immutable
 * value the Activity works with.
 */
@InvokeArg
class StopUrlArgs {
  lateinit var scheme: String
  lateinit var host: String
  var port: Long = 0
  lateinit var path: String
}

/** `plugin:vpay-checkout|show`'s arguments — the camelCase keys the wire
 *  contract fixes, and nothing else. */
@InvokeArg
class ShowArgs {
  /**
   * The session's own hosted URL (D6: carries the session secret in its
   * fragment). This host loads exactly this and nothing else — it does not
   * construct a URL of its own, and it never reads anything out of it.
   */
  lateinit var url: String

  /**
   * Matched against an **incoming deep link** only (D2). Empty is normal
   * and must not be treated as an error: a hosted session that only ever
   * forwards one way carries just one of `success_url`/`cancel_url`, and a
   * session with neither carries none.
   */
  var stopUrls: List<StopUrlArgs> = emptyList()

  /**
   * D6's named insecure opt-in, forwarded so a host does not have to
   * re-derive "is this the demo stack" from the URL's scheme itself.
   *
   * Accepted and deliberately **not acted on** here, exactly as the
   * Flutter Android host accepts and does not act on it: the decision to
   * allow an `http://` base URL is taken once, by name, in guest-JS before
   * any window opens, and an Android host that re-checked the scheme would
   * be a second, divergent copy of that policy. It is parsed rather than
   * ignored so that the wire shape stays honest and a future host that
   * does need it finds it already arriving.
   */
  var allowInsecureUrl: Boolean = false

  /**
   * The `tauri::ipc::Channel` the one [CheckoutWindowEvent] travels on.
   * Nullable only because Jackson cannot express "required" for a
   * reference type here; a missing channel is rejected below, because
   * without it a `show` could never report anything and guest-JS would
   * wait for an event that can no longer arrive.
   */
  var onEvent: Channel? = null
}

/**
 * [ShowArgs] after every field has actually been read, which is what makes
 * the single `try` in [VpayCheckoutPlugin.show] sufficient — see that
 * method's own comment. Immutable, non-`lateinit`, and never held beyond
 * the call.
 *
 * It has no `allowInsecureUrl`: the Android host does not act on it (see
 * [ShowArgs.allowInsecureUrl]), and carrying a value no code reads into a
 * second type would only invite someone to start reading it here.
 */
private class ParsedShowArgs(
  val url: String,
  val stopUrls: List<CheckoutStopUrl>,
  val onEvent: Channel?,
)

/** Fixed rejection messages. Each is a constant that interpolates nothing
 *  (D6) and carries the same token the wire contract names, so guest-JS's
 *  mapping to `platform_window_failed` is the only thing reading them. */
private const val ERROR_ALREADY_OPEN = "already_open: a checkout window is already open."
private const val ERROR_INVALID_REQUEST = "invalid_request: show was called with unusable arguments."
private const val ERROR_INVALID_URL = "invalid_url: show was called with an unparseable url."

@TauriPlugin
class VpayCheckoutPlugin(private val activity: Activity) : Plugin(activity) {

  /**
   * The `Channel` of the show currently in flight, or `null` when none is.
   *
   * This field is the plugin-side half of "exactly one event per `show`"
   * (`VpayCheckoutActivity`'s `reported` guard is the window-side half).
   * It is set only after the `already_open` check has passed, and cleared
   * in [onCheckoutResult] before the event is sent, so a channel can
   * never be overwritten while its event is still owed.
   *
   * Touched only on the main thread: `@Command` methods are dispatched
   * from `PluginManager.runCommand` and `@ActivityCallback` methods from
   * an `ActivityResultLauncher` callback, both of which run on the
   * Activity's own thread.
   */
  private var pendingEvents: Channel? = null

  /**
   * Opens the payer's browser on `url` as a partial (bottom sheet) Custom
   * Tab, by starting [VpayCheckoutActivity]. Resolves once that launch has
   * been handed to the system — the contract's "resolves once the window
   * is showing" — and never waits for the outcome, which arrives later on
   * `onEvent`.
   */
  @Command
  fun show(invoke: Invoke) {
    // Parsing AND every read of a parsed field happen inside this one
    // guard, deliberately. Jackson writes `@InvokeArg` fields by
    // reflection, so a missing key leaves a `lateinit` field unset and the
    // throw lands at the first *access*, not at `parseArgs` — a `try`
    // around `parseArgs` alone would let that escape.
    //
    // Caught broadly and reported as a fixed string on purpose (D6):
    // Jackson's own message can quote the offending JSON, and the
    // offending JSON here contains the session URL. Nothing about the
    // exception is bound, forwarded, logged or rethrown. Without this
    // catch the throw would unwind into
    // `PluginManager.dispatchPluginMessage`
    // (plugin/PluginManager.kt:198-206), which rejects with
    // `exception.message` verbatim — that is the leak this prevents.
    val request =
      try {
        val args = invoke.parseArgs(ShowArgs::class.java)
        ParsedShowArgs(
          url = args.url,
          stopUrls =
            args.stopUrls.map {
              CheckoutStopUrl(scheme = it.scheme, host = it.host, port = it.port, path = it.path)
            },
          onEvent = args.onEvent,
        )
      } catch (_: Exception) {
        invoke.reject(ERROR_INVALID_REQUEST)
        return
      }

    val channel = request.onEvent
    if (channel == null) {
      invoke.reject(ERROR_INVALID_REQUEST)
      return
    }

    // Checked BEFORE the channel is stored: rejecting a second `show`
    // must not disturb the first one's pending event. `pendingEvents` is
    // part of the condition as well as `isOpen()`, because the window can
    // have finished while its activity result is still in flight — during
    // that gap the first show's event is still owed, and accepting a
    // second show would silently drop it.
    if (pendingEvents != null || VpayCheckoutActivity.isOpen()) {
      invoke.reject(ERROR_ALREADY_OPEN)
      return
    }

    // `Uri.parse` never throws, so this scheme check is the only thing
    // "unparseable" can honestly mean on Android; the contract's
    // `invalid_url` rejection exists for hosts (iOS) whose URL parser can
    // fail outright. The URL itself is never included in the message.
    if (Uri.parse(request.url).scheme == null) {
      invoke.reject(ERROR_INVALID_URL)
      return
    }

    // The contract also names a `no_activity` rejection. It has no
    // reachable path on Tauri's Android API: `Plugin` is constructed with
    // the app's Activity (`Plugin(activity)`), which is non-null for this
    // object's whole lifetime, so there is no state in which `show` has a
    // plugin but no Activity. No branch is invented for it.

    pendingEvents = channel
    startActivityForResult(
      invoke,
      VpayCheckoutActivity.buildIntent(activity, request.url, request.stopUrls),
      "onCheckoutResult",
    )
    invoke.resolve()
  }

  /**
   * [VpayCheckoutActivity] reporting back. Sends the one
   * [CheckoutWindowEvent] of this show on the stored `Channel` and clears
   * it.
   *
   * `invoke` is required by the `@ActivityCallback` signature
   * (PluginHandle.kt:41-47 invokes `method(instance, invoke, result)`) and
   * is deliberately untouched: `show` already resolved it when the window
   * was launched, and resolving or rejecting the same invoke twice would
   * send a second response for a request that has been answered.
   */
  @ActivityCallback
  @Suppress("UNUSED_PARAMETER")
  private fun onCheckoutResult(invoke: Invoke, result: ActivityResult) {
    val channel = pendingEvents ?: return
    // Cleared first: whatever happens below, this show has had its turn,
    // and the next `show` must not be rejected as `already_open` forever.
    pendingEvents = null

    val event = VpayCheckoutActivity.eventFromResult(result.data)
    channel.send(
      JSObject().apply {
        put("outcome", event.outcome)
        // `JSONObject.put(key, null)` REMOVES the key rather than writing
        // a JSON null, and `JSObject` inherits that (plugin/JSObject.kt's
        // `put` overloads all delegate to `super.put`). The contract says
        // `reachedUrl` is `string | null`, so the null case is written
        // explicitly as `JSONObject.NULL` and the key is always present.
        put("reachedUrl", event.reachedUrl ?: JSONObject.NULL)
      },
    )
  }

  /**
   * Closes the window if one is open; a no-op otherwise, and resolving
   * either way is the contract.
   *
   * Sends no event of its own: closing the Activity produces the ordinary
   * activity result, which reaches [onCheckoutResult] and becomes the one
   * `dismissed` event of that show. A `dismiss` with no window open
   * resolves and reports nothing, because there is no show to report to.
   */
  @Command
  fun dismiss(invoke: Invoke) {
    VpayCheckoutActivity.dismissIfOpen()
    invoke.resolve()
  }
}
