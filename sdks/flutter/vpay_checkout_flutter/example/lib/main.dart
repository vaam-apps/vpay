/// A runnable example that behaves like a real merchant app.
///
/// **The payer never sees, types or pastes a session URL.** Tapping "Buy"
/// asks this app's own backend for one, exactly as a real shop's app would,
/// and hands the answer straight to [showVpayCheckoutSheet] (issue #189,
/// Lane 2 — the native Flutter checkout sheet). A session URL in a text
/// field was a testing affordance that modelled nothing real — no merchant
/// ships that, and an example that shows one teaches the wrong integration.
///
/// The invariant still holds (design doc, "The invariant"): this app does
/// **not** create the session itself. `POST /v1/checkout/sessions` needs a
/// merchant credential, and a merchant credential on a payer's device is a
/// credential that has left the merchant's control. It asks
/// [`examples/shop`](../../../../../examples/shop) — a real merchant server,
/// holding a real private key, doing a real `private_key_jwt` exchange — and
/// that server returns the session `url`.
///
/// So the flow here is the real one, end to end:
///
///   tap Buy → this app → shop's `orders.create` → vpay `POST /v1/…` → url
///           → showVpayCheckoutSheet(url) → sheet rises over this app →
///           select rail → confirm → poll → typed result
///
/// # Running it
///
/// Bring the demo stack up (`just demo-up`) and run. The defaults point at
/// it; override per build if your ports differ:
///
/// ```bash
/// flutter run \
///   --dart-define=VPAY_BASE_URL=http://localhost:8080 \
///   --dart-define=VPAY_SHOP_URL=http://localhost:3001 \
///   --dart-define=VPAY_PUBLISHABLE_KEY=pk_test_shopmerchantsandbox1
/// ```
///
/// On an Android emulator, `localhost` is the emulator. Either map the host
/// in with `adb reverse tcp:8080 tcp:8080` (and 3001), or point these at
/// `http://10.0.2.2:…`.
library;

import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

/// vpay's own API — where `VpayCheckout` reads the session and polls the
/// intent.
const String _vpayBaseUrl = String.fromEnvironment(
  'VPAY_BASE_URL',
  defaultValue: 'http://localhost:8080',
);

/// The **merchant's** server (`examples/shop`). Not vpay: this is the thing
/// a real app would call to start an order.
const String _shopUrl = String.fromEnvironment(
  'VPAY_SHOP_URL',
  defaultValue: 'http://localhost:3001',
);

/// The **hosted checkout page**'s origin — where
/// `/.well-known/vpay-checkout` lives. Not the API: this is the surface
/// that describes the deployment (issue #193), and `prepareCheckout`
/// below caches its answer so the sheet can render the operator's
/// support contact and brand colour without a network hop mid-payment.
const String _checkoutUrl = String.fromEnvironment(
  'VPAY_CHECKOUT_URL',
  defaultValue: 'http://localhost:3080',
);

const String _publishableKey = String.fromEnvironment(
  'VPAY_PUBLISHABLE_KEY',
  defaultValue: 'pk_test_shopmerchantsandbox1',
);

void main() {
  runApp(const VpayCheckoutExampleApp());
}

class VpayCheckoutExampleApp extends StatelessWidget {
  const VpayCheckoutExampleApp({super.key});

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'vpay_checkout_flutter example',
    // A real Material 3 host theme, so the example shows what a merchant
    // app actually looks like rather than Flutter's bare default. The
    // sheet inherits this: typography, density and — unless the
    // deployment publishes its own brand colour — the scheme too.
    theme: ThemeData(
      colorScheme: ColorScheme.fromSeed(seedColor: const Color(0xFF2E7D6F)),
    ),
    darkTheme: ThemeData(
      colorScheme: ColorScheme.fromSeed(
        seedColor: const Color(0xFF2E7D6F),
        brightness: Brightness.dark,
      ),
    ),
    home: const _ShopPage(),
  );
}

class _ShopPage extends StatefulWidget {
  const _ShopPage();

  @override
  State<_ShopPage> createState() => _ShopPageState();
}

class _ShopPageState extends State<_ShopPage> {
  String _status = 'Tap Buy to start a real checkout.';
  bool _running = false;

  @override
  void initState() {
    super.initState();
    // Warm the deployment-config cache once, at a moment the payer is not
    // waiting on it. Everything it fetches is presentational, so a
    // failure here costs a support line and a brand colour, never a
    // payment — which is why nothing below checks the result.
    unawaited(prepareCheckout(checkoutBaseUrl: _checkoutUrl));
  }

  /// What a real merchant app does, in order.
  Future<void> _buy() async {
    setState(() {
      _running = true;
      _status = 'Creating the order…';
    });
    try {
      // 1. Ask OUR OWN backend for an order. It holds the merchant key; this
      //    app never does. Its answer carries the session `url`.
      final String sessionUrl = await _createOrderOnOurServer();

      // 2. Open the native checkout sheet. It rises over THIS screen — the
      //    payer never leaves the app, and never sees this URL string.
      if (!mounted) {
        return;
      }
      setState(() => _status = 'Opening checkout…');
      final VpayCheckoutResult result = await showVpayCheckoutSheet(
        context,
        sessionUrl: sessionUrl,
        baseUrl: _vpayBaseUrl,
        publishableKey: _publishableKey,
        // The demo stack is plain http:// — named opt-in, never inferred
        // from a debug build.
        allowInsecureBaseUrl: true,
        merchantName: 'Njangi Store',
        // Everything here is optional — `const VpayCheckoutTheme()` is
        // the stock Material 3 sheet. This one only softens the shapes,
        // to show where the knobs are. Colour is deliberately NOT set,
        // so the demo deployment's own `primary_color` seeds the scheme
        // and you can see that path working.
        theme: const VpayCheckoutTheme(
          sheetCornerRadius: 32,
          surfaceCornerRadius: 20,
        ),
      );

      // 3. Every arm handled — the compiler insists, because
      //    `VpayCheckoutResult` is sealed.
      setState(() {
        _status = switch (result) {
          VpayCheckoutSucceeded(:final paymentIntentId) =>
            'Paid ✓\n$paymentIntentId\n\n(The shop fulfils on its webhook, '
                'not on this line.)',
          VpayCheckoutFailed(:final code, :final providerMessage) =>
            'Declined: ${code ?? 'unknown'}'
                '${providerMessage == null ? '' : '\n$providerMessage'}',
          VpayCheckoutCanceled() => 'Canceled.',
          VpayCheckoutPending() =>
            'Still settling — the shop will confirm shortly.',
          VpayCheckoutUnresolved(:final error) => 'Unresolved: ${error.code}',
        };
      });
    } on Object catch (error) {
      // Never interpolate a thrown value that could quote a URL: the session
      // URL's fragment is the session's own client_secret (D6).
      setState(() => _status = 'Could not start the checkout: $error');
    } finally {
      if (mounted) {
        setState(() => _running = false);
      }
    }
  }

  /// `examples/shop`'s real `orders.create`, over its real tRPC endpoint.
  /// This is the merchant's server doing the privileged half.
  Future<String> _createOrderOnOurServer() async {
    final http.Response response = await http.post(
      Uri.parse('$_shopUrl/api/trpc/orders.create'),
      headers: const <String, String>{'content-type': 'application/json'},
      body: jsonEncode(<String, Object?>{
        'email': 'example-app@example.test',
        'lines': <Object?>[
          <String, Object?>{'productId': 'njangi-tote', 'quantity': 1},
        ],
        'mode': 'hosted',
      }),
    );
    if (response.statusCode != 200) {
      throw StateError(
        'the shop answered ${response.statusCode} — is `just demo-up` running?',
      );
    }
    // tRPC wraps the payload as `{ result: { data: { url } } }`. Walked one
    // step at a time so a shape change fails here, with a readable error,
    // rather than as a cast blowing up somewhere else.
    final Object? decoded = jsonDecode(response.body);
    final Map<String, Object?>? envelope = decoded is Map<String, Object?>
        ? decoded['result'] as Map<String, Object?>?
        : null;
    final Map<String, Object?>? data =
        envelope?['data'] as Map<String, Object?>?;
    final Object? url = data?['url'];
    if (url is! String || url.isEmpty) {
      throw StateError('the shop returned no session url');
    }
    return url;
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('Njangi Store')),
    body: Padding(
      padding: const EdgeInsets.all(24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Text(
            'Njangi tote bag',
            style: Theme.of(context).textTheme.headlineSmall,
          ),
          const SizedBox(height: 4),
          Text(
            'FCFA 12,000',
            style: Theme.of(context).textTheme.titleMedium?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 24),
          FilledButton(
            onPressed: _running ? null : _buy,
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 12),
              child: Text(_running ? 'Working…' : 'Buy'),
            ),
          ),
          const SizedBox(height: 24),
          Text(_status),
          const Spacer(),
          Text(
            'shop: $_shopUrl\nvpay: $_vpayBaseUrl\ncheckout: $_checkoutUrl',
            style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
    ),
  );
}
