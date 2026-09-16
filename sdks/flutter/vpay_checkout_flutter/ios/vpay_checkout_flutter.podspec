#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint vpay_checkout_flutter.podspec` to validate before publishing.
#
# This package **compiles and runs on a real iOS Simulator** as of
# 2026-09-16 (`flutter build ios --simulator --debug` from `example/`) — see
# `vpay_checkout_flutter/Sources/vpay_checkout_flutter/VpayCheckoutExternalBrowserSession.swift`'s
# header for what that claim does and does not cover (a real device, a real
# Universal Link and a real MTN/Orange endpoint remain unverified).
Pod::Spec.new do |s|
  s.name             = 'vpay_checkout_flutter'
  s.version          = '0.0.1'
  s.summary          = 'The iOS host for vpay_checkout_flutter.'
  s.description      = <<-DESC
Opens vpay's hosted checkout page in the payer's own browser
(SFSafariViewController) and reports a typed result once a payment intent
actually settles. See the Dart package's own README.
                       DESC
  s.homepage         = 'https://github.com/vaam-apps/vpay'
  s.license          = { :type => 'See repository', :text => 'See ../../../LICENSE at the repository root.' }
  s.author           = { 'vpay' => 'noreply@example.com' }
  s.source           = { :path => '.' }
  s.source_files = 'vpay_checkout_flutter/Sources/vpay_checkout_flutter/**/*.swift'
  s.dependency 'Flutter'
  # 15.0, not D-M4's original 12.0 "widest reach" floor: Swift Concurrency
  # (this plugin's `@MainActor` hosts, load-bearing — see
  # VpayCheckoutFlutterPlugin.swift's own header) needs 13+, and Flutter
  # 3.48 itself pins new/regenerated iOS projects to 15 regardless. The
  # 12.0 floor this comment used to claim could not actually compile.
  s.platform = :ios, '15.0'

  # Flutter.framework does not contain a i386 slice.
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386' }
  s.swift_version = '5.0'
end
