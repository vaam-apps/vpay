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
        getByName("debug") {
            // Lane E's JS-evaluation test harness
            // (VpayCheckoutActivityTestHarness.kt) lives only here, so it
            // compiles into a debug build and never a release one — see
            // VpayCheckoutActivity.kt's own file header.
            java.srcDirs("src/debug/kotlin")
        }
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
    // (design doc D5: back press is always a dismissal, never a WebView
    // history pop).
    implementation("androidx.activity:activity:1.9.3")
    // D8's Custom Tabs mode, wired 2026-09-14: `ShowCheckoutRequest.mode`
    // carries `EXTERNAL_BROWSER`, and `VpayCheckoutFlutterPlugin.show`
    // launches a `CustomTabsIntent` for it. No custom URL scheme anywhere
    // (D8) — return detection is tier 0, `Application
    // .ActivityLifecycleCallbacks` watching for the host Activity's own
    // `onResume`, never a scheme callback.
    implementation("androidx.browser:browser:1.8.0")

    testImplementation("org.jetbrains.kotlin:kotlin-test")
    testImplementation("org.mockito:mockito-core:5.0.0")
}
