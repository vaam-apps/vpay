group = "dev.vpay.checkout_flutter"
version = "1.0-SNAPSHOT"

buildscript {
    val kotlinVersion = "2.4.0"
    repositories {
        google()
        mavenCentral()
    }

    dependencies {
        classpath("com.android.tools.build:gradle:9.1.0")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:$kotlinVersion")
    }
}

allprojects {
    repositories {
        google()
        mavenCentral()
    }
}

plugins {
    id("com.android.library")
}

android {
    namespace = "dev.vpay.checkout_flutter"

    compileSdk = flutter.compileSdkVersion

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            java.srcDirs("src/main/kotlin")
        }
        // The debug-only `src/debug/kotlin` source set (Lane E's
        // JS-evaluation test harness, VpayCheckoutActivityTestHarness.kt)
        // was retired 2026-09-16 with the WebView it reached for — see
        // VpayCheckoutActivity.kt's own file header ("browser, not
        // WebView"). Nothing lives under `src/debug/kotlin` any more, so
        // there is no debug source set to declare here.
        getByName("test") {
            java.srcDirs("src/test/kotlin")
        }
    }

    defaultConfig {
        // D-M4 (docs/plans/2026-09-13-flutter-plugin.md): the widest reach,
        // not the newest API — the low-end Android handset is the payer
        // this repository is actually for.
        minSdk = 21
    }

    testOptions {
        unitTests {
            isIncludeAndroidResources = true
            all {
                it.useJUnitPlatform()

                it.outputs.upToDateWhen { false }

                it.testLogging {
                    events("passed", "skipped", "failed", "standardOut", "standardError")
                    showStandardStreams = true
                }
            }
        }
    }
}

kotlin {
    compilerOptions {
        jvmTarget = org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17
    }
}

dependencies {
    // The generated pigeon Kotlin (Messages.g.kt) uses suspend functions and
    // CoroutineScope(Dispatchers.Main).launch { … } to bridge the channel's
    // callback shape to Kotlin coroutines.
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    // VpayCheckoutActivity is a ComponentActivity, for onBackPressedDispatcher
    // (design doc D5: back press is always a dismissal).
    implementation("androidx.activity:activity:1.9.3")
    // D5/D8, revised 2026-09-16 ("browser, not WebView"): every checkout
    // window is now a partial (bottom sheet) Custom Tab
    // (`VpayCheckoutActivity.launchCustomTab`,
    // `setInitialActivityHeightPx`/`setToolbarCornerRadiusDp`/
    // `setCloseButtonPosition`, all confirmed present in this version). No
    // custom URL scheme anywhere (ADR-0021/D8) — return detection is either
    // tier 0 (this Activity's own second `onResume`) or, if a merchant has
    // configured real App Links, an incoming deep link forwarded by
    // `VpayCheckoutAppLinkActivity`.
    implementation("androidx.browser:browser:1.8.0")
    // The old Material `BottomSheetBehavior`/`CoordinatorLayout` sheet this
    // module used to build around a `WebView` is gone as of the same
    // revision — Chrome's own partial-Custom-Tabs feature is the bottom
    // sheet now, so `com.google.android.material:material` and
    // `androidx.coordinatorlayout:coordinatorlayout` are no longer used
    // anywhere in this module and are deliberately not dependencies any
    // more.

    testImplementation("org.jetbrains.kotlin:kotlin-test")
    testImplementation("org.mockito:mockito-core:5.0.0")
}
