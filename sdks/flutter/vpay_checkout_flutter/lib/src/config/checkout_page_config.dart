/// Deployment-config discovery — issue #193, the SDK half.
///
/// `GET {checkoutBaseUrl}/.well-known/vpay-checkout` serves one JSON
/// document describing how *this* deployment wants to look:
/// `frontends/apps/checkout/src/config/public-document.ts` is its wire
/// contract, restated here for a Dart caller. [prepareCheckout] fetches it
/// once and caches it; [resolveCheckoutPageConfig] reads the cache back.
///
/// # THE RULE THAT GOVERNS EVERYTHING HERE
///
/// **This document may only affect presentation. It may never affect
/// whether or how a payment can happen.** Everything payment-critical
/// already arrives per-session from
/// `GET /v1/browser/checkout/sessions/{id}` — rails, payer fields, region,
/// phone type, flow — adapter-declared and server-enforced. Two
/// consequences, enforced by construction rather than only documented:
///
/// 1. **[prepareCheckout] is an optimisation, never a prerequisite.** A
///    merchant who never calls it, or whose payer has no network, still
///    gets a working checkout: every read in this file degrades to
///    [CheckoutPageConfig.defaults] on any failure — a network error, a
///    non-200, a malformed body, a store that throws, a stale cache — and
///    never throws into a caller's payment flow.
/// 2. **No validation ever comes from this document.** Nothing in this
///    file's model is read by `rails.dart`'s own supported/unsupported
///    split; [narrowAllowedMethods] only ever *removes* a rail code from
///    what a merchant already asked for, and the server independently
///    re-validates the session's own rails regardless of what this SDK
///    cached.
library;

import 'dart:async';
import 'dart:convert';
import 'dart:ui' show Color;

import 'package:http/http.dart' as http;

// ---------------------------------------------------------------------------
// The model — every field nullable/absent-safe, matching
// `public-document.ts`'s own "a field may be added but not silently
// repurposed" promise from the other side: an old SDK reading a document a
// newer deployment served just sees `null` for anything it predates.
// ---------------------------------------------------------------------------

/// `public-document.ts`'s `branding` object.
final class CheckoutPageBranding {
  const CheckoutPageBranding({
    this.displayName,
    this.logoUrl,
    this.primaryColor,
    this.supportContact,
  });

  /// Nothing carries a value here — this file's own tolerant parsing
  /// (`_asNonEmptyString`) never distinguishes "absent" from "blank".
  static const CheckoutPageBranding empty = CheckoutPageBranding();

  final String? displayName;

  /// Parsed but **never loaded into an `Image` by this package** — this
  /// library's own module doc comment, and the checkout sheet's own
  /// (`lib/src/sheet/checkout_sheet.dart`): loading a remote image into a
  /// payment sheet is a separate risk conversation this issue does not
  /// open. A caller building its own branded surface may still read it.
  final String? logoUrl;

  /// Parsed from `#rrggbb` (a leading `#` is optional on read). Anything
  /// else — wrong length, non-hex digits, not a string at all — parses to
  /// `null` rather than throwing a [FormatException] into a payment flow.
  ///
  /// **Not wired into [VpayCheckoutSheet]'s own rendering.** That widget's
  /// module doc comment states a locked-in contract — "every colour comes
  /// from `Theme.of(context)`" — so a payer's checkout always matches the
  /// host app's own theme rather than an operator's. Overriding that would
  /// be a real theme system, which this issue explicitly asks not to
  /// invent; a caller that wants this colour applies it to its own
  /// `ThemeData` before opening the sheet, exactly as it would any other
  /// brand colour it already owns.
  final Color? primaryColor;

  /// `branding.support_contact` — free text (an email, a phone number, an
  /// "email / phone" pair), never parsed into a `mailto:`/`tel:` link here,
  /// mirroring `screens.tsx`'s own `SupportLine`.
  final String? supportContact;

  factory CheckoutPageBranding.fromJson(Object? json) {
    if (json is! Map) {
      return CheckoutPageBranding.empty;
    }
    return CheckoutPageBranding(
      displayName: _asNonEmptyString(json['display_name']),
      logoUrl: _asNonEmptyString(json['logo_url']),
      primaryColor: _asColor(json['primary_color']),
      supportContact: _asNonEmptyString(json['support_contact']),
    );
  }
}

/// `public-document.ts`'s `checkout` object.
final class CheckoutPageCheckoutSettings {
  const CheckoutPageCheckoutSettings({
    this.publicBaseUrl,
    this.allowedMethods,
    this.pageMemory = false,
  });

  static const CheckoutPageCheckoutSettings empty =
      CheckoutPageCheckoutSettings();

  final String? publicBaseUrl;

  /// The operator's own narrowing of `payment_method_types` — a **floor**,
  /// never a widening: `null` is "no opinion", which is not the same as an
  /// empty list (`public-document.ts`'s own doc comment, restated for this
  /// SDK). See [narrowAllowedMethods] for how a caller-supplied allow-list
  /// combines with this one.
  final List<String>? allowedMethods;

  final bool pageMemory;

  factory CheckoutPageCheckoutSettings.fromJson(Object? json) {
    if (json is! Map) {
      return CheckoutPageCheckoutSettings.empty;
    }
    final Object? features = json['features'];
    final bool pageMemory = features is Map && features['page_memory'] == true;
    return CheckoutPageCheckoutSettings(
      publicBaseUrl: _asNonEmptyString(json['public_base_url']),
      allowedMethods: _asStringList(json['allowed_methods']),
      pageMemory: pageMemory,
    );
  }
}

/// The whole document — `public-document.ts`'s `CheckoutPageConfigDocument`,
/// restated. Construct only through [CheckoutPageConfig.fromJson]; the
/// public constructor exists for [defaults] and for tests that build a
/// fixture directly.
final class CheckoutPageConfig {
  const CheckoutPageConfig({
    this.branding = CheckoutPageBranding.empty,
    this.checkout = CheckoutPageCheckoutSettings.empty,
  });

  /// The compiled-in default — every field absent, exactly as if this
  /// document had never been fetched. What every entry point in this file
  /// degrades to on a network failure, a non-200, a malformed body, an
  /// unrecognised `version`, or a store that misbehaves (this file's own
  /// module doc comment, rule 1).
  static const CheckoutPageConfig defaults = CheckoutPageConfig();

  final CheckoutPageBranding branding;
  final CheckoutPageCheckoutSettings checkout;

  /// Tolerant top to bottom: a `json` that is not a `Map`, whose `object`
  /// is not `"checkout_page_config"`, or whose `version` is not `1`
  /// answers [defaults] rather than reading a shape this SDK does not
  /// recognise (the `version` field's own reason for existing,
  /// `public-document.ts`'s doc comment: "a promise... it can change
  /// without stranding an installed app").
  factory CheckoutPageConfig.fromJson(Object? json) {
    if (json is! Map ||
        json['object'] != 'checkout_page_config' ||
        json['version'] != 1) {
      return CheckoutPageConfig.defaults;
    }
    return CheckoutPageConfig(
      branding: CheckoutPageBranding.fromJson(json['branding']),
      checkout: CheckoutPageCheckoutSettings.fromJson(json['checkout']),
    );
  }
}

String? _asNonEmptyString(Object? value) =>
    value is String && value.isNotEmpty ? value : null;

List<String>? _asStringList(Object? value) {
  if (value is! List) {
    return null;
  }
  final List<String> result = <String>[];
  for (final Object? item in value) {
    if (item is! String) {
      // One non-string element makes the whole list untrustworthy —
      // never a partially-parsed allow-list silently missing an entry.
      return null;
    }
    result.add(item);
  }
  return List<String>.unmodifiable(result);
}

Color? _asColor(Object? value) {
  if (value is! String) {
    return null;
  }
  String hex = value.trim();
  if (hex.startsWith('#')) {
    hex = hex.substring(1);
  }
  if (hex.length != 6) {
    return null;
  }
  final int? channel = int.tryParse(hex, radix: 16);
  if (channel == null) {
    return null;
  }
  return Color(0xFF000000 | channel);
}

// ---------------------------------------------------------------------------
// Narrowing — the merge rule for `allowedMethods`.
// ---------------------------------------------------------------------------

/// Combines a caller's own `allowedMethods` with
/// [CheckoutPageCheckoutSettings.allowedMethods] (the operator's floor),
/// per issue #193: the operator's list is a floor a caller may narrow but
/// never widen — the same semantics
/// `frontends/apps/checkout/src/lib/rails.ts`'s `railChoices` enforces
/// server-side, and this package's own `sheet/rails.dart` already enforces
/// against a session's rails. This function is the one place that combines
/// the two *inputs* to that existing narrowing, so `rails.dart` itself
/// needed no change.
///
/// - both `null` ("no opinion" from either side) → `null` (unrestricted).
/// - only one side has an opinion → that one, unchanged.
/// - both have an opinion → their **intersection**, in [explicit]'s order —
///   so a caller's own list can only ever lose a code the operator's floor
///   already excluded, never regain one the operator excluded, and never
///   gain a code the caller itself never named.
List<String>? narrowAllowedMethods({
  required List<String>? explicit,
  required List<String>? operatorFloor,
}) {
  if (operatorFloor == null) {
    return explicit;
  }
  if (explicit == null) {
    return operatorFloor;
  }
  return explicit.where(operatorFloor.contains).toList(growable: false);
}

// ---------------------------------------------------------------------------
// Storage — a small pluggable seam, not a storage API of this package's own
// design.
// ---------------------------------------------------------------------------

/// Where [prepareCheckout] keeps its answer. Deliberately three methods and
/// plain strings: anything wider would be a storage API this package has
/// no business designing — a merchant already using Hive, Isar, secure
/// storage, or its own store supplies an adapter over that, rather than
/// this package forcing a storage plugin choice on every merchant the way
/// a single hard-coded `shared_preferences` dependency would.
///
/// **A misbehaving store degrades exactly like no cache at all.**
/// [resolveCheckoutPageConfig] catches anything [read] throws and treats a
/// non-JSON or otherwise unreadable value the same as a miss — this file's
/// rule 1, extended to a store this package did not write itself.
abstract interface class VpayCheckoutConfigStore {
  Future<String?> read(String key);
  Future<void> write(String key, String value);
  Future<void> delete(String key);
}

/// The default store — an in-memory `Map`, process-lifetime only.
///
/// **This is not a limitation for [prepareCheckout]'s own use case, it is
/// the right semantics.** A merchant calls it once at app start specifically
/// to warm the sheet's *first* read before a payer ever taps anything; a
/// process-lifetime cache gets the whole benefit of that (no fetch in front
/// of the payer's first tap) with none of the staleness a persisted copy
/// would carry across a relaunch, and no dependency this package would
/// otherwise have to pick on a merchant's behalf. Persistence only earns
/// its keep for a merchant that is itself offline-first — and that
/// merchant supplies its own [VpayCheckoutConfigStore] (Hive, Isar, its own
/// key-value layer) rather than this package choosing one for it.
final class InMemoryVpayCheckoutConfigStore implements VpayCheckoutConfigStore {
  InMemoryVpayCheckoutConfigStore();

  final Map<String, String> _values = <String, String>{};

  @override
  Future<String?> read(String key) async => _values[key];

  @override
  Future<void> write(String key, String value) async {
    _values[key] = value;
  }

  @override
  Future<void> delete(String key) async {
    _values.remove(key);
  }
}

/// The package-level default [VpayCheckoutConfigStore], used whenever a
/// caller passes none to either [prepareCheckout] or
/// [resolveCheckoutPageConfig] — a single shared instance so the
/// zero-configuration path (call [prepareCheckout] once at app start, call
/// [resolveCheckoutPageConfig] — or open a sheet — many times after,
/// passing a store to neither) actually shares one cache, rather than two
/// stores that never see each other.
final VpayCheckoutConfigStore defaultCheckoutPageConfigStore =
    InMemoryVpayCheckoutConfigStore();

const String _cacheKey = 'vpay_checkout_flutter.checkout_page_config.v1';

/// How long a cached document is trusted before [resolveCheckoutPageConfig]
/// treats it as a miss and answers [CheckoutPageConfig.defaults] instead.
///
/// **24 hours.** Every field this document carries — a brand colour, a
/// display name, a support contact, an operator's rail narrowing — changes
/// only on a deliberate operator redeploy, not continuously, so this is not
/// tuned against how fast the *data* moves. It is tuned against two
/// opposing costs: too short, and a long-lived process (a desktop or web
/// tab left open for days, or a mobile app a payer never force-quits)
/// re-reads a document that cannot have changed on every cold read; too
/// long, and a redeployed operator narrowing takes unreasonably long to
/// reach an app that was already running when it shipped. A day sits well
/// inside "the same business day" for the second cost while all but
/// eliminating the first — and because this document is presentation-only
/// (this file's own governing rule), a stale day-old cache is never more
/// than a cosmetic lag, never a correctness gap.
const Duration checkoutPageConfigTtl = Duration(hours: 24);

/// Fetches `{checkoutBaseUrl}/.well-known/vpay-checkout` and caches it in
/// [store] (or [defaultCheckoutPageConfigStore]) for [resolveCheckoutPageConfig]
/// to read back later.
///
/// **An optimisation, never a prerequisite** (this file's own module doc
/// comment) — every failure path here is swallowed and answered as `false`,
/// never a thrown [Future]: a merchant that awaits this and ignores the
/// result loses nothing but the head start it would have given the sheet's
/// first read.
///
/// Returns `true` only once a plausibly-shaped document (a JSON object
/// whose `object` is `"checkout_page_config"`) was written to [store].
/// Whether that document's *contents* are usable is [resolveCheckoutPageConfig]'s
/// question, asked again on every read — this function does not pre-judge
/// it beyond "this is worth caching at all".
Future<bool> prepareCheckout({
  required String checkoutBaseUrl,
  VpayCheckoutConfigStore? store,
  http.Client? httpClient,
}) async {
  final VpayCheckoutConfigStore effectiveStore =
      store ?? defaultCheckoutPageConfigStore;
  final http.Client client = httpClient ?? http.Client();
  final bool ownsClient = httpClient == null;
  try {
    final Uri? uri = _wellKnownUri(checkoutBaseUrl);
    if (uri == null) {
      return false;
    }
    final http.Response response;
    try {
      response = await client.get(uri);
    } on Object {
      // A refused connection, no network at all, a timeout the underlying
      // `http.Client` surfaced as an exception — all the same answer here.
      return false;
    }
    if (response.statusCode != 200) {
      return false;
    }
    final Object? decoded;
    try {
      decoded = jsonDecode(response.body);
    } on FormatException {
      return false;
    }
    if (decoded is! Map || decoded['object'] != 'checkout_page_config') {
      return false;
    }
    final String envelope = jsonEncode(<String, Object?>{
      'fetched_at': DateTime.now().toUtc().toIso8601String(),
      'document': decoded,
    });
    try {
      await effectiveStore.write(_cacheKey, envelope);
    } on Object {
      // A merchant-supplied store that cannot write is no different from
      // one that was never asked — the fetch itself succeeded, but there is
      // nothing to report as cached.
      return false;
    }
    return true;
  } finally {
    if (ownsClient) {
      client.close();
    }
  }
}

Uri? _wellKnownUri(String checkoutBaseUrl) {
  String base = checkoutBaseUrl.trim();
  while (base.endsWith('/')) {
    base = base.substring(0, base.length - 1);
  }
  if (base.isEmpty) {
    return null;
  }
  final Uri? parsed = Uri.tryParse('$base/.well-known/vpay-checkout');
  if (parsed == null || !parsed.hasScheme) {
    return null;
  }
  return parsed;
}

/// Reads [store] (or [defaultCheckoutPageConfigStore]) back, honouring
/// [ttl] against the `fetched_at` [prepareCheckout] wrote alongside the
/// document — the TTL is enforced here, on read, exactly the way
/// `remember_msisdn.dart`'s own 90-day window is: nothing proactively
/// evicts a stale entry, this function simply stops trusting it once
/// [now] is far enough past `fetched_at`.
///
/// Every failure this function can encounter — no entry, a store that
/// throws, an unreadable envelope, a missing or unparsable `fetched_at`, a
/// cache older than [ttl], or a `document` [CheckoutPageConfig.fromJson]
/// itself cannot make sense of — answers [CheckoutPageConfig.defaults].
/// Never a thrown [Future]: this is the read [VpayCheckoutSheet] makes on
/// every open, and this file's rule 1 applies to it exactly as it does to
/// [prepareCheckout].
Future<CheckoutPageConfig> resolveCheckoutPageConfig({
  VpayCheckoutConfigStore? store,
  Duration ttl = checkoutPageConfigTtl,
  DateTime Function() now = DateTime.now,
}) async {
  final VpayCheckoutConfigStore effectiveStore =
      store ?? defaultCheckoutPageConfigStore;
  final String? raw;
  try {
    raw = await effectiveStore.read(_cacheKey);
  } on Object {
    return CheckoutPageConfig.defaults;
  }
  if (raw == null) {
    return CheckoutPageConfig.defaults;
  }
  final Object? envelope;
  try {
    envelope = jsonDecode(raw);
  } on FormatException {
    return CheckoutPageConfig.defaults;
  }
  if (envelope is! Map) {
    return CheckoutPageConfig.defaults;
  }
  final Object? fetchedAtRaw = envelope['fetched_at'];
  final DateTime? fetchedAt = fetchedAtRaw is String
      ? DateTime.tryParse(fetchedAtRaw)
      : null;
  if (fetchedAt == null) {
    return CheckoutPageConfig.defaults;
  }
  if (now().toUtc().difference(fetchedAt.toUtc()) > ttl) {
    return CheckoutPageConfig.defaults;
  }
  return CheckoutPageConfig.fromJson(envelope['document']);
}
