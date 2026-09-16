import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _sessionUrl =
    'https://checkout.example/c/cs_123?key=pk_test_1#$_csSecret';

http.Response _json(Object body, {int status = 200}) =>
    http.Response(jsonEncode(body), status);

Map<String, Object?> _paymentIntentJson(String status) => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 5000,
  'currency': 'xaf',
  'status': status,
  'payment_method_types': ['mtn_momo'],
  'next_action': null,
  'last_payment_error': null,
  'metadata': <String, Object?>{},
  'description': null,
  'created': 1700000000,
  'livemode': false,
  'client_secret': _piSecret,
};

Map<String, Object?> _sessionJson({String uiMode = 'hosted'}) => {
  'id': 'cs_123',
  'object': 'checkout.session',
  'livemode': false,
  'payment_intent': _paymentIntentJson('requires_payment_method'),
  'ui_mode': uiMode,
  'status': 'open',
  'payment_status': 'unpaid',
  'success_url': 'https://shop.example/thanks',
  'cancel_url': 'https://shop.example/cancel',
  'return_url': null,
  'url': _sessionUrl,
  'expires_at': 1700086400,
  'created': 1700000000,
  // No `client_secret` key: the real session read never sends the
  // session's own secret back — see `CheckoutSession.clientSecret`'s doc
  // comment in `lib/src/models.dart`.
};

/// A clock that advances instantly rather than waiting real time — keeps
/// the "still processing" tests below from actually burning the default
/// polling budgets in wall-clock seconds.
class _FakeClock extends VpayClock {
  _FakeClock(this._now);

  DateTime _now;

  @override
  DateTime now() => _now;

  @override
  Future<void> delay(Duration duration) async {
    _now = _now.add(duration);
  }
}

class _FakePlatform extends VpayCheckoutPlatform {
  _FakePlatform(this.outcome);

  final CheckoutWindowOutcome? outcome;
  bool shown = false;
  bool? lastAllowInsecureUrl;

  /// A **broadcast** controller, exactly as both real platform
  /// implementations use — so a test can reproduce the hazard those have:
  /// an event added while nobody is listening is dropped, not buffered.
  final StreamController<CheckoutWindowEvent> _events =
      StreamController<CheckoutWindowEvent>.broadcast();

  /// What a real host does: the outcome is reported *after* `show`'s future
  /// resolves, not before.
  @override
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
  }) async {
    shown = true;
    lastAllowInsecureUrl = allowInsecureUrl;
    final CheckoutWindowOutcome? outcome = this.outcome;
    if (outcome == null) {
      // A host that opened a window and then never reported anything.
      unawaited(Future<void>.microtask(_events.close));
      return;
    }
    unawaited(
      Future<void>.microtask(
        () => _events.add(CheckoutWindowEvent(outcome: outcome)),
      ),
    );
  }

  @override
  Future<void> dismiss() async {}

  @override
  Stream<CheckoutWindowEvent> get windowEvents => _events.stream;
}

/// A host whose `show` fails — a browser that refused the popup, an
/// `Activity` that would not start.
class _RefusingPlatform extends VpayCheckoutPlatform {
  final StreamController<CheckoutWindowEvent> _events =
      StreamController<CheckoutWindowEvent>.broadcast();

  @override
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
  }) async {
    throw StateError(
      'the browser refused to open https://checkout.example/c/cs_123'
      '#$_csSecret',
    );
  }

  @override
  Future<void> dismiss() async {}

  @override
  Stream<CheckoutWindowEvent> get windowEvents => _events.stream;
}

/// A stand-in for "no platform host has registered itself" that is
/// deliberately a DIFFERENT type from [UnimplementedVpayCheckoutPlatform] —
/// see the host-vs-target hazard this sidesteps, documented at this file's
/// one test that sets it.
final class _UnwiredPlatform extends VpayCheckoutPlatform {
  const _UnwiredPlatform();

  @override
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
  }) => throw UnimplementedError('no platform host registered (test double)');

  @override
  Future<void> dismiss() =>
      throw UnimplementedError('no platform host registered (test double)');

  @override
  Stream<CheckoutWindowEvent> get windowEvents =>
      throw UnimplementedError('no platform host registered (test double)');
}

void main() {
  tearDown(() {
    VpayCheckoutPlatform.instance = const UnimplementedVpayCheckoutPlatform();
  });

  group('VpayCheckout.start — no platform host exists yet (Lane C)', () {
    test(
      'throws UnimplementedError once it reaches the platform seam',
      () async {
        // HOST-VS-TARGET HAZARD: this cannot rely on the default
        // `VpayCheckoutPlatform.instance` (reset to
        // `const UnimplementedVpayCheckoutPlatform()` by the `tearDown`
        // above) and just call `checkout.start`. `instance`'s getter
        // (`checkout_platform.dart`) lazily swaps itself for
        // `MethodChannelVpayCheckoutPlatform` the moment it is read on a
        // host where `isNativeMobileHost` is true —
        // `Platform.isAndroid || Platform.isIOS || Platform.isMacOS` from
        // `dart:io`, which under `flutter test` reports the OS running the
        // test process, not a target platform. On a macOS laptop that is
        // true, so `.instance` resolves away to a real method-channel
        // platform before `VpayCheckout.start` ever calls `show`, and
        // nothing throws — this test passed in CI (a Linux runner, where
        // `isNativeMobileHost` is false) and failed silently host-OS-
        // dependently here. `_UnwiredPlatform` below sidesteps the `is
        // UnimplementedVpayCheckoutPlatform` check the getter makes by
        // simply being a different type, so the outcome no longer depends
        // on which OS happens to run the suite.
        VpayCheckoutPlatform.instance = const _UnwiredPlatform();
        final checkout = VpayCheckout(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient((request) async => _json(_sessionJson())),
        );

        await expectLater(
          checkout.start(_sessionUrl),
          throwsA(isA<UnimplementedError>()),
        );
      },
    );

    test('UnimplementedVpayCheckoutPlatform itself throws UnimplementedError '
        'from every member, not only the one VpayCheckout.start happens to '
        'reach first', () {
      // Formerly "VpayCheckoutMode.externalBrowser (D8) with no platform
      // host at all, still throws UnimplementedError": that test proved
      // the stub was not special-cased per mode. `mode` is gone entirely
      // now (VpayCheckoutMode and CheckoutWindowMode were both deleted),
      // so there is no longer a second call shape to prove is untouched.
      // What is left worth asserting on its own — distinct from the test
      // above, which only ever reaches `show` — is that the production
      // stub (`checkout_platform.dart`'s
      // `UnimplementedVpayCheckoutPlatform`) makes good on its contract
      // for `dismiss` and `windowEvents` too, exercised directly rather
      // than through the host-OS-sensitive `VpayCheckoutPlatform.instance`
      // getter (see the test above).
      const platform = UnimplementedVpayCheckoutPlatform();

      expect(
        () => platform.show(
          url: _sessionUrl,
          stopUrls: const [],
          allowInsecureUrl: false,
        ),
        throwsUnimplementedError,
      );
      expect(() => platform.dismiss(), throwsUnimplementedError);
      expect(() => platform.windowEvents, throwsUnimplementedError);
    });
  });

  group(
    'VpayCheckout.start — with a platform host wired in (as Lane C will)',
    () {
      test(
        'a reached stop URL resolves through the poll, not off the URL',
        () async {
          VpayCheckoutPlatform.instance = _FakePlatform(
            CheckoutWindowOutcome.stopUrlReached,
          );
          final checkout = VpayCheckout(
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: MockClient(
              (request) async => request.url.path.contains('checkout/sessions')
                  ? _json(_sessionJson())
                  : _json(_paymentIntentJson('succeeded')),
            ),
          );

          final result = await checkout.start(_sessionUrl);

          expect(result, isA<VpayCheckoutSucceeded>());
        },
      );

      test(
        'a dismissal with a still-processing intent resolves to pending',
        () async {
          VpayCheckoutPlatform.instance = _FakePlatform(
            CheckoutWindowOutcome.dismissed,
          );
          final checkout = VpayCheckout(
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: MockClient(
              (request) async => request.url.path.contains('checkout/sessions')
                  ? _json(_sessionJson())
                  : _json(_paymentIntentJson('processing')),
            ),
            clock: _FakeClock(DateTime(2026, 9, 13)),
          );

          final result = await checkout.start(_sessionUrl);

          expect(result, isA<VpayCheckoutPending>());
        },
      );
    },
  );

  group('VpayCheckout.start — an embedded session is refused before any window opens', () {
    test('never calls the platform host at all', () async {
      final fake = _FakePlatform(CheckoutWindowOutcome.dismissed);
      VpayCheckoutPlatform.instance = fake;
      final checkout = VpayCheckout(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => _json(_sessionJson(uiMode: 'embedded')),
        ),
      );

      final result = await checkout.start(_sessionUrl);

      expect(result, isA<VpayCheckoutUnresolved>());
      expect(
        (result as VpayCheckoutUnresolved).error.code,
        VpayClientErrorCodes.embeddedSessionNotSupported,
      );
      expect(fake.shown, isFalse);
    });
  });

  group('VpayCheckout.start — the window event cannot be lost', () {
    test('an outcome reported after show() resolves is still received, though the stream is broadcast', () async {
      VpayCheckoutPlatform.instance = _FakePlatform(
        CheckoutWindowOutcome.stopUrlReached,
      );
      final checkout = VpayCheckout(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => request.url.path.contains('checkout/sessions')
              ? _json(_sessionJson())
              : _json(_paymentIntentJson('succeeded')),
        ),
      );

      final result = await checkout
          .start(_sessionUrl)
          .timeout(
            const Duration(seconds: 5),
            onTimeout: () => fail(
              'start() hung: the window event was dropped, not delivered',
            ),
          );

      expect(result, isA<VpayCheckoutSucceeded>());
    });

    test('a window that closes without reporting an outcome resolves unresolved rather than hanging', () async {
      VpayCheckoutPlatform.instance = _FakePlatform(null);
      final checkout = VpayCheckout(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient(
          (request) async => request.url.path.contains('checkout/sessions')
              ? _json(_sessionJson())
              : _json(_paymentIntentJson('succeeded')),
        ),
      );

      final result = await checkout
          .start(_sessionUrl)
          .timeout(
            const Duration(seconds: 5),
            onTimeout: () =>
                fail('start() hung on a host that reported nothing'),
          );

      expect(result, isA<VpayCheckoutUnresolved>());
      expect(
        (result as VpayCheckoutUnresolved).error.code,
        VpayClientErrorCodes.platformWindowFailed,
      );
      expect(result, isNot(isA<VpayCheckoutSucceeded>()));
    });
  });

  group('VpayCheckout.start — a platform window that will not open', () {
    test('resolves unresolved with a fixed message that never quotes the thrown value', () async {
      VpayCheckoutPlatform.instance = _RefusingPlatform();
      final checkout = VpayCheckout(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async => _json(_sessionJson())),
      );

      final result = await checkout.start(_sessionUrl);

      expect(result, isA<VpayCheckoutUnresolved>());
      final error = (result as VpayCheckoutUnresolved).error;
      expect(error.code, VpayClientErrorCodes.platformWindowFailed);
      // D6: the StateError's own message quoted the session URL, and the
      // session URL carries the session secret in its fragment.
      expect(error.message, isNot(contains(_csSecret)));
      expect(result.toString(), isNot(contains(_csSecret)));
    });
  });

  group('VpayCheckout.start — the insecure-base opt-in reaches the host', () {
    test('allowInsecureUrl mirrors allowInsecureBaseUrl instead of being hard-coded false', () async {
      final fake = _FakePlatform(CheckoutWindowOutcome.dismissed);
      VpayCheckoutPlatform.instance = fake;
      final checkout = VpayCheckout(
        baseUrl: 'http://localhost:8081',
        publishableKey: 'pk_test_1',
        allowInsecureBaseUrl: true,
        httpClient: MockClient(
          (request) async => request.url.path.contains('checkout/sessions')
              ? _json(_sessionJson())
              : _json(_paymentIntentJson('canceled')),
        ),
      );

      await checkout.start(_sessionUrl);

      expect(fake.lastAllowInsecureUrl, isTrue);
    });

    test(
      'and stays false for an https base, which is every non-demo deployment',
      () async {
        final fake = _FakePlatform(CheckoutWindowOutcome.dismissed);
        VpayCheckoutPlatform.instance = fake;
        final checkout = VpayCheckout(
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: MockClient(
            (request) async => request.url.path.contains('checkout/sessions')
                ? _json(_sessionJson())
                : _json(_paymentIntentJson('canceled')),
          ),
        );

        await checkout.start(_sessionUrl);

        expect(fake.lastAllowInsecureUrl, isFalse);
      },
    );
  });

  group('VpayCheckout.start — a malformed sessionUrl', () {
    test('is refused without any network call', () async {
      var called = false;
      final checkout = VpayCheckout(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((request) async {
          called = true;
          return _json(_sessionJson());
        }),
      );

      final result = await checkout.start(
        'https://checkout.example/c/cs_123?key=pk_test_1',
      );

      expect(result, isA<VpayCheckoutUnresolved>());
      expect(called, isFalse);
    });
  });
}
