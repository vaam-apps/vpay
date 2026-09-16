import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/src/checkout_controller.dart'
    show StopUrlSpec;
import 'package:vpay_checkout_flutter/src/platform/checkout_platform.dart';
import 'package:vpay_checkout_flutter/src/platform/messages.g.dart';
import 'package:vpay_checkout_flutter/src/platform/method_channel_checkout_platform.dart';

/// Records what [MethodChannelVpayCheckoutPlatform] hands to the pigeon
/// `VpayCheckoutHostApi` without a real `BinaryMessenger` — the same
/// injection point `checkout_platform.dart`'s own doc comment gives as this
/// seam's reason to exist.
class _RecordingHostApi extends VpayCheckoutHostApi {
  ShowCheckoutRequest? lastShowRequest;
  bool dismissed = false;
  Object? showError;

  @override
  Future<void> show(ShowCheckoutRequest request) async {
    lastShowRequest = request;
    final Object? error = showError;
    if (error != null) {
      throw error;
    }
  }

  @override
  Future<void> dismiss() async {
    dismissed = true;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  group('MethodChannelVpayCheckoutPlatform.show', () {
    test('converts each StopUrlSpec to a CheckoutStopUrl carrying the same scheme, host, port and path', () async {
      final hostApi = _RecordingHostApi();
      final platform = MethodChannelVpayCheckoutPlatform(hostApi: hostApi);

      await platform.show(
        url: 'https://checkout.example/c/cs_123#secret',
        stopUrls: const [
          StopUrlSpec(
            scheme: 'https',
            host: 'shop.example',
            port: 443,
            path: '/success',
          ),
          StopUrlSpec(
            scheme: 'http',
            host: 'shop.example',
            port: 8080,
            path: '/cancel',
          ),
        ],
        allowInsecureUrl: false,
        mode: CheckoutWindowMode.inApp,
      );

      final ShowCheckoutRequest? request = hostApi.lastShowRequest;
      expect(request, isNotNull);
      expect(request!.url, 'https://checkout.example/c/cs_123#secret');
      expect(request.allowInsecureUrl, isFalse);
      expect(request.mode, CheckoutWindowMode.inApp);
      expect(request.stopUrls, [
        CheckoutStopUrl(
          scheme: 'https',
          host: 'shop.example',
          port: 443,
          path: '/success',
        ),
        CheckoutStopUrl(
          scheme: 'http',
          host: 'shop.example',
          port: 8080,
          path: '/cancel',
        ),
      ]);
    });

    test('forwards allowInsecureUrl true unchanged', () async {
      final hostApi = _RecordingHostApi();
      final platform = MethodChannelVpayCheckoutPlatform(hostApi: hostApi);

      await platform.show(
        url: 'http://localhost:8081/c/cs_123#secret',
        stopUrls: const [],
        allowInsecureUrl: true,
        mode: CheckoutWindowMode.inApp,
      );

      expect(hostApi.lastShowRequest!.allowInsecureUrl, isTrue);
      expect(hostApi.lastShowRequest!.stopUrls, isEmpty);
    });

    test('forwards mode: externalBrowser unchanged (D8)', () async {
      final hostApi = _RecordingHostApi();
      final platform = MethodChannelVpayCheckoutPlatform(hostApi: hostApi);

      await platform.show(
        url: 'https://checkout.example/c/cs_123#secret',
        stopUrls: const [],
        allowInsecureUrl: false,
        mode: CheckoutWindowMode.externalBrowser,
      );

      expect(hostApi.lastShowRequest!.mode, CheckoutWindowMode.externalBrowser);
    });

    test('propagates a failure the host API throws, without swallowing it', () {
      final hostApi = _RecordingHostApi()
        ..showError = StateError('host refused');
      final platform = MethodChannelVpayCheckoutPlatform(hostApi: hostApi);

      expect(
        () => platform.show(
          url: 'https://checkout.example/c/cs_123#secret',
          stopUrls: const [],
          allowInsecureUrl: false,
          mode: CheckoutWindowMode.inApp,
        ),
        throwsA(isA<StateError>()),
      );
    });
  });

  test('dismiss() calls the host API\'s dismiss', () async {
    final hostApi = _RecordingHostApi();
    final platform = MethodChannelVpayCheckoutPlatform(hostApi: hostApi);

    await platform.dismiss();

    expect(hostApi.dismissed, isTrue);
  });

  test('onWindowEvent republishes on windowEvents, unchanged, exactly once per event', () async {
    final platform = MethodChannelVpayCheckoutPlatform(
      hostApi: _RecordingHostApi(),
    );

    final events = <CheckoutWindowEvent>[];
    final subscription = platform.windowEvents.listen(events.add);

    platform.onWindowEvent(
      CheckoutWindowEvent(
        outcome: CheckoutWindowOutcome.stopUrlReached,
        reachedUrl: 'https://shop.example/success?order=1',
      ),
    );
    // Pump the microtask queue so the broadcast stream's listener runs.
    await Future<void>.delayed(Duration.zero);

    expect(events, hasLength(1));
    expect(events.single.outcome, CheckoutWindowOutcome.stopUrlReached);
    expect(events.single.reachedUrl, 'https://shop.example/success?order=1');

    await subscription.cancel();
  });

  test('registerWith sets VpayCheckoutPlatform.instance to a MethodChannelVpayCheckoutPlatform', () {
    MethodChannelVpayCheckoutPlatform.registerWith();

    expect(
      VpayCheckoutPlatform.instance,
      isA<MethodChannelVpayCheckoutPlatform>(),
    );
  });
}
