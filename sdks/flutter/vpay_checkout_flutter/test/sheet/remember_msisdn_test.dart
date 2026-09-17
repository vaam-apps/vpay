import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

class _InMemoryStore implements VpayRememberedMsisdnStore {
  RememberedMsisdnRecord? record;

  @override
  Future<void> clear() async => record = null;

  @override
  Future<RememberedMsisdnRecord?> read() async => record;

  @override
  Future<void> write(RememberedMsisdnRecord r) async => record = r;
}

void main() {
  group('VpayRememberedMsisdn', () {
    test('reads nothing when nothing was written', () async {
      final feature = VpayRememberedMsisdn(store: _InMemoryStore());
      expect(await feature.read('mtn_momo'), isNull);
      expect(await feature.hasRecord(), isFalse);
    });

    test(
      'read returns exactly what was remembered, for the matching rail',
      () async {
        final store = _InMemoryStore();
        final feature = VpayRememberedMsisdn(
          store: store,
          now: () => DateTime(2026, 1, 1),
        );

        await feature.remember(msisdn: '237671234567', railCode: 'mtn_momo');

        expect(await feature.read('mtn_momo'), '237671234567');
        expect(await feature.hasRecord(), isTrue);
      },
    );

    test(
      'read answers null for a different rail than the one remembered',
      () async {
        final store = _InMemoryStore();
        final feature = VpayRememberedMsisdn(
          store: store,
          now: () => DateTime(2026, 1, 1),
        );

        await feature.remember(msisdn: '237671234567', railCode: 'mtn_momo');

        expect(await feature.read('a_future_push_rail'), isNull);
      },
    );

    test('a record exactly at the 90-day boundary is still readable', () async {
      final store = _InMemoryStore();
      final DateTime writtenAt = DateTime(2026, 1, 1);
      DateTime now = writtenAt;
      final feature = VpayRememberedMsisdn(store: store, now: () => now);
      await feature.remember(msisdn: '237671234567', railCode: 'mtn_momo');

      now = writtenAt.add(rememberedMsisdnTtl);

      expect(await feature.read('mtn_momo'), '237671234567');
    });

    test(
      'a record one microsecond past the 90-day TTL is no longer readable',
      () async {
        final store = _InMemoryStore();
        final DateTime writtenAt = DateTime(2026, 1, 1);
        DateTime now = writtenAt;
        final feature = VpayRememberedMsisdn(store: store, now: () => now);
        await feature.remember(msisdn: '237671234567', railCode: 'mtn_momo');

        now = writtenAt.add(
          rememberedMsisdnTtl + const Duration(microseconds: 1),
        );

        expect(await feature.read('mtn_momo'), isNull);
        // The TTL is enforced on READ, not by deleting the record — it is
        // still "present" for `hasRecord`'s own purpose (offering "Forget").
        expect(await feature.hasRecord(), isTrue);
      },
    );

    test('forget clears the record entirely', () async {
      final store = _InMemoryStore();
      final feature = VpayRememberedMsisdn(
        store: store,
        now: () => DateTime(2026, 1, 1),
      );
      await feature.remember(msisdn: '237671234567', railCode: 'mtn_momo');

      await feature.forget();

      expect(await feature.read('mtn_momo'), isNull);
      expect(await feature.hasRecord(), isFalse);
    });

    test('no PIN field exists anywhere on the record — RememberedMsisdnRecord carries only msisdn, railCode and rememberedAt', () {
      final record = RememberedMsisdnRecord(
        msisdn: '237671234567',
        railCode: 'mtn_momo',
        rememberedAt: DateTime(2026, 1, 1),
      );
      expect(record.toJson().keys.toSet(), <String>{
        'msisdn',
        'rail_code',
        'remembered_at',
      });
    });
  });

  group('RememberedMsisdnRecord round-trip', () {
    test('toJson/fromJson round-trips exactly', () {
      final original = RememberedMsisdnRecord(
        msisdn: '237671234567',
        railCode: 'mtn_momo',
        rememberedAt: DateTime.utc(2026, 3, 4, 5, 6, 7),
      );
      final RememberedMsisdnRecord? restored = RememberedMsisdnRecord.fromJson(
        original.toJson(),
      );
      expect(restored, isNotNull);
      expect(restored!.msisdn, original.msisdn);
      expect(restored.railCode, original.railCode);
      expect(restored.rememberedAt, original.rememberedAt);
    });

    test('fromJson answers null for malformed input rather than throwing', () {
      expect(RememberedMsisdnRecord.fromJson(null), isNull);
      expect(RememberedMsisdnRecord.fromJson('not a map'), isNull);
      expect(RememberedMsisdnRecord.fromJson(<String, Object?>{}), isNull);
      expect(
        RememberedMsisdnRecord.fromJson(<String, Object?>{
          'msisdn': '237671234567',
          'rail_code': 'mtn_momo',
          'remembered_at': 'not a date',
        }),
        isNull,
      );
    });
  });
}
