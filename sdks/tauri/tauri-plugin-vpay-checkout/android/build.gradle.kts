// The Gradle module for `tauri-plugin-vpay-checkout`'s Android host.
//
// This is a `com.android.library` module that is never built on its own:
// the Tauri CLI includes it into the *consuming app's* Gradle build (the
// app's generated `tauri.settings.gradle` adds an `include` + `projectDir`
// for every `android/` directory a plugin's `build.rs` declares), and
// symlinks the `:tauri-android` project into `./.tauri/tauri-api`. So the
// versions that actually apply to `com.android.library` and
// `org.jetbrains.kotlin.android` come from the app's own root
// `build.gradle.kts` buildscript classpath, not from here — see
// `settings.gradle` next to this file.
//
// Shape copied from the upstream v2 plugin template
// (tauri-apps/tauri@tauri-v2.11.6
// `crates/tauri-cli/templates/plugin/android/build.gradle.kts`) and from
// tauri-apps/plugins-workspace@v2 `plugins/opener/android/build.gradle.kts`
// and `plugins/dialog/android/build.gradle.kts`, which are the maintained
// real-world instances of that template.
plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    // Matches the Kotlin package of the three source files in this module,
    // and the `identifier`/class name pair the Rust side registers with
    // `register_android_plugin("dev.vpay.tauri.checkout", "VpayCheckoutPlugin")`.
    namespace = "dev.vpay.tauri.checkout"

    // 36 is what both `:tauri-android` itself
    // (tauri-apps/tauri@tauri-v2.11.6 `crates/tauri/mobile/android/build.gradle.kts`)
    // and the generated app module compile against, so this module matches
    // them rather than pinning a lower floor of its own. It is also the
    // floor `androidx.browser:browser:1.8.0` and `androidx.activity:activity`
    // need (both require compileSdk 34+).
    compileSdk = 36

    defaultConfig {
        // D-M4 (docs/plans/2026-09-13-flutter-plugin.md, inherited unchanged
        // by docs/plans/2026-09-22-tauri-plugin-brief.md): the widest reach,
        // not the newest API — the low-end Android handset is the payer this
        // repository is actually for. It is also exactly what
        // `:tauri-android` and the upstream plugin template use, so this
        // module does not raise the floor of any app that adopts it.
        minSdk = 21

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // `:tauri-android` names its own `proguard-rules.pro` as the consumer
        // file; the plugin template names a `consumer-rules.pro` it does not
        // ship. This module follows `:tauri-android` so the file named here
        // is the file that exists.
        consumerProguardFiles("proguard-rules.pro")
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }

    // NOTE FOR LANE D, stated rather than hidden: `:tauri-android`, the
    // upstream plugin template and the generated app module all compile at
    // Java/JVM 1.8. This module compiles at 17, matching
    // `sdks/flutter/vpay_checkout_flutter/android/build.gradle.kts` (the host
    // this is a port of) and the JDK 21 the brief's build instructions
    // assume. Kotlin and AGP compile modules independently, so a higher
    // target in one library module is ordinarily fine — but this has been
    // compiled by nobody in this lane. If a Tauri Android build fails with
    // "Cannot inline bytecode built with JVM target 17 into bytecode that is
    // being built with JVM target 1.8", or an equivalent jvmTarget
    // complaint, the fix is to drop these three values to 1.8; nothing in
    // this module's source uses a post-8 language feature.
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}

dependencies {
    // The Tauri Android runtime: `app.tauri.plugin.Plugin`, `Invoke`,
    // `Channel`, `JSObject` and the `app.tauri.annotation.*` annotations this
    // module's `VpayCheckoutPlugin` is built on. Resolved from
    // `./.tauri/tauri-api`, which the Tauri CLI creates — see
    // `settings.gradle`.
    implementation(project(":tauri-android"))

    // D5/D8, revised 2026-09-16 ("browser, not WebView"): every checkout
    // window is a partial (bottom sheet) Custom Tab —
    // `VpayCheckoutActivity.launchCustomTab` uses `setInitialActivityHeightPx`
    // (androidx.browser 1.6+), `setToolbarCornerRadiusDp` and
    // `setCloseButtonPosition`, all present in 1.8.0. No custom URL scheme
    // anywhere in this plugin (ADR-0021/D8). This is the same version and the
    // same three calls the Flutter Android host already depends on.
    implementation("androidx.browser:browser:1.8.0")

    // `VpayCheckoutActivity` is a `ComponentActivity`, for
    // `onBackPressedDispatcher` (D5: a back press is always a dismissal), and
    // `androidx.activity.result.ActivityResult` is the type Tauri's
    // `@ActivityCallback` methods receive.
    implementation("androidx.activity:activity:1.9.3")

    // `app.tauri.plugin.Plugin`'s own overridable members mention
    // `androidx.appcompat.app.AppCompatActivity` (`onDestroy(activity:)`,
    // `onRestart(activity:)`) and `androidx.core.app.ActivityCompat`, so both
    // have to be resolvable to subclass it. Every upstream plugin module
    // carries the same two lines for the same reason.
    implementation("androidx.appcompat:appcompat:1.6.0")
    implementation("androidx.core:core-ktx:1.9.0")

    // Deliberately NOT here, and the omission is the point:
    //  - no `com.google.android.material` / `androidx.coordinatorlayout` —
    //    Chrome's own partial Custom Tab is the bottom sheet now, and the
    //    `WebView` the old Material sheet wrapped is gone (D5 rev. 2026-09-16);
    //  - no `androidx.webkit` — there is no `WebView` in the payment path at
    //    all, on purpose (D5). Nothing in this module imports one.
}
