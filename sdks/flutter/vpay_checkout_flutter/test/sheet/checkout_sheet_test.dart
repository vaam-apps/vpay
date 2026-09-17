import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

const _piSecret = 'pi_123_secret_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _csSecret = 'cs_123_secret_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _sessionUrl =
    'https://checkout.example/c/cs_123?key=pk_test_1#$_csSecret';

http.Response _json(Object body, {int status = 200}) =>
    http.Response(jsonEncode(body), status);

Map<String, Object?> _intentJson({
  String status = 'requires_payment_method',
  Object? lastPaymentError,
}) => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 1234500,
  'currency': 'xaf',
  'status': status,
  'payment_method_types': ['mtn_momo', 'orange_money'],
  'next_action': null,
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

Map<String, Object?> _unsupportedRailJson() => {
  'code': 'a_future_rail_this_sdk_has_never_heard_of',
  'flow': 'push',
  'label_key': 'rail.a_future_rail',
  'fields': [
    {
      'name': 'card_number',
      'type': 'card', // decodes to RailFieldKindUnknown — unsupported.
      'required': true,
      'label_key': 'card.number',
    },
  ],
};

Map<String, Object?> _sessionJson({
  List<Object?> rails = const <Object?>[],
  bool livemode = false,
  Object? intent,
}) => {
  'id': 'cs_123',
  'object': 'checkout.session',
  'livemode': livemode,
  'payment_intent': intent ?? _intentJson(),
  'ui_mode': 'hosted',
  'status': 'open',
  'payment_status': 'unpaid',
  'success_url': null,
  'cancel_url': null,
  'return_url': null,
  'url': 'https://checkout.example/c/cs_123#$_csSecret',
  'expires_at': 1700086400,
  'created': 1700000000,
  'rails': rails,
};

Future<void> _pump(WidgetTester tester, Widget sheet) async {
  await tester.pumpWidget(MaterialApp(home: Scaffold(body: sheet)));
  await tester.pumpAndSettle();
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  SharedPreferences.setMockInitialValues(<String, Object>{});

  testWidgets(
    'renders the loading screen, then the msisdn form for one supported rail',
    (WidgetTester tester) async {
      final client = MockClient(
        (http.Request request) async =>
            _json(_sessionJson(rails: [_mtnRailJson()])),
      );

      await _pump(
        tester,
        VpayCheckoutSheet(
          sessionUrl: _sessionUrl,
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: client,
        ),
      );

      expect(find.text('MTN Mobile Money'), findsOneWidget);
      expect(find.byType(TextField), findsOneWidget);
    },
  );

  testWidgets('shows the test-mode banner when the session is not livemode', (
    WidgetTester tester,
  ) async {
    final client = MockClient(
      (http.Request request) async =>
          _json(_sessionJson(rails: [_mtnRailJson()], livemode: false)),
    );

    await _pump(
      tester,
      VpayCheckoutSheet(
        sessionUrl: _sessionUrl,
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: client,
        locale: VpayLocale.en,
      ),
    );

    expect(
      find.text('Test mode — no money moves on this deployment.'),
      findsOneWidget,
    );
  });

  testWidgets('shows no test-mode banner when the session is livemode', (
    WidgetTester tester,
  ) async {
    final client = MockClient(
      (http.Request request) async =>
          _json(_sessionJson(rails: [_mtnRailJson()], livemode: true)),
    );

    await _pump(
      tester,
      VpayCheckoutSheet(
        sessionUrl: _sessionUrl,
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: client,
        locale: VpayLocale.en,
      ),
    );

    expect(
      find.text('Test mode — no money moves on this deployment.'),
      findsNothing,
    );
  });

  testWidgets('French is the default locale — no locale argument given', (
    WidgetTester tester,
  ) async {
    final client = MockClient(
      (http.Request request) async =>
          _json(_sessionJson(rails: [_mtnRailJson()])),
    );

    await _pump(
      tester,
      VpayCheckoutSheet(
        sessionUrl: _sessionUrl,
        baseUrl: 'https://api.example',
        publishableKey: 'pk_test_1',
        httpClient: client,
      ),
    );

    // `msisdn.hint` in French, not English.
    expect(
      find.textContaining('camerounais', findRichText: true),
      findsWidgets,
    );
  });

  testWidgets(
    'select_rail lists an unsupported rail with its own code (D9) alongside the supported one',
    (WidgetTester tester) async {
      final client = MockClient(
        (http.Request request) async => _json(
          _sessionJson(
            rails: [_mtnRailJson(), _orangeRailJson(), _unsupportedRailJson()],
          ),
        ),
      );

      await _pump(
        tester,
        VpayCheckoutSheet(
          sessionUrl: _sessionUrl,
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: client,
          locale: VpayLocale.en,
        ),
      );

      // Two SUPPORTED rails (mtn, orange) -> select_rail is reached (the
      // reducer's own rule: `rails.supported.length > 1`), and the
      // unsupported third one is listed underneath, with its own code —
      // never silently dropped, never folded into a shorter list with no
      // explanation (D9).
      expect(find.text('MTN Mobile Money'), findsOneWidget);
      expect(find.text('Orange Money'), findsOneWidget);
      expect(
        find.textContaining(
          'This page cannot take a payment on a_future_rail_this_sdk_has_never_heard_of',
        ),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'a session offering only an unsupported rail refuses outright (D9)',
    (WidgetTester tester) async {
      final client = MockClient(
        (http.Request request) async =>
            _json(_sessionJson(rails: [_unsupportedRailJson()])),
      );

      await _pump(
        tester,
        VpayCheckoutSheet(
          sessionUrl: _sessionUrl,
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: client,
          locale: VpayLocale.en,
        ),
      );

      expect(
        find.text('This payment offers no payment method this page can show.'),
        findsOneWidget,
      );
    },
  );

  testWidgets(
    'an already-succeeded session renders the outcome screen with no countdown control',
    (WidgetTester tester) async {
      final client = MockClient((http.Request request) async {
        final Map<String, Object?> session = _sessionJson(
          rails: [_mtnRailJson()],
          intent: _intentJson(status: 'succeeded'),
        );
        // Session status "complete" is what the reducer's own
        // `stateForContext` checks first for the succeeded outcome.
        session['status'] = 'complete';
        return _json(session);
      });

      await _pump(
        tester,
        VpayCheckoutSheet(
          sessionUrl: _sessionUrl,
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: client,
          locale: VpayLocale.en,
        ),
      );

      expect(find.text('Payment received'), findsOneWidget);
      // Exactly one button on this screen — no countdown, no timer widget.
      expect(find.byType(ElevatedButton), findsOneWidget);
    },
  );

  testWidgets(
    "a failed outcome shows the rail's own providerReason beside the translated failure line, never instead of it",
    (WidgetTester tester) async {
      final client = MockClient(
        (http.Request request) async => _json(
          _sessionJson(
            rails: [_mtnRailJson()],
            intent: _intentJson(
              status: 'requires_payment_method',
              lastPaymentError: {
                'code': 'insufficient_funds',
                'message': 'MTN-4001: solde insuffisant',
              },
            ),
          ),
        ),
      );

      await _pump(
        tester,
        VpayCheckoutSheet(
          sessionUrl: _sessionUrl,
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: client,
          locale: VpayLocale.en,
        ),
      );

      // The translated line — never dropped in favour of the raw reason.
      expect(
        find.text('There was not enough money in the account.'),
        findsOneWidget,
      );
      // The rail's own words — shown BESIDE it, as data.
      expect(
        find.textContaining('MTN-4001: solde insuffisant'),
        findsOneWidget,
      );
    },
  );

  testWidgets('the live region is present on the very first frame', (
    WidgetTester tester,
  ) async {
    final client = MockClient(
      (http.Request request) async =>
          _json(_sessionJson(rails: [_mtnRailJson()])),
    );

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: client,
          ),
        ),
      ),
    );
    // Before the session read even resolves — the live region must already
    // be mounted, per this file's own module doc comment.
    final Iterable<Semantics> liveRegions = tester
        .widgetList<Semantics>(find.byType(Semantics))
        .where((Semantics s) => s.properties.liveRegion == true);
    expect(liveRegions, isNotEmpty);
    await tester.pumpAndSettle();
  });
}
