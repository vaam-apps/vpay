// The plugin's ONE exported entry point (new as of the 2026-09-16 "browser,
// not WebView" revision — see VpayCheckoutActivity.kt's own header for the
// full context of that cutover). Every other component in this package is
// `android:exported="false"`; App Link intake is the one thing an
// `exported="false"` component genuinely cannot do at all — an external,
// cross-app Intent can only ever reach an exported component — so this
// class exists to be the narrowest possible exported surface, rather than
// widening `VpayCheckoutActivity` itself.
//
// SECURITY TRADEOFF, stated loudly per this task's own instruction rather
// than left implicit: this Activity IS directly addressable by any other
// app on the device, via an explicit
// `Intent(component=VpayCheckoutAppLinkActivity::class, data=<any URI>)`.
// That bypasses Android App Link domain verification entirely — explicit
// component targeting always reaches an exported activity regardless of
// `autoVerify`/asset-link status (AndroidManifest.xml's own comment on this
// activity has the manifest-level half of this same tradeoff). A malicious
// app could therefore spoof a `stopUrlReached` event, with an
// attacker-chosen URL that happens to match the currently-open window's
// configured `stopUrls`, for any checkout this app currently has open.
//
// Why that is an accepted cost and not a hole: D1 (see `Messages.g.kt`'s
// own doc comment on `CheckoutWindowOutcome`) means
// `checkout_controller.dart` NEVER trusts a `stopUrlReached` event, or its
// `reachedUrl`, as proof of anything — it always re-polls the real
// `GET /v1/browser/payment_intents/{id}` before deciding
// succeeded/canceled/failed. The worst a spoofed event can do is close the
// sheet early and trigger that poll, which still resolves against the true
// payment state. This class also never sees the checkout session's own
// credential-bearing URL (D6) — only the public success/cancel URL a
// `stopUrl` already is — so there is nothing reachable here an attacker
// could not already see by watching the merchant's own public redirect.
//
// This class ships with NO `<intent-filter>` of its own — the merchant
// declares one, for their own verified host, against this activity's name
// in their own manifest. `AndroidManifest.xml`'s comment here carries the
// snippet and the reason: the only filter a shared plugin could write is
// `<data android:scheme="https"/>` with no host, which on API 21-30 makes
// every app shipping this plugin a candidate handler for every https link
// on the device, and on API 31+ routes nothing at all. Harmful on old
// Android, inert on new Android.
//
// UNVERIFIED, stated plainly rather than claimed: this repository has no
// HTTPS domain serving `assetlinks.json`, so Android App Link verification
// itself cannot be tested here. This class is wired, not proven — no
// comment in this file claims a real deep-link return has ever been driven
// through it end to end. Until a merchant does both halves, every checkout
// on Android ends as `dismissed`, which D1/D4 make correctness-complete.
//
// This class holds no state and decides nothing itself (D1): it only
// forwards the incoming URI to whichever VpayCheckoutActivity is currently
// open — [VpayCheckoutActivity.reportAppLinkIfOpen], the same
// scheme+host+port+path match (D2) that file's own (now-removed) WebView
// navigation check used to make — and always finishes itself immediately
// with no UI. `AndroidManifest.xml`'s `Theme.Translucent.NoTitleBar` keeps
// it from ever painting a visible frame; `android:noHistory="true"` keeps
// it out of the back stack once it has.
package dev.vpay.checkout_flutter

import android.app.Activity
import android.content.Intent
import android.os.Bundle

class VpayCheckoutAppLinkActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    forward(intent)
    finish()
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    forward(intent)
    finish()
  }

  private fun forward(intent: Intent) {
    if (intent.action != Intent.ACTION_VIEW) return
    val uri = intent.data ?: return
    VpayCheckoutActivity.reportAppLinkIfOpen(uri)
  }
}
