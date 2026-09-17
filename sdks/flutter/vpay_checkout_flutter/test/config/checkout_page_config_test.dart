import 'dart:convert';
import 'dart:ui' show Color;

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

// `headers` names `application/json` explicitly so `http.Response.body`
// decodes with utf8 rather than defaulting to latin1 (`package:http`'s own
// `Response.body` doc comment) — load-bearing here because
// `_documentJson`'s `support_contact` fixture carries a real em dash/en
// dash, and latin1 cannot even *encode* those on the way in, let alone
// decode them back out.
http.Response _json(Object body, {int status = 200}) => http.Response(
  jsonEncode(body),
  status,
  headers: const {'content-type': 'application/json'},
);

Map<String, Object?> _documentJson({
  String? displayName = 'Vaam Payments',
  String? logoUrl,
  String? primaryColor = '#f3c623',
  String? supportContact = 'support@vaam.example — 8h–18h',
  List<Object?>? allowedMethods = const ['mtn_momo', 'orange_money'],
  int version = 1,
}) => {
  'object': 'checkout_page_config',
  'version': version,
  'branding': {
    'display_name': displayName,
    'logo_url': logoUrl,
    'primary_color': primaryColor,
    'support_contact': supportContact,
  },
  'checkout': {
    'public_base_url': null,
    'allowed_methods': allowedMethods,
    'features': {'page_memory': true},
  },
};

/// A store that answers whatever a test hands it, including throwing —
/// this file's stand-in for a merchant's own Hive/Isar/secure-storage
/// adapter, which this package cannot assume behaves well.
class _FakeStore implements VpayCheckoutConfigStore {
  _FakeStore({
    this.readValue,
    this.throwOnRead = false,
    this.throwOnWrite = false,
  });

  String? readValue;
  final bool throwOnRead;
  final bool throwOnWrite;
  String? written;

  @override
  Future<String?> read(String key) async {
    if (throwOnRead) {
      throw StateError('this store is broken');
    }
    return readValue;
  }

  @override
  Future<void> write(String key, String value) async {
    if (throwOnWrite) {
      throw StateError('this store cannot write');
    }
    written = value;
    readValue = value;
  }

  @override
  Future<void> delete(String key) async {
    readValue = null;
  }
}

void main() {
  group('CheckoutPageConfig.fromJson', () {
    test('a well-formed document parses every field', () {
      final CheckoutPageConfig config = CheckoutPageConfig.fromJson(
        _documentJson(),
      );
      expect(config.branding.displayName, 'Vaam Payments');
      expect(config.branding.supportContact, 'support@vaam.example — 8h–18h');
      expect(config.branding.primaryColor, const Color(0xFFF3C623));
      expect(config.checkout.allowedMethods, ['mtn_momo', 'orange_money']);
      expect(config.checkout.pageMemory, true);
    });

    test('null is not a document', () {
      expect(
        CheckoutPageConfig.fromJson(null),
        same(CheckoutPageConfig.defaults),
      );
    });

    test('a string is not a document', () {
      expect(
        CheckoutPageConfig.fromJson('not a map'),
        same(CheckoutPageConfig.defaults),
      );
    });

    test('the wrong object name degrades to defaults', () {
      final Map<String, Object?> json = _documentJson();
      json['object'] = 'something_else';
      expect(
        CheckoutPageConfig.fromJson(json),
        same(CheckoutPageConfig.defaults),
      );
    });

    test('an unrecognised version degrades to defaults', () {
      final Map<String, Object?> json = _documentJson(version: 2);
      expect(
        CheckoutPageConfig.fromJson(json),
        same(CheckoutPageConfig.defaults),
      );
    });

    test('every branding/checkout field missing still parses (all null)', () {
      final CheckoutPageConfig config = CheckoutPageConfig.fromJson({
        'object': 'checkout_page_config',
        'version': 1,
      });
      expect(config.branding.displayName, isNull);
      expect(config.branding.primaryColor, isNull);
      expect(config.branding.supportContact, isNull);
      expect(config.checkout.allowedMethods, isNull);
      expect(config.checkout.pageMemory, false);
    });

    test('a malformed primary_color parses to null, never throws', () {
      final CheckoutPageConfig config = CheckoutPageConfig.fromJson(
        _documentJson(primaryColor: 'not-a-color'),
      );
      expect(config.branding.primaryColor, isNull);
    });

    test('primary_color without a leading # still parses', () {
      final CheckoutPageConfig config = CheckoutPageConfig.fromJson(
        _documentJson(primaryColor: 'f3c623'),
      );
      expect(config.branding.primaryColor, const Color(0xFFF3C623));
    });

    test('allowed_methods with a non-string element degrades to null, never a partial list', () {
      final CheckoutPageConfig config = CheckoutPageConfig.fromJson(
        _documentJson(allowedMethods: ['mtn_momo', 42]),
      );
      expect(config.checkout.allowedMethods, isNull);
    });

    test('a malformed document (branding is a string, not an object) degrades to defaults for that section', () {
      final CheckoutPageConfig config = CheckoutPageConfig.fromJson({
        'object': 'checkout_page_config',
        'version': 1,
        'branding': 'not an object',
        'checkout': _documentJson()['checkout'],
      });
      expect(config.branding.displayName, isNull);
      expect(config.checkout.allowedMethods, ['mtn_momo', 'orange_money']);
    });
  });

  group('narrowAllowedMethods', () {
    test('both null — unrestricted', () {
      expect(narrowAllowedMethods(explicit: null, operatorFloor: null), isNull);
    });

    test('only the operator has an opinion — the operator floor stands', () {
      expect(
        narrowAllowedMethods(explicit: null, operatorFloor: ['mtn_momo']),
        ['mtn_momo'],
      );
    });

    test('only the caller has an opinion — the caller list stands', () {
      expect(
        narrowAllowedMethods(
          explicit: ['mtn_momo', 'orange_money'],
          operatorFloor: null,
        ),
        ['mtn_momo', 'orange_money'],
      );
    });

    test('the operator floor is narrower than the caller list — narrowed to the intersection '
        '(a merchant cannot widen past the operator floor)', () {
      expect(
        narrowAllowedMethods(
          explicit: ['mtn_momo', 'orange_money'],
          operatorFloor: ['mtn_momo'],
        ),
        ['mtn_momo'],
      );
    });

    test('the caller list is already narrower than the operator floor — the caller list stands '
        '(the operator floor never widens a merchant\'s own narrowing)', () {
      expect(
        narrowAllowedMethods(
          explicit: ['mtn_momo'],
          operatorFloor: ['mtn_momo', 'orange_money'],
        ),
        ['mtn_momo'],
      );
    });

    test('disjoint lists intersect to empty, not to either side', () {
      expect(
        narrowAllowedMethods(
          explicit: ['mtn_momo'],
          operatorFloor: ['orange_money'],
        ),
        isEmpty,
      );
    });
  });

  group('InMemoryVpayCheckoutConfigStore', () {
    test('round-trips a value', () async {
      final InMemoryVpayCheckoutConfigStore store =
          InMemoryVpayCheckoutConfigStore();
      await store.write('k', 'v');
      expect(await store.read('k'), 'v');
      await store.delete('k');
      expect(await store.read('k'), isNull);
    });
  });

  group('prepareCheckout', () {
    test(
      'a 200 with a well-formed document caches it and answers true',
      () async {
        final _FakeStore store = _FakeStore();
        late final Uri requestedUri;
        final http.Client client = MockClient((http.Request request) async {
          requestedUri = request.url;
          return _json(_documentJson());
        });

        final bool ok = await prepareCheckout(
          checkoutBaseUrl: 'https://checkout.example',
          store: store,
          httpClient: client,
        );

        expect(ok, true);
        expect(store.written, isNotNull);
        expect(requestedUri.path, '/.well-known/vpay-checkout');
      },
    );

    test('a failing connection answers false and never throws', () async {
      final _FakeStore store = _FakeStore();
      final http.Client client = MockClient(
        (http.Request request) async => throw http.ClientException('refused'),
      );

      final bool ok = await prepareCheckout(
        checkoutBaseUrl: 'https://checkout.example',
        store: store,
        httpClient: client,
      );

      expect(ok, false);
      expect(store.written, isNull);
    });

    test('a non-200 answers false and does not cache anything', () async {
      final _FakeStore store = _FakeStore();
      final http.Client client = MockClient(
        (http.Request request) async => _json({'error': 'boom'}, status: 500),
      );

      final bool ok = await prepareCheckout(
        checkoutBaseUrl: 'https://checkout.example',
        store: store,
        httpClient: client,
      );

      expect(ok, false);
      expect(store.written, isNull);
    });

    test('an unparsable body answers false, never throws', () async {
      final _FakeStore store = _FakeStore();
      final http.Client client = MockClient(
        (http.Request request) async => http.Response('not json', 200),
      );

      final bool ok = await prepareCheckout(
        checkoutBaseUrl: 'https://checkout.example',
        store: store,
        httpClient: client,
      );

      expect(ok, false);
    });

    test('a store that throws on write answers false, never throws out of prepareCheckout', () async {
      final _FakeStore store = _FakeStore(throwOnWrite: true);
      final http.Client client = MockClient(
        (http.Request request) async => _json(_documentJson()),
      );

      final bool ok = await prepareCheckout(
        checkoutBaseUrl: 'https://checkout.example',
        store: store,
        httpClient: client,
      );

      expect(ok, false);
    });
  });

  group('resolveCheckoutPageConfig', () {
    test(
      'an empty cache (no prior prepareCheckout) answers defaults',
      () async {
        final CheckoutPageConfig config = await resolveCheckoutPageConfig(
          store: _FakeStore(),
        );
        expect(config, same(CheckoutPageConfig.defaults));
      },
    );

    test(
      'a store that throws on read degrades to defaults, never throws',
      () async {
        final CheckoutPageConfig config = await resolveCheckoutPageConfig(
          store: _FakeStore(throwOnRead: true),
        );
        expect(config, same(CheckoutPageConfig.defaults));
      },
    );

    test('a store returning garbage (not JSON) degrades to defaults', () async {
      final CheckoutPageConfig config = await resolveCheckoutPageConfig(
        store: _FakeStore(readValue: 'not json at all { { {'),
      );
      expect(config, same(CheckoutPageConfig.defaults));
    });

    test('a store returning JSON that is not the expected envelope degrades to defaults', () async {
      final CheckoutPageConfig config = await resolveCheckoutPageConfig(
        store: _FakeStore(readValue: jsonEncode(['just', 'an', 'array'])),
      );
      expect(config, same(CheckoutPageConfig.defaults));
    });

    test('a fresh cache entry (just fetched) parses normally', () async {
      final _FakeStore store = _FakeStore();
      final DateTime fetchedAt = DateTime.utc(2026, 9, 17, 10);
      store.readValue = jsonEncode({
        'fetched_at': fetchedAt.toIso8601String(),
        'document': _documentJson(),
      });

      final CheckoutPageConfig config = await resolveCheckoutPageConfig(
        store: store,
        now: () => fetchedAt.add(const Duration(minutes: 5)),
      );

      expect(config.branding.displayName, 'Vaam Payments');
      expect(config.checkout.allowedMethods, ['mtn_momo', 'orange_money']);
    });

    test('an entry older than the TTL degrades to defaults', () async {
      final _FakeStore store = _FakeStore();
      final DateTime fetchedAt = DateTime.utc(2026, 9, 17, 10);
      store.readValue = jsonEncode({
        'fetched_at': fetchedAt.toIso8601String(),
        'document': _documentJson(),
      });

      final CheckoutPageConfig config = await resolveCheckoutPageConfig(
        store: store,
        ttl: const Duration(hours: 24),
        now: () => fetchedAt.add(const Duration(hours: 25)),
      );

      expect(config, same(CheckoutPageConfig.defaults));
    });

    test('an entry exactly at the TTL boundary is still trusted', () async {
      final _FakeStore store = _FakeStore();
      final DateTime fetchedAt = DateTime.utc(2026, 9, 17, 10);
      store.readValue = jsonEncode({
        'fetched_at': fetchedAt.toIso8601String(),
        'document': _documentJson(),
      });

      final CheckoutPageConfig config = await resolveCheckoutPageConfig(
        store: store,
        ttl: const Duration(hours: 24),
        now: () => fetchedAt.add(const Duration(hours: 24)),
      );

      expect(config.branding.displayName, 'Vaam Payments');
    });

    test('prepareCheckout then resolveCheckoutPageConfig round-trips through the same store', () async {
      final _FakeStore store = _FakeStore();
      final http.Client client = MockClient(
        (http.Request request) async => _json(_documentJson()),
      );

      final bool ok = await prepareCheckout(
        checkoutBaseUrl: 'https://checkout.example',
        store: store,
        httpClient: client,
      );
      expect(ok, true);

      final CheckoutPageConfig config = await resolveCheckoutPageConfig(
        store: store,
      );
      expect(config.branding.supportContact, 'support@vaam.example — 8h–18h');
      expect(config.checkout.allowedMethods, ['mtn_momo', 'orange_money']);
    });
  });
}
