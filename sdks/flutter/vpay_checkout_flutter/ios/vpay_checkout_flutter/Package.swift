// swift-tools-version: 5.9
// The swift-tools-version declares the minimum version of Swift required to build this package.
//
// **Compiled by nobody** (docs/plans/2026-09-13-flutter-plugin-brief.md: this
// repository has no macOS/iOS toolchain). Reviewed by reading only.

import PackageDescription

let package = Package(
  name: "vpay_checkout_flutter",
  platforms: [
    // D-M4, decided 2026-09-13: the widest reach, not the newest API.
    .iOS("12.0")
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
