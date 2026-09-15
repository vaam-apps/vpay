/// Lane D — a real end-to-end test of `vpay_checkout_flutter`'s Dart core
/// against a RUNNING vpay stack. No `MockClient`, and no fixture standing in
/// for the wire, anywhere in this file's path: every HTTP call below is a
/// real `package:http` request to a real `vpay-server`.
///
/// **Kept OUT of `test/` on purpose.** `flutter test` with no arguments only
/// discovers `test/`, so this file never runs as part of the plain,
/// stack-independent unit suite (`just test-flutter`), which must stay green
/// — 80 passed, 0 skipped — with no stack at all. This file needs
/// `just test-flutter-e2e`, which checks the stack is up FIRST and refuses
/// loudly, never a skip, the moment it is not
/// (`docs/plans/2026-09-14-flutter-e2e-real-stack.md`).
///
/// # The fixture, and what minted it
///
/// `just test-flutter-e2e` places a JSON file at `$VPAY_E2E_FIXTURE_FILE`
/// before running this suite. It is built by minting TWO real Checkout
/// Sessions through `examples/shop`'s own running server — a real
/// `POST /v1/payment_intents` followed by a real `POST
/// /v1/checkout/sessions`, authenticated with a real `private_key_jwt`
/// `client_credentials` exchange, exactly the two calls
/// `examples/shop/src/server/orders.ts` (`:273`, `:295`) makes for a paying
/// customer — and, for the second session only, expiring it with
/// `POST /v1/checkout/sessions/{id}/expire` under a merchant access token the
/// recipe mints the same way. **Nothing here is a merchant credential this
/// package would ever hold**: `examples/shop`'s server does the signing, this
/// file only ever receives a session `url`, exactly what a merchant's own
/// backend would hand a payer's device.
///
/// # What this proves, and what it deliberately does not
///
/// - Proves the plugin's own [BrowserClient] and [CheckoutController] reading
///   and polling a real, running `vpay-server` end to end: a real session
///   read, a real pre-flight, a real poll to a real terminal outcome — and
///   that outcome is re-checked against the server directly afterwards
///   (twice: once more through [BrowserClient], once with no package code at
///   all), never trusted from the package's own return value alone.
/// - **Since 2026-09-15, also proves the MERCHANT learned — not just that
///   the payer's device believes it succeeded.** Every assertion above this
///   point, and every other test in this package (82 unit tests, the two
///   other groups in this file, all three emulator suites), stops at the
///   payer-side read: `GET /v1/browser/payment_intents/{id}`, which is
///   exactly the route the plugin's own poll already used. The README's
///   claim that `VpayCheckoutResult.succeeded` is a UI fact and a merchant's
///   server must still act on `payment_intent.succeeded` was, until this
///   group, untested. The success fixture is a REAL order through
///   `examples/shop`'s own server (`orders.create` — nothing new there); what
///   is new is that `just test-flutter-e2e` now also captures that order's
///   id and this file polls `examples/shop`'s `orders.get` — a SEPARATE
///   process, over HTTP, under no package code — for it to flip to `paid`.
///   That field is written by exactly one thing in the whole stack:
///   `examples/shop/src/server/webhook.ts`, after it verifies a real
///   `vpay-signature` HMAC on a real `POST /api/vpay/webhook` delivery from
///   `vpay-worker`. This is tier 1 of the three the brief ranked (the shop's
///   own database, matching `docs/flows/hosted-checkout.md`'s own bar and
///   `frontends/tests/e2e/cypress/support/shop.ts`'s `readOrder` for
///   Cypress) — not a webhook-journal read and not the merchant event feed,
///   because the shop's order row was already the strongest evidence this
///   deployment can produce and reaching for a second tier once tier 1 holds
///   would prove nothing further.
/// - Does **not** prove anything about a real payment rail. This deployment's
///   rail is WireMock, exactly as it is for every other test in this
///   repository (`docs/status.md`'s banner: no HTTP call to a real rail has
///   ever been made) — this file's payment settles the same way
///   `examples/merchant-demo`'s own outcome table documents: 12 000 XAF
///   matches no amount-keyed WireMock mapping, so `vpay-worker`'s status poll
///   answers `PENDING` once and the catch-all `SUCCESS` on the next rung.
/// - Does **not** drive the platform window (`VpayCheckout.start`), which
///   needs an actual `WebView`/browser this repository has no way to
///   automate from a Dart test process. That is `pigeons/checkout.dart`
///   territory and stays unproven here, exactly as `docs/sdks/parity.md`'s
///   dated ⛔ rows already say.
/// - **The one HTTP call not on [BrowserClient]: the confirm.** The package
///   has no `confirm` method by design — D3 of
///   `docs/plans/2026-09-13-flutter-plugin.md` is that vpay's own hosted
///   PAGE submits the confirm, never this package, and no JavaScript bridge
///   exists for it to do otherwise. So `_rawConfirm` below makes that one
///   `POST /v1/browser/payment_intents/{id}/confirm` directly with
///   `package:http`, standing in for what a payer's browser submits on that
///   page. Every other request in this file goes through the package's own
///   [BrowserClient].
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

/// What `just test-flutter-e2e` writes and this suite reads. Failing to find
/// it is a loud [fail] in `setUpAll`, never a skip — see this file's header.
class _Fixture {
  const _Fixture({
    required this.baseUrl,
    required this.publishableKey,
    required this.successSessionUrl,
    required this.expiredSessionUrl,
    required this.expiredIntentId,
    required this.expiredIntentClientSecret,
    required this.shopUrl,
    required this.successOrderId,
  });

  /// vpay's own API origin, e.g. `http://localhost:8080` — what
  /// [BrowserClient.baseUrl] is constructed with. Not the checkout page's
  /// origin, which is a different port in the demo stack.
  final String baseUrl;

  /// `shop-merchant`'s publishable key, read out of the container that
  /// minted these sessions. Public by name; authorises nothing.
  final String publishableKey;

  /// A fresh, still-`open` hosted session's `url`
  /// (`{base}/c/{cs_id}?key=...#{cs_secret}`) — the success path.
  final String successSessionUrl;

  /// A second session's `url`, expired by the recipe with a real merchant
  /// token **after** [expiredIntentClientSecret] was captured from it.
  final String expiredSessionUrl;

  final String expiredIntentId;

  /// The expired session's intent's own `client_secret`, read while the
  /// session was still `open` — D2 item 1's polling credential, captured
  /// before expiry precisely so this suite can still use it afterwards to
  /// prove the intent read outlives the session while the confirm does not.
  final String expiredIntentClientSecret;

  /// `examples/shop`'s own origin, e.g. `http://localhost:3001` — a
  /// DIFFERENT server from [baseUrl] (vpay itself), and the whole point of
  /// the merchant-side assertion below: it is asked of the merchant, not of
  /// vpay.
  final String shopUrl;

  /// The shop's own database id for the order behind [successSessionUrl] —
  /// what `orders.create` answered when `just test-flutter-e2e` minted it.
  /// Never touched by anything in this package; used only to ask the shop's
  /// own `orders.get` whether ITS row says the order is paid.
  final String successOrderId;

  static _Fixture load() {
    final String? path = Platform.environment['VPAY_E2E_FIXTURE_FILE'];
    if (path == null || path.trim().isEmpty) {
      fail(
        'VPAY_E2E_FIXTURE_FILE is not set. real_stack_e2e_test.dart is a '
        'REAL end-to-end test and refuses to run against nothing — it never '
        "falls back to a mock and it never skips. Run it through 'just "
        "test-flutter-e2e', which mints this fixture against a running "
        'vpay stack and sets this variable — see the justfile.',
      );
    }
    final File file = File(path);
    if (!file.existsSync()) {
      fail('VPAY_E2E_FIXTURE_FILE names $path, and that file does not exist.');
    }
    final Object? decoded = jsonDecode(file.readAsStringSync());
    if (decoded is! Map) {
      fail('$path did not decode to a JSON object.');
    }
    final Map<String, Object?> json = decoded.cast();
    String field(String name) {
      final Object? value = json[name];
      if (value is! String || value.isEmpty) {
        fail('$path is missing a non-empty "$name" field.');
      }
      return value;
    }

    return _Fixture(
      baseUrl: field('baseUrl'),
      publishableKey: field('publishableKey'),
      successSessionUrl: field('successSessionUrl'),
      expiredSessionUrl: field('expiredSessionUrl'),
      expiredIntentId: field('expiredIntentId'),
      expiredIntentClientSecret: field('expiredIntentClientSecret'),
      shopUrl: field('shopUrl'),
      successOrderId: field('successOrderId'),
    );
  }
}

/// `{base}/c/{cs_id}?key={pk}#{cs_secret}` (D6) split into its three parts —
/// this file's own copy of the parse `VpayCheckout.start` does internally
/// (`lib/src/vpay_checkout.dart`'s private `_SessionUrl.parse`), kept
/// separate because this file drives [BrowserClient]/[CheckoutController]
/// directly rather than going through `VpayCheckout.start`, which needs a
/// platform window this suite has no way to open.
class _ParsedSessionUrl {
  const _ParsedSessionUrl({
    required this.checkoutSessionId,
    required this.publishableKey,
    required this.clientSecret,
  });

  final String checkoutSessionId;
  final String publishableKey;
  final String clientSecret;

  static _ParsedSessionUrl parse(String sessionUrl) {
    final int hash = sessionUrl.indexOf('#');
    if (hash == -1 || hash == sessionUrl.length - 1) {
      fail('fixture session URL carries no fragment secret: $sessionUrl');
    }
    final Uri withoutFragment = Uri.parse(sessionUrl.substring(0, hash));
    final String clientSecret = sessionUrl.substring(hash + 1);
    final List<String> segments = withoutFragment.pathSegments;
    if (segments.length < 2 || segments[segments.length - 2] != 'c') {
      fail('fixture session URL is not "{base}/c/{id}...": $sessionUrl');
    }
    final String? key = withoutFragment.queryParameters['key'];
    if (key == null || key.isEmpty) {
      fail('fixture session URL carries no ?key=...: $sessionUrl');
    }
    return _ParsedSessionUrl(
      checkoutSessionId: segments.last,
      publishableKey: key,
      clientSecret: clientSecret,
    );
  }
}

/// `POST /v1/browser/payment_intents/{id}/confirm` — see this file's header
/// for why it is not a call on [BrowserClient].
///
/// **Builds the body by hand rather than handing `http.Client.post` a
/// `Map<String, String>`.** vpay's form grammar
/// (`backends/crates/vpay-api/src/form.rs`) splits a key on its **raw,
/// still-escaped** `[`/`]` — the brackets are structural wire syntax, never
/// percent-encoded, exactly like `sdks/nodejs/src/form.ts`'s own encoder.
/// `http.Client.post`'s map-body helper percent-encodes the KEY too
/// (`payment_method_data%5Btype%5D`), which vpay's decoder does not
/// recognise as a bracket at all — measured against this real server: it
/// answers `400 payment_method_data[type]` on that encoding, not the 200
/// curl's own `--data-urlencode name=value` (which never escapes the name)
/// gets.
Future<http.Response> _rawConfirm(
  http.Client client, {
  required String baseUrl,
  required String paymentIntentId,
  required String key,
  required String clientSecret,
  required String paymentMethodType,
}) {
  final Uri uri = Uri.parse(
    '$baseUrl/v1/browser/payment_intents/$paymentIntentId/confirm',
  );
  final String body = [
    'key=${Uri.encodeQueryComponent(key)}',
    'client_secret=${Uri.encodeQueryComponent(clientSecret)}',
    'payment_method_data[type]=${Uri.encodeQueryComponent(paymentMethodType)}',
  ].join('&');
  return client.post(
    uri,
    headers: const {'Content-Type': 'application/x-www-form-urlencoded'},
    body: body,
  );
}

/// `GET {shopUrl}/api/trpc/orders.get?input=...` — the shop's OWN read of
/// its OWN database, exactly as its return page polls it and exactly as
/// `frontends/tests/e2e/cypress/support/shop.ts`'s `readOrder` does for
/// Cypress. No batching, no transformer, and — the whole point — no package
/// code: this is a bare HTTP GET against `examples/shop`, a server this
/// package has never heard of and never sent a byte to anywhere else in this
/// file.
Future<String> _readShopOrderStatus(
  http.Client client, {
  required String shopUrl,
  required String orderId,
}) async {
  final Uri uri = Uri.parse('$shopUrl/api/trpc/orders.get').replace(
    queryParameters: {
      'input': jsonEncode(<String, String>{'id': orderId}),
    },
  );
  final http.Response response = await client.get(uri);
  if (response.statusCode != 200) {
    fail(
      "the shop's own orders.get answered HTTP ${response.statusCode} for "
      'order $orderId: ${response.body}',
    );
  }
  final Object? decoded = jsonDecode(response.body);
  if (decoded is! Map) {
    fail(
      "the shop's orders.get did not answer a JSON object: ${response.body}",
    );
  }
  final Map<String, Object?> body = decoded.cast();
  final Object? result = body['result'];
  if (result is! Map) {
    fail('the shop\'s orders.get answered no "result": ${response.body}');
  }
  final Map<String, Object?> resultMap = result.cast();
  final Object? data = resultMap['data'];
  if (data is! Map) {
    fail('the shop\'s orders.get answered no "result.data": ${response.body}');
  }
  final Map<String, Object?> orderMap = data.cast();
  final Object? status = orderMap['status'];
  if (status is! String) {
    fail('the shop\'s orders.get answered no "status": ${response.body}');
  }
  return status;
}

/// Polls [_readShopOrderStatus] until it reaches [expected] or [timeout]
/// elapses. The wait exists for one real thing, not for network jitter:
/// `vpay-worker`'s fan-out is a timed job loop, not a callback this test can
/// hook, and the delivery it is waiting for is the same delivery
/// `examples/shop/src/server/webhook.ts` verifies before writing anything.
Future<String> _waitForShopOrderStatus(
  http.Client client, {
  required String shopUrl,
  required String orderId,
  required String expected,
  Duration timeout = const Duration(seconds: 60),
  Duration interval = const Duration(seconds: 2),
}) async {
  final DateTime deadline = DateTime.now().add(timeout);
  String last = 'unpaid';
  while (true) {
    last = await _readShopOrderStatus(
      client,
      shopUrl: shopUrl,
      orderId: orderId,
    );
    if (last == expected) {
      return last;
    }
    if (DateTime.now().isAfter(deadline)) {
      fail(
        "the shop's own order $orderId stayed '$last' for "
        "${timeout.inSeconds}s; expected '$expected' — the merchant never "
        'independently learned this payment happened, even though the '
        "payer's own poll did",
      );
    }
    await Future<void>.delayed(interval);
  }
}

void main() {
  late final _Fixture fixture;

  setUpAll(() {
    fixture = _Fixture.load();
  });

  group('real stack — BrowserClient + CheckoutController end to end', () {
    test(
      'mints, preflights, confirms and polls a real session to succeeded',
      () async {
        final _ParsedSessionUrl parsed = _ParsedSessionUrl.parse(
          fixture.successSessionUrl,
        );
        final http.Client httpClient = http.Client();
        addTearDown(httpClient.close);

        final BrowserClient browserClient = BrowserClient(
          baseUrl: fixture.baseUrl,
          publishableKey: parsed.publishableKey,
          httpClient: httpClient,
          // The demo stack this fixture was minted against is plain HTTP
          // (`compose.demo.yml`) — the exact, named opt-in `baseUrl`'s own
          // doc comment describes, never inferred from a debug build.
          allowInsecureBaseUrl: true,
        );
        final CheckoutController controller = CheckoutController(
          client: browserClient,
        );

        // 1. Real session read + pre-flight:
        //    GET /v1/browser/checkout/sessions/{id}.
        final CheckoutPreflight preflight = await controller.preflight(
          parsed.clientSecret,
        );
        if (preflight is CheckoutPreflightFailure) {
          fail('preflight refused a fresh, open session: ${preflight.error}');
        }
        final CheckoutPreflightReady ready =
            (preflight as CheckoutPreflightSuccess).ready;

        stdout.writeln(
          '[real_stack_e2e] preflight ok — session ${ready.sessionId}, '
          'intent ${ready.paymentIntentId}',
        );
        expect(ready.sessionId, startsWith('cs_'));
        expect(ready.paymentIntentId, startsWith('pi_'));
        expect(ready.sessionId, parsed.checkoutSessionId);

        // 2. Real confirm. `orange_money` is a redirect rail, but with an
        //    open checkout session vpay's confirm handler uses the
        //    session's OWN return page and needs no `return_url` from the
        //    caller (`vpay_api::v1::payment_intents::payer_instrument`), so
        //    the type is all this form needs.
        final http.Response confirmResponse = await _rawConfirm(
          httpClient,
          baseUrl: fixture.baseUrl,
          paymentIntentId: ready.paymentIntentId,
          key: parsed.publishableKey,
          clientSecret: ready.intentClientSecret,
          paymentMethodType: 'orange_money',
        );
        expect(
          confirmResponse.statusCode,
          200,
          reason: 'confirm answered: ${confirmResponse.body}',
        );
        final Map<String, Object?> confirmed =
            (jsonDecode(confirmResponse.body) as Map).cast();
        expect(confirmed['status'], 'requires_action');
        stdout.writeln(
          '[real_stack_e2e] confirmed pi ${ready.paymentIntentId} — status '
          'requires_action, next_action present: '
          '${confirmed['next_action'] != null}',
        );

        // 3. Real poll — the package's own code — to a real terminal
        //    outcome. See this file's header for why this settles with no
        //    browser: the amount matches WireMock's catch-all mapping.
        final VpayCheckoutResult result = await controller
            .resolveAfterStopUrlReached(
              ready,
              timeout: const Duration(seconds: 90),
              interval: const Duration(seconds: 2),
            );
        stdout.writeln('[real_stack_e2e] resolved -> $result');
        expect(
          result,
          isA<VpayCheckoutSucceeded>(),
          reason:
              'the poll loop did not observe a succeeded intent within the '
              'budget; see the log above for what it actually resolved to',
        );

        // 4. Real state, re-checked independently — never the client's own
        //    return value alone.
        final PaymentIntentResult reread = await browserClient
            .retrievePaymentIntent(ready.intentClientSecret);
        expect(reread.isError, isFalse, reason: '${reread.error}');
        expect(reread.paymentIntent!.status, PaymentIntentStatus.succeeded);

        // And once more with NO package code at all: a bare HTTP GET, so a
        // bug in BrowserClient's own JSON decoding could not paper over a
        // payment that never actually happened.
        final http.Response rawState = await httpClient.get(
          Uri.parse(
            '${fixture.baseUrl}/v1/browser/payment_intents/'
            '${ready.paymentIntentId}',
          ).replace(
            queryParameters: {
              'key': parsed.publishableKey,
              'client_secret': ready.intentClientSecret,
            },
          ),
        );
        expect(rawState.statusCode, 200);
        final Map<String, Object?> rawBody = (jsonDecode(rawState.body) as Map)
            .cast();
        expect(rawBody['status'], 'succeeded');
        stdout.writeln(
          '[real_stack_e2e] raw HTTP confirms it too: pi '
          '${ready.paymentIntentId} is succeeded on the server',
        );

        // 5. THE MERCHANT'S OWN EVIDENCE — a SECOND, INDEPENDENT observer.
        //    Everything above this line, in this test and in every other
        //    test this package has, is a payer-side read: the plugin's own
        //    poll, then the same route read twice more. This step asks a
        //    DIFFERENT process — examples/shop, over HTTP, under no package
        //    code — whether ITS OWN database says the order is paid. That
        //    field is written by exactly one thing in the stack:
        //    examples/shop/src/server/webhook.ts, and only after it verifies
        //    a real vpay-signature HMAC on a real POST /api/vpay/webhook
        //    delivery from vpay-worker. If this assertion passes, the
        //    README's claim — that VpayCheckoutResult.succeeded is a UI fact
        //    and a merchant's own server must still act on
        //    payment_intent.succeeded — is proven, not merely stated.
        final String shopStatus = await _waitForShopOrderStatus(
          httpClient,
          shopUrl: fixture.shopUrl,
          orderId: fixture.successOrderId,
          expected: 'paid',
        );
        expect(
          shopStatus,
          'paid',
          reason:
              "examples/shop's own orders.get for order "
              '${fixture.successOrderId} did not reach paid',
        );
        stdout.writeln(
          '[real_stack_e2e] MERCHANT-SIDE EVIDENCE: examples/shop order '
          '${fixture.successOrderId} is paid in the SHOP\'S OWN database — '
          'written only by its webhook handler after a real, '
          'signature-verified payment_intent.succeeded delivery for pi '
          '${ready.paymentIntentId}. The merchant, not just the payer\'s '
          'device, learned this payment happened.',
        );
      },
      timeout: const Timeout(Duration(minutes: 3)),
    );
  });

  group('real stack — the uniform 404', () {
    test(
      'an unknown id, a wrong secret and a wrong key all answer the same 404',
      () async {
        final http.Client httpClient = http.Client();
        addTearDown(httpClient.close);
        final _ParsedSessionUrl real = _ParsedSessionUrl.parse(
          fixture.successSessionUrl,
        );

        final BrowserClient client = BrowserClient(
          baseUrl: fixture.baseUrl,
          publishableKey: real.publishableKey,
          httpClient: httpClient,
          allowInsecureBaseUrl: true,
        );
        final CheckoutSessionResult unknownId = await client
            .retrieveCheckoutSession(
              'cs_doesnotexist00000000_secret_'
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            );
        final CheckoutSessionResult wrongSecret = await client
            .retrieveCheckoutSession(
              '${real.checkoutSessionId}_secret_'
              'wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww',
            );
        final BrowserClient wrongKeyClient = BrowserClient(
          baseUrl: fixture.baseUrl,
          publishableKey: 'pk_test_doesnotexist00000000000',
          httpClient: httpClient,
          allowInsecureBaseUrl: true,
        );
        final CheckoutSessionResult wrongKey = await wrongKeyClient
            .retrieveCheckoutSession(real.clientSecret);

        for (final CheckoutSessionResult result in [
          unknownId,
          wrongSecret,
          wrongKey,
        ]) {
          expect(result.isError, isTrue);
          expect(result.error!.type, 'invalid_request_error');
          expect(result.error!.code, 'resource_missing');
        }
        stdout.writeln(
          '[real_stack_e2e] unknown id / wrong secret / wrong key all -> '
          'the same 404 (resource_missing), against the real server',
        );
      },
    );
  });

  group('real stack — a session that is not open refuses the confirm', () {
    test('the intent read still answers, the pre-flight fails closed, and the confirm is refused with checkout_session_expired', () async {
      final http.Client httpClient = http.Client();
      addTearDown(httpClient.close);
      final _ParsedSessionUrl parsed = _ParsedSessionUrl.parse(
        fixture.expiredSessionUrl,
      );
      final BrowserClient client = BrowserClient(
        baseUrl: fixture.baseUrl,
        publishableKey: parsed.publishableKey,
        httpClient: httpClient,
        allowInsecureBaseUrl: true,
      );

      // D2 item 1: the intent's OWN read outlives the session. Proven
      // with the secret this file's fixture captured BEFORE the recipe
      // expired the session below.
      final PaymentIntentResult stillReadable = await client
          .retrievePaymentIntent(fixture.expiredIntentClientSecret);
      expect(stillReadable.isError, isFalse, reason: '${stillReadable.error}');
      expect(
        stillReadable.paymentIntent!.status,
        PaymentIntentStatus.requiresPaymentMethod,
        reason: 'this intent was never confirmed',
      );

      // But the session's OWN read no longer carries the intent's
      // client_secret once it is not `open`
      // (`vpay_api::browser::checkout_sessions`), so the plugin's own
      // pre-flight must fail CLOSED rather than hand back a stale
      // credential it invented.
      final CheckoutController controller = CheckoutController(client: client);
      final CheckoutPreflight preflight = await controller.preflight(
        parsed.clientSecret,
      );
      expect(
        preflight,
        isA<CheckoutPreflightFailure>(),
        reason:
            'an expired session omits payment_intent.client_secret; '
            'preflight must not paper over that with a stale value',
      );

      // And the confirm itself — the same call vpay's own hosted page
      // would submit — is refused with the server's own named error, not
      // a generic 400.
      final http.Response confirmResponse = await _rawConfirm(
        httpClient,
        baseUrl: fixture.baseUrl,
        paymentIntentId: fixture.expiredIntentId,
        key: parsed.publishableKey,
        clientSecret: fixture.expiredIntentClientSecret,
        paymentMethodType: 'orange_money',
      );
      expect(confirmResponse.statusCode, 409);
      final Map<String, Object?> body =
          (jsonDecode(confirmResponse.body) as Map).cast();
      final Map<String, Object?> error = (body['error']! as Map).cast();
      expect(error['code'], 'checkout_session_expired');
      stdout.writeln(
        '[real_stack_e2e] confirm on an expired session -> 409 '
        'checkout_session_expired (real, against the running server)',
      );
    });
  });
}
