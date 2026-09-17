/// "Remember this number on this device" — the Dart port of
/// `frontends/apps/checkout/src/lib/memory.ts`'s job, backed by
/// `shared_preferences` rather than IndexedDB (there is no browser storage
/// on a native app; `shared_preferences` is this platform's equivalent
/// key-value store, and it is device-local exactly the way IndexedDB was —
/// never synced to an account, never sent anywhere by this package).
///
/// **No PIN is ever stored, on this device or any other — never.** Only the
/// canonical MSISDN ([normalizeCameroonMsisdn]'s own output) and the moment
/// it was written.
///
/// **Opt-in, and written only on a real submit.** [VpayRememberedMsisdn.remember]
/// is never called from a keystroke handler — only from the same place a
/// payer's Pay button fires a confirm, so ticking the box and then abandoning
/// the form writes nothing.
///
/// **A 90-day TTL, enforced on *read*, not on write.** A record is never
/// proactively deleted the day it turns 90 days old; [VpayRememberedMsisdn.read]
/// simply stops returning it once [DateTime.now] is far enough past
/// [RememberedMsisdnRecord.rememberedAt] — `memory.ts`'s own rule, so a
/// stale record left on a device that is never opened again just becomes
/// permanently unreadable rather than needing a background job to clean it
/// up.
library;

import 'dart:convert';

import 'package:shared_preferences/shared_preferences.dart';

/// `memory.ts`'s own 90-day window, in [Duration] form.
const Duration rememberedMsisdnTtl = Duration(days: 90);

/// The key under which this package's own record lives —
/// namespaced so this SDK never reads or clobbers a merchant app's own
/// `shared_preferences` key.
const String _prefsKey = 'vpay_checkout_flutter.remembered_msisdn.v1';

/// One remembered number, and when it was written.
final class RememberedMsisdnRecord {
  const RememberedMsisdnRecord({
    required this.msisdn,
    required this.railCode,
    required this.rememberedAt,
  });

  /// The canonical form [normalizeCameroonMsisdn]
  /// (`sheet/msisdn.dart`) produced — never whatever a payer typed.
  final String msisdn;

  /// The rail this number was submitted against, e.g. `mtn_momo`. Carried
  /// so a future rail with its own remembered field does not collide with
  /// this one under the same key.
  final String railCode;

  final DateTime rememberedAt;

  bool isExpired(DateTime now, {Duration ttl = rememberedMsisdnTtl}) =>
      now.difference(rememberedAt) > ttl;

  Map<String, Object?> toJson() => <String, Object?>{
    'msisdn': msisdn,
    'rail_code': railCode,
    'remembered_at': rememberedAt.toUtc().toIso8601String(),
  };

  static RememberedMsisdnRecord? fromJson(Object? json) {
    if (json is! Map) {
      return null;
    }
    final Object? msisdn = json['msisdn'];
    final Object? railCode = json['rail_code'];
    final Object? rememberedAt = json['remembered_at'];
    if (msisdn is! String || railCode is! String || rememberedAt is! String) {
      return null;
    }
    final DateTime? parsed = DateTime.tryParse(rememberedAt);
    if (parsed == null) {
      return null;
    }
    return RememberedMsisdnRecord(
      msisdn: msisdn,
      railCode: railCode,
      rememberedAt: parsed,
    );
  }
}

/// The store this sheet reads and writes through — an interface so a widget
/// test can substitute an in-memory fake rather than touching real device
/// storage, the same reason [BrowserClient] takes an injectable `http.Client`.
abstract class VpayRememberedMsisdnStore {
  Future<RememberedMsisdnRecord?> read();
  Future<void> write(RememberedMsisdnRecord record);
  Future<void> clear();
}

/// The real store — `shared_preferences`, one JSON-encoded string under
/// [_prefsKey].
final class SharedPreferencesRememberedMsisdnStore
    implements VpayRememberedMsisdnStore {
  const SharedPreferencesRememberedMsisdnStore();

  @override
  Future<RememberedMsisdnRecord?> read() async {
    final SharedPreferences prefs = await SharedPreferences.getInstance();
    final String? raw = prefs.getString(_prefsKey);
    if (raw == null) {
      return null;
    }
    try {
      return RememberedMsisdnRecord.fromJson(jsonDecode(raw));
    } on FormatException {
      return null;
    }
  }

  @override
  Future<void> write(RememberedMsisdnRecord record) async {
    final SharedPreferences prefs = await SharedPreferences.getInstance();
    await prefs.setString(_prefsKey, jsonEncode(record.toJson()));
  }

  @override
  Future<void> clear() async {
    final SharedPreferences prefs = await SharedPreferences.getInstance();
    await prefs.remove(_prefsKey);
  }
}

/// The whole feature, on top of a [VpayRememberedMsisdnStore]: TTL
/// enforcement on read, and the "forget" affordance.
final class VpayRememberedMsisdn {
  const VpayRememberedMsisdn({
    this.store = const SharedPreferencesRememberedMsisdnStore(),
    DateTime Function()? now,
  }) : _now = now ?? DateTime.now;

  final VpayRememberedMsisdnStore store;
  final DateTime Function() _now;

  /// The remembered number for [railCode], or `null` when there is none or
  /// it is past [rememberedMsisdnTtl] — a stale record is treated exactly
  /// like no record, never surfaced as "this device once knew a number".
  Future<String?> read(String railCode) async {
    final RememberedMsisdnRecord? record = await store.read();
    if (record == null || record.railCode != railCode) {
      return null;
    }
    if (record.isExpired(_now())) {
      return null;
    }
    return record.msisdn;
  }

  /// Whether this device is currently holding *any* record — used to decide
  /// whether to offer "Forget what this device remembers" at all, per
  /// `memory.ts`'s `hasRecord`. A record past its TTL still counts as
  /// present here (there is something to forget, even though [read] will
  /// not return it) — mirroring `memory.ts`'s own choice to let a payer
  /// clear stale state explicitly rather than only offer the button once a
  /// background sweep has run.
  Future<bool> hasRecord() async => (await store.read()) != null;

  /// Writes [msisdn] against [railCode] with the current time —
  /// called **only** from a real submit (this file's own doc comment).
  Future<void> remember({required String msisdn, required String railCode}) =>
      store.write(
        RememberedMsisdnRecord(
          msisdn: msisdn,
          railCode: railCode,
          rememberedAt: _now(),
        ),
      );

  Future<void> forget() => store.clear();
}
