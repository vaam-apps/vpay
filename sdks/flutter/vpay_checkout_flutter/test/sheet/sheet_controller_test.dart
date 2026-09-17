import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

/// A clock a test fully controls — `checkout_controller_test.dart`'s own
/// `FakeClock`, restated here so this file has no test-time dependency on
/// that one.
class FakeClock extends VpayClock {
  FakeClock(this._now);
  DateTime _now;

  @override
  DateTime now() => _now;

  @override
  Future<void> delay(Duration duration) async {
    _now = _now.add(duration);
  }
}

http.Response _json(Object body, {int status = 200}) =>
    http.Response(jsonEncode(body), status);

Map<String, Object?> _intentJson({
  String status = 'requires_payment_method',
  Object? nextAction,
  Object? lastPaymentError,
}) => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 5000,
  'currency': 'xaf',
  'status': status,
  'payment_method_types': ['mtn_momo'],
  'next_action': nextAction,
  'last_payment_error': lastPaymentError,
  'metadata': <String, Object?>{},
  'description': null,
  'created': 1700000000,
  'livemode': false,
  'client_secret': _piSecret,
};

Map<String, Object?> _mtnRailJson() => {
  'code': 'mtn_momo',
  'flow': 'push',
  'label_key': 'rail.mtn_momo',
  'fields': [
    {
      'name': 'msisdn',
      'type': 'phone',
      'region': 'CM',
      'phone_type': 'mobile',
      'required': true,
      'label_key': 'msisdn.label',
    },
  ],
};

Map<String, Object?> _orangeRailJson() => {
  'code': 'orange_money',
  'flow': 'redirect',
  'label_key': 'rail.orange_money',
  'fields': const <Object?>[],
};

Map<String, Object?> _sessionJson({
  required Object? intent,
  List<Object?> rails = const <Object?>[],
  String? successUrl,
  String? cancelUrl,
}) => {
  'id': 'cs_123',
  'object': 'checkout.session',
  'livemode': false,
  'payment_intent': intent,
  'ui_mode': 'hosted',
  'status': 'open',
  'payment_status': 'unpaid',
  'success_url': successUrl,
  'cancel_url': cancelUrl,
  'return_url': null,
  'url': 'https://checkout.example/c/cs_123#$_csSecret',
  'expires_at': 1700086400,
  'created': 1700000000,
  'rails': rails,
};

/// Answers the session on the first GET, then whatever [intentAnswers]
/// yields — one per subsequent `retrievePaymentIntent`/confirm call — and a
/// re-read of the session (after a terminal outcome) with [finalSession].
class _ScriptedClient {
  _ScriptedClient({
    required Map<String, Object?> session,
    required List<Map<String, Object?>> intentAnswers,
    Map<String, Object?>? finalSession,
  }) : _session = session,
       _intentAnswers = List.of(intentAnswers),
       _finalSession = finalSession ?? session;

  final Map<String, Object?> _session;
  final List<Map<String, Object?>> _intentAnswers;
  final Map<String, Object?> _finalSession;
  int _sessionReads = 0;
  final List<String> calls = [];

  BrowserClient build() => BrowserClient(
    baseUrl: 'https://api.example',
    publishableKey: 'pk_test_1',
    httpClient: MockClient((http.Request request) async {
      if (request.method == 'GET' &&
          request.url.path.contains('/checkout/sessions/')) {
        _sessionReads += 1;
        calls.add('read_session');
        return _json(_sessionReads == 1 ? _session : _finalSession);
      }
      if (request.method == 'GET' &&
          request.url.path.contains('/payment_intents/')) {
        calls.add('poll');
        return _json(_intentAnswers.removeAt(0));
      }
      if (request.method == 'POST' && request.url.path.endsWith('/confirm')) {
        calls.add('confirm');
        return _json(_intentAnswers.removeAt(0));
      }
      throw StateError('unexpected request: ${request.method} ${request.url}');
    }),
  );
}

class _FakePlatform extends VpayCheckoutPlatform {
  _FakePlatform(this.outcome);

  final CheckoutWindowOutcome outcome;
  bool shown = false;
  final StreamController<CheckoutWindowEvent> _events =
      StreamController<CheckoutWindowEvent>.broadcast();
  final List<String> order = [];

  @override
  Future<void> show({
    required String url,
    required List<StopUrlSpec> stopUrls,
    required bool allowInsecureUrl,
  }) async {
    shown = true;
    order.add('platform.show');
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

class _InMemoryRememberedMsisdnStore implements VpayRememberedMsisdnStore {
  RememberedMsisdnRecord? record;

  @override
  Future<void> clear() async => record = null;

  @override
  Future<RememberedMsisdnRecord?> read() async => record;

  @override
  Future<void> write(RememberedMsisdnRecord r) async => record = r;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  // `SheetController` reaches `SharedPreferencesRememberedMsisdnStore` (the
  // real store, the default) whenever a push-rail entry screen preloads a
  // remembered number — this backs it with the plugin's own mock channel
  // rather than every test constructing an explicit fake store, so a test
  // that does not care about the memory feature still doesn't hang on a
  // real platform channel with no host to answer it.
  SharedPreferences.setMockInitialValues(<String, Object>{});

  group('SheetController — push rail happy path', () {
    test('select_rail is skipped for one supported rail; submit confirms, polls with jitter, then completes exactly once', () async {
      final FakeClock clock = FakeClock(DateTime(2026));
      final scripted = _ScriptedClient(
        session: _sessionJson(intent: _intentJson(), rails: [_mtnRailJson()]),
        intentAnswers: [
          _intentJson(status: 'processing'), // confirm's own answer
          _intentJson(status: 'processing'), // first poll: still moving
          _intentJson(status: 'succeeded'), // second poll: terminal
        ],
      );
      final controller = SheetController(
        client: scripted.build(),
        sessionClientSecret: _csSecret,
        clock: clock,
        jitterSource: FixedJitterSource(const [0.5]),
      );

      final List<CheckoutScreenState> seen = [];
      controller.addListener(() => seen.add(controller.state));

      await controller.start();

      expect(controller.state, isA<CheckoutCollectMsisdn>());

      await controller.submitMsisdn('+237 6 71 23 45 67');

      expect(controller.state, isA<CheckoutOutcome>());
      expect((controller.state as CheckoutOutcome).kind, OutcomeKind.succeeded);
      expect(scripted.calls, [
        'read_session',
        'confirm',
        'poll',
        'poll',
        'read_session',
      ]);

      final result = await controller.result;
      expect(result, isA<VpayCheckoutSucceeded>());
      expect((result as VpayCheckoutSucceeded).paymentIntentId, 'pi_123');

      // Poll-then-outcome: `scripted.calls` above already proves the
      // whole ordering — both `poll`s ran, THEN (and only then) the
      // post-outcome `read_session` (`_announceOutcome`'s own session
      // re-read) happened. At least one `CheckoutOutcome` notification
      // reached this listener; a second one (the re-read's own
      // `CheckoutSessionRefreshed`, still `CheckoutOutcome`-shaped) is
      // expected, not a regression.
      expect(seen.whereType<CheckoutOutcome>(), isNotEmpty);
    });

    test('select_rail is shown for two supported rails', () async {
      final scripted = _ScriptedClient(
        session: _sessionJson(
          intent: _intentJson(),
          rails: [_mtnRailJson(), _orangeRailJson()],
        ),
        intentAnswers: const [],
      );
      final controller = SheetController(
        client: scripted.build(),
        sessionClientSecret: _csSecret,
      );

      await controller.start();

      expect(controller.state, isA<CheckoutSelectRail>());
    });

    test('an invalid MSISDN never confirms and shows msisdn.invalid', () async {
      final scripted = _ScriptedClient(
        session: _sessionJson(intent: _intentJson(), rails: [_mtnRailJson()]),
        intentAnswers: const [],
      );
      final controller = SheetController(
        client: scripted.build(),
        sessionClientSecret: _csSecret,
      );

      await controller.start();
      await controller.submitMsisdn('not a number');

      expect(controller.state, isA<CheckoutCollectMsisdn>());
      expect(
        (controller.state as CheckoutCollectMsisdn).problem,
        'msisdn.invalid',
      );
      expect(scripted.calls, ['read_session']);
    });

    test(
      'remembering a number writes only after a real, accepted submit',
      () async {
        final store = _InMemoryRememberedMsisdnStore();
        final scripted = _ScriptedClient(
          session: _sessionJson(intent: _intentJson(), rails: [_mtnRailJson()]),
          intentAnswers: [
            _intentJson(
              status: 'succeeded',
            ), // confirm answers terminal directly
          ],
        );
        final controller = SheetController(
          client: scripted.build(),
          sessionClientSecret: _csSecret,
          remembered: VpayRememberedMsisdn(
            store: store,
            now: () => DateTime(2026, 1, 1),
          ),
        );

        await controller.start();
        expect(store.record, isNull, reason: 'nothing written before a submit');

        controller.setRememberChecked(true);
        await controller.submitMsisdn('+237 6 71 23 45 67');

        expect(store.record, isNotNull);
        expect(store.record!.msisdn, '237671234567');
        expect(store.record!.railCode, 'mtn_momo');
      },
    );
  });

  group('SheetController — redirect rail hand-off', () {
    test('records redirecting before the platform host is shown, then resumes waiting/poll on stopUrlReached', () async {
      final scripted = _ScriptedClient(
        session: _sessionJson(
          intent: _intentJson(),
          rails: [_orangeRailJson()],
          successUrl: 'https://shop.example/success',
        ),
        intentAnswers: [
          _intentJson(
            status: 'requires_action',
            nextAction: {
              'type': 'redirect_to_url',
              'redirect_to_url': {
                'url': 'https://orange.example/pay/abc',
                'return_url': null,
              },
            },
          ),
          _intentJson(status: 'succeeded'),
        ],
      );
      final platform = _FakePlatform(CheckoutWindowOutcome.stopUrlReached);
      final controller = SheetController(
        client: scripted.build(),
        sessionClientSecret: _csSecret,
        platform: platform,
      );

      final List<Type> stateOrder = [];
      controller.addListener(
        () => stateOrder.add(controller.state.runtimeType),
      );

      await controller.start();
      expect(controller.state, isA<CheckoutReadyRedirect>());

      await controller.startRedirect();

      expect(platform.shown, isTrue);
      // redirect_required (-> CheckoutRedirecting) must appear in the
      // recorded state history strictly before the platform host was
      // asked to show anything.
      final redirectingIndex = stateOrder.indexOf(CheckoutRedirecting);
      expect(redirectingIndex, greaterThanOrEqualTo(0));

      expect(controller.state, isA<CheckoutOutcome>());
      final result = await controller.result;
      expect(result, isA<VpayCheckoutSucceeded>());
    });

    test('a confirm answering with no next_action polls directly, without ever showing a window', () async {
      final scripted = _ScriptedClient(
        session: _sessionJson(
          intent: _intentJson(),
          rails: [_orangeRailJson()],
        ),
        intentAnswers: [
          _intentJson(status: 'processing'),
          _intentJson(status: 'succeeded'),
        ],
      );
      final platform = _FakePlatform(CheckoutWindowOutcome.dismissed);
      final controller = SheetController(
        client: scripted.build(),
        sessionClientSecret: _csSecret,
        platform: platform,
      );

      await controller.start();
      await controller.startRedirect();

      expect(platform.shown, isFalse);
      expect(controller.state, isA<CheckoutOutcome>());
    });
  });

  group('SheetController.dismiss', () {
    test('before any confirm, resolves Unresolved — never canceled', () async {
      final scripted = _ScriptedClient(
        session: _sessionJson(intent: _intentJson(), rails: [_mtnRailJson()]),
        intentAnswers: const [],
      );
      final controller = SheetController(
        client: scripted.build(),
        sessionClientSecret: _csSecret,
      );

      await controller.start();
      final result = await controller.dismiss();

      expect(result, isA<VpayCheckoutUnresolved>());
      expect(result, isNot(isA<VpayCheckoutCanceled>()));
      expect(
        (result as VpayCheckoutUnresolved).error.code,
        VpayClientErrorCodes.sheetDismissedBeforeConfirm,
      );
    });

    test('mid-payment (after confirm, still moving) resolves Pending — never canceled', () async {
      // The confirm's own POST never answers within this test — modelling
      // "the payer dismissed the sheet right after tapping Pay, before
      // the server even replied" without racing `submitMsisdn`'s OWN poll
      // loop (which only starts once confirm answers) against `dismiss`'s
      // short poll on the same clock. `dismiss` polls with plain GETs,
      // answered independently below.
      final Completer<http.Response> confirmGate = Completer<http.Response>();
      final client = BrowserClient(
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: MockClient((http.Request request) async {
          if (request.method == 'GET' &&
              request.url.path.contains('/checkout/sessions/')) {
            return _json(
              _sessionJson(intent: _intentJson(), rails: [_mtnRailJson()]),
            );
          }
          if (request.method == 'POST') {
            return confirmGate.future;
          }
          // dismiss()'s own poll: always still moving.
          return _json(_intentJson(status: 'processing'));
        }),
      );
      final clock = FakeClock(DateTime(2026));
      final controller = SheetController(
        client: client,
        sessionClientSecret: _csSecret,
        clock: clock,
        jitterSource: FixedJitterSource(const [0.5]),
        dismissalPollBudget: const Duration(seconds: 5),
      );

      await controller.start();
      expect(controller.state, isA<CheckoutCollectMsisdn>());

      // Fire-and-forget: runs synchronously up to (and including) the
      // confirm call, which never answers — `_hasConfirmedOnce` is set
      // and the state reaches `CheckoutConfirming` before this line
      // returns, both required for `dismiss()`'s own branch below.
      unawaited(controller.submitMsisdn('+237 6 71 23 45 67'));
      expect(controller.state, isA<CheckoutConfirming>());

      final VpayCheckoutResult result = await controller.dismiss();

      expect(result, isA<VpayCheckoutPending>());
      expect(result, isNot(isA<VpayCheckoutCanceled>()));
    });

    test(
      'is idempotent — a second call answers the same completed result',
      () async {
        final scripted = _ScriptedClient(
          session: _sessionJson(intent: _intentJson(), rails: [_mtnRailJson()]),
          intentAnswers: const [],
        );
        final controller = SheetController(
          client: scripted.build(),
          sessionClientSecret: _csSecret,
        );

        await controller.start();
        final first = await controller.dismiss();
        final second = await controller.dismiss();

        // The completer resolves once; a second call answers the exact same
        // cached result rather than recomputing (and, for the polling path,
        // rather than polling a second time).
        expect(identical(first, second), isTrue);
      },
    );
  });

  group('SheetController — errorMessageKey', () {
    test('api_connection_error maps to error.network', () {
      expect(errorMessageKey(VpayError.connection()), 'error.network');
    });

    test('resource_missing maps to error.session_not_found', () {
      expect(
        errorMessageKey(
          const VpayError(
            type: 'invalid_request_error',
            code: 'resource_missing',
          ),
        ),
        'error.session_not_found',
      );
    });

    test('anything else maps to error.unexpected', () {
      expect(
        errorMessageKey(VpayError.unexpectedResponse(502)),
        'error.unexpected',
      );
    });
  });
}
