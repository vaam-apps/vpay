# Consumer ProGuard/R8 rules for tauri-plugin-vpay-checkout's Android host.
#
# This file is named by `consumerProguardFiles` in build.gradle.kts, so it is
# applied to the *merchant app's* release build, where the Tauri app template
# sets `isMinifyEnabled = true`.
#
# Most of what this module needs is already covered by `:tauri-android`'s own
# consumer rules (tauri-apps/tauri@tauri-v2.11.6
# `crates/tauri/mobile/android/proguard-rules.pro`), which keep:
#
#   -keep @app.tauri.annotation.TauriPlugin public class * {
#     @app.tauri.annotation.Command public <methods>;
#     @app.tauri.annotation.ActivityCallback <methods>;
#     public <init>(...);
#   }
#   -keep @app.tauri.annotation.InvokeArg public class * { *; }
#
# — i.e. `VpayCheckoutPlugin`, its `@Command`/`@ActivityCallback` methods and
# its `@InvokeArg` argument classes survive minification because of those,
# which is why `VpayCheckoutPlugin` and the `@InvokeArg` classes in this
# module are deliberately `public` (Kotlin's default) and not `internal`.
#
# The rules below are this module's own, and are not redundant with those:

# The Rust side resolves this class BY NAME, through JNI —
# `register_android_plugin("dev.vpay.tauri.checkout", "VpayCheckoutPlugin")`
# — so R8 sees no reference to it from any Kotlin or Java call site inside
# the app. The `:tauri-android` rule above already keeps the class itself;
# this pins the name too, which is what a by-name lookup actually needs.
-keep class dev.vpay.tauri.checkout.VpayCheckoutPlugin { *; }

# The two Activities are declared in this module's AndroidManifest.xml, and
# AGP keeps manifest-declared components automatically. Kept explicitly
# anyway, because `VpayCheckoutAppLinkActivity` is an *exported* entry point
# a merchant re-declares by fully-qualified name in their own manifest
# (see AndroidManifest.xml's `tools:node="merge"` snippet) — a renamed class
# would silently break that merge rather than fail the build.
-keep class dev.vpay.tauri.checkout.VpayCheckoutActivity { *; }
-keep class dev.vpay.tauri.checkout.VpayCheckoutAppLinkActivity { *; }

# Deliberately absent: no `-keepclassmembers class * { *; public *; }` for a
# JavaScript interface. This module ships no WebView and no
# `@JavascriptInterface` (D5, revised 2026-09-16 — "browser, not WebView"),
# so the WebView block every Android ProGuard template starts life with has
# nothing to keep here.

# Uncomment to preserve line numbers in release stack traces.
#-keepattributes SourceFile,LineNumberTable
#-renamesourcefileattribute SourceFile
