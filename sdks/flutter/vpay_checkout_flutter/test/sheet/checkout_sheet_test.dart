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
  Object? nextAction,
}) => {
  'id': 'pi_123',
  'object': 'payment_intent',
  'amount': 1234500,
  'currency': 'xaf',
  'status': status,
  'payment_method_types': ['mtn_momo', 'orange_money'],
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
      //
      // Any Material button family, via `ButtonStyleButton`, their shared
      // base. This assertion is about the *count* — "one control, and
      // nothing counting down beside it" — and pinning `ElevatedButton`
      // made it fail for the unrelated reason that the M3 pass moved this
      // CTA to `FilledButton`. Written this way it also catches a stray
      // `TextButton` or `OutlinedButton`, which the old form missed.
      //
      // `byWidgetPredicate`, not `byType`: `find.byType` compares
      // `runtimeType` exactly and so matches no subclass at all — a
      // `byType(ButtonStyleButton)` here finds zero widgets, not one.
      expect(
        find.byWidgetPredicate((Widget w) => w is ButtonStyleButton),
        findsOneWidget,
      );
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

  group('issue #193 — deployment-config discovery', () {
    // A dedicated `_json` that names `application/json` explicitly, so a
    // non-ASCII fixture (an em dash in a support contact, say) round-trips
    // through `http.Response.body` as utf8 rather than the package's own
    // latin1 default for headerless responses.
    http.Response configJson(Object body, {int status = 200}) => http.Response(
      jsonEncode(body),
      status,
      headers: const {'content-type': 'application/json'},
    );

    Map<String, Object?> documentJson({
      String? supportContact = 'support@vaam.example',
      List<Object?>? allowedMethods,
    }) => {
      'object': 'checkout_page_config',
      'version': 1,
      'branding': {
        'display_name': 'Vaam Payments',
        'logo_url': null,
        'primary_color': '#f3c623',
        'support_contact': supportContact,
      },
      'checkout': {
        'public_base_url': null,
        'allowed_methods': allowedMethods,
        'features': {'page_memory': true},
      },
    };

    testWidgets(
      'an empty cache and a MockClient that fails every config fetch — the checkout still works',
      (WidgetTester tester) async {
        final InMemoryVpayCheckoutConfigStore store =
            InMemoryVpayCheckoutConfigStore();

        // `prepareCheckout` is an optimisation a merchant may call, and here
        // it fails outright — no network at all for the config endpoint.
        final http.Client failingConfigClient = MockClient(
          (http.Request request) async =>
              throw http.ClientException('no network'),
        );
        final bool prepared = await prepareCheckout(
          checkoutBaseUrl: 'https://checkout.example',
          store: store,
          httpClient: failingConfigClient,
        );
        expect(prepared, false);
        // The cache is still empty — `resolveCheckoutPageConfig` must
        // degrade to defaults, not throw, when the sheet reads it.
        final CheckoutPageConfig config = await resolveCheckoutPageConfig(
          store: store,
        );
        expect(config, same(CheckoutPageConfig.defaults));

        // The sheet itself never touches the config endpoint — only the
        // session/intent one, which works normally here — so a payer can
        // still pay with nothing cached and no way to fetch it.
        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            configStore: store,
          ),
        );

        expect(find.text('MTN Mobile Money'), findsOneWidget);
        expect(find.byType(TextField), findsOneWidget);
      },
    );

    testWidgets(
      'allowed_methods: the operator floor narrows a wider caller list',
      (WidgetTester tester) async {
        final InMemoryVpayCheckoutConfigStore store =
            InMemoryVpayCheckoutConfigStore();
        final http.Client configClient = MockClient(
          (http.Request request) async =>
              configJson(documentJson(allowedMethods: ['mtn_momo'])),
        );
        expect(
          await prepareCheckout(
            checkoutBaseUrl: 'https://checkout.example',
            store: store,
            httpClient: configClient,
          ),
          true,
        );

        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson(), _orangeRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            configStore: store,
            // The caller asks for both — wider than the operator's floor.
            allowedMethods: const ['mtn_momo', 'orange_money'],
          ),
        );

        // Narrowed to the one rail the operator's floor allows — the
        // session's own single-supported-rail path (same as the plain
        // "one supported rail" test above), never the two-rail selector.
        expect(find.text('MTN Mobile Money'), findsOneWidget);
        expect(find.byType(TextField), findsOneWidget);
        expect(find.text('Orange Money'), findsNothing);
      },
    );

    testWidgets(
      'allowed_methods: a caller list already narrower than the operator floor is not widened back',
      (WidgetTester tester) async {
        final InMemoryVpayCheckoutConfigStore store =
            InMemoryVpayCheckoutConfigStore();
        final http.Client configClient = MockClient(
          (http.Request request) async => configJson(
            // The operator's own floor is permissive — both rails.
            documentJson(allowedMethods: ['mtn_momo', 'orange_money']),
          ),
        );
        expect(
          await prepareCheckout(
            checkoutBaseUrl: 'https://checkout.example',
            store: store,
            httpClient: configClient,
          ),
          true,
        );

        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson(), _orangeRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            configStore: store,
            // The caller's own narrowing is stricter than the operator's
            // floor — the operator's wider list must not widen it back.
            allowedMethods: const ['mtn_momo'],
          ),
        );

        expect(find.text('MTN Mobile Money'), findsOneWidget);
        expect(find.byType(TextField), findsOneWidget);
        expect(find.text('Orange Money'), findsNothing);
      },
    );

    testWidgets(
      'a malformed cached document degrades to defaults — the sheet still works, unrestricted',
      (WidgetTester tester) async {
        final InMemoryVpayCheckoutConfigStore store =
            InMemoryVpayCheckoutConfigStore();
        // Passes `prepareCheckout`'s own coarse shape check (it is a map
        // naming the right `object`) but `allowed_methods` is not a list —
        // `CheckoutPageConfig.fromJson` must degrade just that field to
        // `null`, not throw and not corrupt the rest of the sheet.
        final http.Client configClient = MockClient(
          (http.Request request) async => configJson({
            'object': 'checkout_page_config',
            'version': 1,
            'branding': {'support_contact': 'support@vaam.example'},
            'checkout': {'allowed_methods': 'mtn_momo, orange_money'},
          }),
        );
        expect(
          await prepareCheckout(
            checkoutBaseUrl: 'https://checkout.example',
            store: store,
            httpClient: configClient,
          ),
          true,
        );

        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson(), _orangeRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            configStore: store,
            locale: VpayLocale.en,
          ),
        );

        // `allowed_methods` degraded to "no opinion" — both rails still
        // offered, so the sheet shows the selector rather than a single
        // pre-picked rail.
        expect(find.text('Choose how you want to pay'), findsOneWidget);
        expect(find.text('MTN Mobile Money'), findsOneWidget);
        expect(find.text('Orange Money'), findsOneWidget);
        // `support_contact` still parsed fine — one malformed field never
        // takes the rest of the document down with it.
        expect(find.textContaining('support@vaam.example'), findsOneWidget);
      },
    );

    testWidgets('support_contact renders through page.support once cached', (
      WidgetTester tester,
    ) async {
      final InMemoryVpayCheckoutConfigStore store =
          InMemoryVpayCheckoutConfigStore();
      final http.Client configClient = MockClient(
        (http.Request request) async =>
            configJson(documentJson(supportContact: 'support@vaam.example')),
      );
      expect(
        await prepareCheckout(
          checkoutBaseUrl: 'https://checkout.example',
          store: store,
          httpClient: configClient,
        ),
        true,
      );

      final http.Client sessionClient = MockClient(
        (http.Request request) async =>
            _json(_sessionJson(rails: [_mtnRailJson()])),
      );

      await _pump(
        tester,
        VpayCheckoutSheet(
          sessionUrl: _sessionUrl,
          baseUrl: 'https://api.example',
          publishableKey: 'pk_test_1',
          httpClient: sessionClient,
          configStore: store,
          locale: VpayLocale.en,
        ),
      );

      expect(find.text('Support: support@vaam.example'), findsOneWidget);
    });

    /// The end of the wire issue #193 left disconnected: a deployment
    /// publishes `primary_color`, and the sheet is actually painted with
    /// it. Asserting on the *rendered* theme rather than on the parsed
    /// config, because the parse already had a test and the rendering is
    /// the part that was missing.
    testWidgets(
      'primary_color from the cached document seeds the rendered M3 scheme',
      (WidgetTester tester) async {
        final InMemoryVpayCheckoutConfigStore store =
            InMemoryVpayCheckoutConfigStore();
        final http.Client configClient = MockClient(
          (http.Request request) async => configJson(documentJson()),
        );
        expect(
          await prepareCheckout(
            checkoutBaseUrl: 'https://checkout.example',
            store: store,
            httpClient: configClient,
          ),
          true,
        );

        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            configStore: store,
            locale: VpayLocale.en,
          ),
        );

        // Read the theme from INSIDE the sheet — the sheet installs its
        // own `Theme` below the host's, so an element above it would
        // report the host scheme and pass no matter what this does.
        final ColorScheme rendered = Theme.of(
          tester.element(find.byType(TextField)),
        ).colorScheme;
        final ColorScheme expected = ColorScheme.fromSeed(
          // `#f3c623`, the colour `documentJson` publishes.
          seedColor: const Color(0xFFF3C623),
          brightness: Brightness.light,
        );
        expect(rendered, expected);
        // And it is genuinely different from what the host app alone
        // would have given, so the assertion above cannot pass vacuously.
        expect(rendered.primary, isNot(const ColorScheme.light().primary));
      },
    );

    testWidgets(
      'useDeploymentBrandColor: false keeps the host app scheme even when a colour is published',
      (WidgetTester tester) async {
        final InMemoryVpayCheckoutConfigStore store =
            InMemoryVpayCheckoutConfigStore();
        final http.Client configClient = MockClient(
          (http.Request request) async => configJson(documentJson()),
        );
        expect(
          await prepareCheckout(
            checkoutBaseUrl: 'https://checkout.example',
            store: store,
            httpClient: configClient,
          ),
          true,
        );

        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            configStore: store,
            locale: VpayLocale.en,
            theme: const VpayCheckoutTheme(useDeploymentBrandColor: false),
          ),
        );

        final BuildContext inside = tester.element(find.byType(TextField));
        expect(Theme.of(inside).colorScheme, ThemeData().colorScheme);
      },
    );

    /// The native half of the `requires_action` fix (the web landed in
    /// #199). A payer who opened Orange's page and came back without
    /// finishing must not be shown the push rail's "check your phone".
    testWidgets(
      'a requires_action intent renders the resume screen — no spinner, and '
      'a way back to the rail',
      (WidgetTester tester) async {
        final http.Client sessionClient = MockClient(
          (http.Request request) async => _json(
            _sessionJson(
              rails: [_mtnRailJson(), _orangeRailJson()],
              intent: _intentJson(
                status: 'requires_action',
                nextAction: <String, Object?>{
                  'type': 'redirect_to_url',
                  'redirect_to_url': <String, Object?>{
                    'url': 'https://rail.example/hosted/tok_abc',
                    'return_url': null,
                  },
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
            httpClient: sessionClient,
            locale: VpayLocale.en,
          ),
        );

        expect(find.text('Payment not completed'), findsOneWidget);
        expect(find.text('Return to the payment page'), findsOneWidget);
        // The thing that was wrong: the push rail's copy, on a redirect
        // rail the payer abandoned.
        expect(find.text('Check your phone'), findsNothing);
        // And no spinner — the status behind this screen only the payer can
        // move, so anything suggesting progress would be a lie.
        expect(find.byType(CircularProgressIndicator), findsNothing);
        // Two rails are on offer, so the secondary way out is there too.
        expect(find.text('Choose another payment method'), findsOneWidget);
      },
    );

    testWidgets(
      'no configStore given, prepareCheckout not called — no support line, unrestricted rails',
      (WidgetTester tester) async {
        final http.Client sessionClient = MockClient(
          (http.Request request) async =>
              _json(_sessionJson(rails: [_mtnRailJson(), _orangeRailJson()])),
        );

        await _pump(
          tester,
          VpayCheckoutSheet(
            sessionUrl: _sessionUrl,
            baseUrl: 'https://api.example',
            publishableKey: 'pk_test_1',
            httpClient: sessionClient,
            locale: VpayLocale.en,
          ),
        );

        expect(find.text('Choose how you want to pay'), findsOneWidget);
        expect(find.text('MTN Mobile Money'), findsOneWidget);
        expect(find.text('Orange Money'), findsOneWidget);
        expect(find.textContaining('Support:'), findsNothing);
      },
    );
  });
}
