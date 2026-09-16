#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint vpay_checkout_flutter.podspec` to validate before publishing.
#
# See VpayCheckoutFlutterPlugin.swift's header for the state of macOS
# verification: this pass built the plugin against a scaffolded
# `example/macos` and reports the real `flutter build macos --debug`
# output in the PR/summary that shipped it, then removed the scaffold — it
# was not left as a permanent example. A real device, App Links/Universal
# Link intake and a real MTN/Orange endpoint remain unverified.
Pod::Spec.new do |s|
  s.name             = 'vpay_checkout_flutter'
  s.version          = '0.0.1'
  s.summary          = 'The macOS host for vpay_checkout_flutter.'
  s.description      = <<-DESC
Opens vpay's hosted checkout page in the payer's default browser
(NSWorkspace.open) and reports a typed result once a payment intent
actually settles.
                       DESC
  s.homepage         = 'https://github.com/vaam-apps/vpay'
  s.license          = { :type => 'See repository', :text => 'See ../../../LICENSE at the repository root.' }
  s.author           = { 'vpay' => 'noreply@example.com' }

  s.source           = { :path => '.' }
  s.source_files = 'vpay_checkout_flutter/Sources/vpay_checkout_flutter/**/*.swift'

  s.dependency 'FlutterMacOS'

  # 12.0, not the old 10.13 "widest reach for WKWebView" floor — WKWebView
  # is gone from this plugin entirely (see VpayCheckoutExternalBrowserSession.swift).
  # 12.0 is the actual requirement now: Swift Concurrency (this plugin's
  # `@MainActor` hosts, load-bearing per VpayCheckoutFlutterPlugin.swift's
  # header) needs 10.15+ at the very least, and Flutter's own macOS app
  # template (`flutter create --platforms=macos`) itself pins
  # `MACOSX_DEPLOYMENT_TARGET = 12.0` regardless of what this plugin
  # declares — so 10.13 was never an achievable floor for an app embedding
  # this plugin, only an unchecked claim.
  s.platform = :osx, '12.0'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  s.swift_version = '5.0'
end
