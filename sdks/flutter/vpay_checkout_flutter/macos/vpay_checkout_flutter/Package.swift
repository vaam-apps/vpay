// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.
//
// See VpayCheckoutFlutterPlugin.swift's header and
// vpay_checkout_flutter.podspec's matching comment for the state of macOS
// verification and the reasoning behind the platform floor below.

import PackageDescription

let package = Package(
  name: "vpay_checkout_flutter",
  platforms: [
    // 12.0, not the old 10.13 WKWebView-era floor — see
    // vpay_checkout_flutter.podspec's matching comment for the full
    // reasoning (Swift Concurrency's own floor, and Flutter's own macOS
    // app template pinning 12.0 regardless).
    .macOS("12.0")
  ],
  products: [
    .library(name: "vpay-checkout-flutter", targets: ["vpay_checkout_flutter"])
  ],
  dependencies: [],
  targets: [
    .target(
      name: "vpay_checkout_flutter",
      dependencies: []
    )
  ]
)
