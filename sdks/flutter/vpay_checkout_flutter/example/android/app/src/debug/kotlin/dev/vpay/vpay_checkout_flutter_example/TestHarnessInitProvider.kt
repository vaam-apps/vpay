package dev.vpay.vpay_checkout_flutter_example

import android.app.Application
import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import dev.vpay.checkout_flutter.VpayCheckoutActivityTestHarness

/**
 * DEBUG-ONLY (this file lives in `src/debug/kotlin`; the `<provider>` that
 * runs it is declared only in `src/debug/AndroidManifest.xml`, so neither
 * exists in a release build of this example app). Runs before
 * `Application.onCreate` — the standard Android content-provider auto-init
 * mechanism, the same one `androidx.startup` itself is built on — and
 * wires [TestHarnessBridge.evaluateJavascript]
 * (`MainActivity.kt`'s own doc comment) to
 * `VpayCheckoutActivityTestHarness.evaluateJavascriptForTests`
 * (`dev.vpay.checkout_flutter`'s own `debug` source set). Neither the
 * plugin's harness nor this provider exists in a release build, so
 * [TestHarnessBridge.evaluateJavascript] simply stays `null` there.
 */
class TestHarnessInitProvider : ContentProvider() {
  override fun onCreate(): Boolean {
    val application = context?.applicationContext as? Application ?: return true
    VpayCheckoutActivityTestHarness.install(application)
    TestHarnessBridge.evaluateJavascript =
      VpayCheckoutActivityTestHarness::evaluateJavascriptForTests
    return true
  }

  override fun query(
    uri: Uri,
    projection: Array<String>?,
    selection: String?,
    selectionArgs: Array<String>?,
    sortOrder: String?,
  ): Cursor? = null

  override fun getType(uri: Uri): String? = null

  override fun insert(uri: Uri, values: ContentValues?): Uri? = null

  override fun delete(uri: Uri, selection: String?, selectionArgs: Array<String>?): Int = 0

  override fun update(
    uri: Uri,
    values: ContentValues?,
    selection: String?,
    selectionArgs: Array<String>?,
  ): Int = 0
}
