// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.
//
// This package **compiles and runs on a real iOS Simulator** as of
// 2026-09-16 (`flutter build ios --simulator --debug` from `example/`) —
// see `Sources/vpay_checkout_flutter/VpayCheckoutExternalBrowserSession.swift`'s
// header for what that claim does and does not cover.

import PackageDescription

let package = Package(
  name: "vpay_checkout_flutter",
  platforms: [
    // 15.0, not D-M4's original 12.0 "widest reach" floor: Swift
    // Concurrency (this plugin's `@MainActor` hosts, load-bearing) needs
    // 13+, and Flutter 3.48 itself pins new/regenerated iOS projects to 15
    // regardless — see `vpay_checkout_flutter.podspec`'s matching comment.
    .iOS("15.0")
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
