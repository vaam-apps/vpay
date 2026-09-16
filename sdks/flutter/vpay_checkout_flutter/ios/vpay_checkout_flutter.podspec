#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint vpay_checkout_flutter.podspec` to validate before publishing.
#
# **Compiled by nobody** (docs/plans/2026-09-13-flutter-plugin-brief.md: this
# repository has no macOS/iOS toolchain). Reviewed by reading only.
Pod::Spec.new do |s|
  s.name             = 'vpay_checkout_flutter'
  s.version          = '0.0.1'
  s.summary          = 'The iOS host for vpay_checkout_flutter.'
  s.description      = <<-DESC
Opens vpay's hosted checkout page in a native window and reports a typed
result once a payment intent actually settles. See the Dart package's own
README before shipping VpayCheckoutMode.externalBrowser.
                       DESC
  s.homepage         = 'https://github.com/vaam-apps/vpay'
  s.license          = { :type => 'See repository', :text => 'See ../../../LICENSE at the repository root.' }
  s.author           = { 'vpay' => 'noreply@example.com' }
  s.source           = { :path => '.' }
  s.source_files = 'vpay_checkout_flutter/Sources/vpay_checkout_flutter/**/*.swift'
  s.dependency 'Flutter'
  # D-M4, decided 2026-09-13: the widest reach, not the newest API.
  s.platform = :ios, '12.0'

  # Flutter.framework does not contain a i386 slice.
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES', 'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386' }
  s.swift_version = '5.0'
end
