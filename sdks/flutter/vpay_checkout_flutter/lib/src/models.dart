/// Wire and API types for vpay's `/v1/browser` surface — the Dart port of
/// `sdks/stripe-js/src/types.ts`'s object shapes, narrowed to what this
/// package's read-only, payer-side use needs.
///
/// D6 governs every `toString()` here: a type holding a session URL, a
/// session secret or an intent secret overrides it and renders
/// `[N chars redacted]` for that member — see `src/redaction.dart`.
library;

import 'redaction.dart';

/// `vpay_core::state::IntentStatus` (`docs/flows/payment-lifecycle.md`).
/// There is deliberately no `failed` value — a rail failure returns the
/// intent to `requiresPaymentMethod` with [LastPaymentError] populated.
enum PaymentIntentStatus {
  requiresPaymentMethod,
  requiresAction,
  processing,
  succeeded,
  canceled,

  /// A status this package does not recognise — a future server value an
  /// older client should not crash on. Never produced server-side today;
  /// carried so a poll loop keeps polling and then answers
  /// `VpayCheckoutPending` when its budget elapses, rather than throwing.
  /// (This comment said `VpayCheckoutUnresolved` until 2026-09-14 and was
  /// wrong: `hasStoppedMoving` is `false` here, so `_resolve` never reaches
  /// its mapping with an unknown status and the budget decides.)
  unknown;

  static PaymentIntentStatus fromWire(String value) => switch (value) {
    'requires_payment_method' => PaymentIntentStatus.requiresPaymentMethod,
    'requires_action' => PaymentIntentStatus.requiresAction,
    'processing' => PaymentIntentStatus.processing,
    'succeeded' => PaymentIntentStatus.succeeded,
    'canceled' => PaymentIntentStatus.canceled,
    _ => PaymentIntentStatus.unknown,
  };
}

/// `vpay_core::failure`'s closed vocabulary
/// (`docs/flows/failures.md`) — kept as the raw wire string rather than a
/// closed Dart enum: `sdks/stripe-js`'s own `FailureCode` union is typed
/// against the same eleven values, but a Dart `enum` a future twelfth code
/// does not fit would make an old client's poll loop throw where the
/// TypeScript equivalent just carries a new string through. The eleven
/// documented values are asserted by `test/models_test.dart`'s fixtures
/// rather than enforced by the type itself.
typedef FailureCode = String;

/// A rail failure attached to a `requires_payment_method` intent — present
/// only then; there is no `failed` [PaymentIntentStatus].
final class LastPaymentError {
  const LastPaymentError({required this.code, required this.message});

  final FailureCode code;

  /// The rail's own words. Carried as data, not translated here — the same
  /// treatment the hosted checkout page gives it
  /// (`frontends/apps/checkout/src/lib/failures.ts`'s `providerReason`).
  final String message;

  factory LastPaymentError.fromJson(Map<String, Object?> json) =>
      LastPaymentError(
        code: json['code']! as String,
        message: json['message']! as String,
      );
}

/// `vpay_api::model::PaymentIntentWithSecret`: the twelve keys of
/// `PaymentIntentObject` plus the `client_secret` the browser surface
/// flattens in alongside them (D2) — thirteen keys total, the same count
/// `sdks/stripe-js`'s parity row for this type names.
final class PaymentIntent {
  const PaymentIntent({
    required this.id,
    required this.amount,
    required this.currency,
    required this.status,
    required this.paymentMethodTypes,
    required this.lastPaymentError,
    required this.metadata,
    required this.description,
    required this.created,
    required this.livemode,
    required this.clientSecret,
  });

  /// `pi_…`.
  final String id;

  /// Integer minor units. XAF is zero-decimal: `5000` means 5,000 FCFA.
  final int amount;

  /// Lowercase ISO 4217 code, e.g. `xaf`.
  final String currency;

  final PaymentIntentStatus status;

  final List<String> paymentMethodTypes;

  /// Present *with* [PaymentIntentStatus.requiresPaymentMethod] — there is
  /// no `failed` status.
  final LastPaymentError? lastPaymentError;

  final Map<String, String> metadata;

  final String? description;

  /// Unix **seconds**, not milliseconds.
  final int created;

  final bool livemode;

  /// `pi_…_secret_…`. Never log this — see [toString].
  final String clientSecret;

  /// `true` once the intent will not change again without a new request.
  /// The Dart port of `sdks/stripe-js/src/client.ts`'s `hasStoppedMoving`:
  /// `requiresPaymentMethod` is terminal **only** with a [lastPaymentError]
  /// — it is also the status of an intent nobody has confirmed yet, and
  /// treating that as final would resolve a poll the instant it started.
  bool get hasStoppedMoving =>
      status == PaymentIntentStatus.succeeded ||
      status == PaymentIntentStatus.canceled ||
      (status == PaymentIntentStatus.requiresPaymentMethod &&
          lastPaymentError != null);

  factory PaymentIntent.fromJson(Map<String, Object?> json) => PaymentIntent(
    id: json['id']! as String,
    amount: json['amount']! as int,
    currency: json['currency']! as String,
    status: PaymentIntentStatus.fromWire(json['status']! as String),
    paymentMethodTypes: (json['payment_method_types']! as List<Object?>)
        .cast<String>(),
    lastPaymentError: json['last_payment_error'] == null
        ? null
        : LastPaymentError.fromJson(
            json['last_payment_error']! as Map<String, Object?>,
          ),
    metadata: (json['metadata']! as Map<Object?, Object?>)
        .cast<String, String>(),
    description: json['description'] as String?,
    created: json['created']! as int,
    livemode: json['livemode']! as bool,
    clientSecret: json['client_secret']! as String,
  );

  static bool isPaymentIntentJson(Object? body) =>
      body is Map && body['object'] == 'payment_intent' && body['id'] is String;

  /// D6: redacts [clientSecret]. Nothing else here is sensitive — an amount,
  /// a currency and a status are not credentials.
  @override
  String toString() =>
      'PaymentIntent(id: $id, status: $status, clientSecret: '
      '${redacted(clientSecret)})';
}

/// `open`, `complete` or `expired` (D10).
enum CheckoutSessionStatus {
  open,
  complete,
  expired,
  unknown;

  static CheckoutSessionStatus fromWire(String value) => switch (value) {
    'open' => CheckoutSessionStatus.open,
    'complete' => CheckoutSessionStatus.complete,
    'expired' => CheckoutSessionStatus.expired,
    _ => CheckoutSessionStatus.unknown,
  };
}

/// `checkout.session`'s `payment_status` — `vpay_api::model::CheckoutSessionObject`'s
/// wire, untouched by an expiry: a session can be `status: expired` with
/// `payment_status: failed` when the intent it drove failed before the
/// session's own expiry ran, and the screen machine
/// (`src/sheet/checkout_screen.dart`) needs exactly that combination to
/// decide whether an expired session shows the neutral "expired" screen or
/// the intent's own outcome — `machine.ts:251-305`'s `stateForContext`,
/// restated.
enum CheckoutSessionPaymentStatus {
  unpaid,
  paid,
  failed,
  unknown;

  static CheckoutSessionPaymentStatus fromWire(String value) => switch (value) {
    'unpaid' => CheckoutSessionPaymentStatus.unpaid,
    'paid' => CheckoutSessionPaymentStatus.paid,
    'failed' => CheckoutSessionPaymentStatus.failed,
    _ => CheckoutSessionPaymentStatus.unknown,
  };
}

/// `hosted` or `embedded`. D2 refuses `embedded` at the pre-flight.
enum CheckoutUiMode {
  hosted,
  embedded,
  unknown;

  static CheckoutUiMode fromWire(String value) => switch (value) {
    'hosted' => CheckoutUiMode.hosted,
    'embedded' => CheckoutUiMode.embedded,
    _ => CheckoutUiMode.unknown,
  };
}

/// `push` or `redirect` — `vpay_core::ProviderFlow`'s wire, and the one
/// capability of a rail a native sheet acts on: whether it collects
/// [RailSpec.fields] and confirms inline, or expects a redirect.
enum RailFlow {
  push,
  redirect,

  /// A flow value this package does not recognise. Structural, not
  /// per-code: `railChoices` (`src/sheet/rails.dart`) treats any rail whose
  /// [RailFlow] is `unknown` as unsupported (D9) rather than guessing what
  /// to render for it. This is what lets a rail the SDK has never heard of
  /// still be handled safely — refused with its code shown, never rendered
  /// blind.
  unknown;

  static RailFlow fromWire(String value) => switch (value) {
    'push' => RailFlow.push,
    'redirect' => RailFlow.redirect,
    _ => RailFlow.unknown,
  };
}

/// `vpay_provider::PhonePayerType`'s wire. Only `mobile` exists today.
enum RailFieldPhoneType {
  mobile,
  unknown;

  static RailFieldPhoneType fromWire(String value) => switch (value) {
    'mobile' => RailFieldPhoneType.mobile,
    _ => RailFieldPhoneType.unknown,
  };
}

/// What a [RailField] demands of the value at its key —
/// `vpay_provider::PayerFieldKind`'s wire, `#[serde(tag = "type")]`
/// flattened onto the field object it decorates.
///
/// **A card-collecting variant must never be added here**, even though the
/// wire's `type` is an open string and a future rail could in principle
/// declare one. Cards are out of scope forever (issue #189): a native PAN
/// field moves the integration from PCI SAQ-A to SAQ-D. A `type` this
/// package does not recognise — `"card"` included — decodes to
/// [RailFieldKindUnknown] and nothing else, so a sheet built on this SDK has
/// no representable way to render one.
sealed class RailFieldKind {
  const RailFieldKind();

  /// Reads the flattened `type` (and, for `"phone"`, `region`/`phone_type`)
  /// out of one field object.
  factory RailFieldKind.fromJson(Map<String, Object?> json) {
    final String type = json['type']! as String;
    if (type == 'phone') {
      return RailFieldKindPhone(
        region: json['region']! as String,
        phoneType: RailFieldPhoneType.fromWire(json['phone_type']! as String),
      );
    }
    return RailFieldKindUnknown(type);
  }
}

/// A phone number, validated server-side with `phonenumber` against
/// [region] and required to be of [phoneType]. The only kind a payer field
/// carries today (MTN's `msisdn`).
final class RailFieldKindPhone extends RailFieldKind {
  const RailFieldKindPhone({required this.region, required this.phoneType});

  /// ISO 3166-1 alpha-2, e.g. `"CM"`.
  final String region;

  final RailFieldPhoneType phoneType;
}

/// A field `type` this package does not recognise — carried through, never
/// rendered. See [RailFieldKind]'s doc comment for why this branch, and
/// only this branch, is where an unrecognised type (including a future
/// `"card"`) lands.
final class RailFieldKindUnknown extends RailFieldKind {
  const RailFieldKindUnknown(this.rawType);

  final String rawType;
}

/// One field a payer must fill in before a push rail can be confirmed
/// against — `vpay_provider::PayerField`'s wire.
final class RailField {
  const RailField({
    required this.name,
    required this.kind,
    required this.required,
    required this.labelKey,
  });

  /// The key under `payment_method_data[<rail code>][<name>]` this field
  /// reads, e.g. `"msisdn"`.
  final String name;

  final RailFieldKind kind;

  /// Whether confirm refuses a missing or blank value.
  final bool required;

  /// The i18n key a client looks up to label this field's input.
  final String labelKey;

  factory RailField.fromJson(Map<String, Object?> json) => RailField(
    name: json['name']! as String,
    kind: RailFieldKind.fromJson(json),
    required: json['required']! as bool,
    labelKey: json['label_key']! as String,
  );
}

/// [RailSpec.displayName]'s two languages — `vpay_api::model::RailDisplayName`'s
/// wire. Both members individually optional: a deployment may configure
/// only one language.
final class RailDisplayName {
  const RailDisplayName({this.en, this.fr});

  final String? en;
  final String? fr;

  factory RailDisplayName.fromJson(Map<String, Object?> json) =>
      RailDisplayName(en: json['en'] as String?, fr: json['fr'] as String?);
}

/// One rail a payer may confirm this session against —
/// `vpay_api::model::RailSpec`'s wire. Everything a native payer sheet needs
/// to render a rail with no branch anywhere in the client keyed on
/// [code]'s own value (#186, #189): the fields to collect, the flow that
/// decides whether to collect them at all, and a label a sheet can show
/// before its own i18n catalogue has a translation for [code].
final class RailSpec {
  const RailSpec({
    required this.code,
    required this.flow,
    required this.labelKey,
    required this.displayName,
    required this.fields,
  });

  /// `providers.code` — also the value confirm's `payment_method_data[type]`
  /// must carry. Never branched on by this package: `src/sheet/rails.dart`'s
  /// `railChoices` decides what to render from [flow] and each field's own
  /// [RailFieldKind], never from this value's identity — see those two for
  /// how a rail this package has never heard of still renders safely.
  final String code;

  final RailFlow flow;

  /// The i18n key for this rail's own name, mechanically `"rail.{code}"`. A
  /// client with no translation for it falls back to [displayName], and
  /// past that to [code] itself.
  final String labelKey;

  /// This deployment's own configured name for the rail, or `null` when it
  /// configured none — the server omits the key entirely rather than
  /// sending `null` (`RailSpec::display_name`'s own doc), and [fromJson]
  /// treats an absent key and a JSON `null` the same way.
  final RailDisplayName? displayName;

  /// What the payer must fill in before this rail can be confirmed against —
  /// empty for a redirect rail, which collects nothing. Always present as
  /// an array, never omitted, even when empty.
  final List<RailField> fields;

  factory RailSpec.fromJson(Map<String, Object?> json) => RailSpec(
    code: json['code']! as String,
    flow: RailFlow.fromWire(json['flow']! as String),
    labelKey: json['label_key']! as String,
    displayName: json['display_name'] == null
        ? null
        : RailDisplayName.fromJson(
            (json['display_name']! as Map).cast<String, Object?>(),
          ),
    fields: (json['fields']! as List<Object?>)
        .map(
          (Object? e) =>
              RailField.fromJson((e! as Map).cast<String, Object?>()),
        )
        .toList(growable: false),
  );
}

/// `checkout.session` as `GET /v1/browser/checkout/sessions/{id}` renders
/// it, with `payment_intent` **expanded** (the one place this differs from
/// the merchant SDKs' `CheckoutSession`, mirroring
/// `sdks/stripe-js/src/types.ts`'s own note on the same field) — thirteen
/// keys on the wire, plus [clientSecret], which is **not** one of them; see
/// its own doc comment.
final class CheckoutSession {
  const CheckoutSession({
    required this.id,
    required this.livemode,
    required this.paymentIntent,
    required this.uiMode,
    required this.status,
    required this.successUrl,
    required this.cancelUrl,
    required this.url,
    required this.expiresAt,
    required this.created,
    required this.clientSecret,
    required this.rails,
    required this.paymentStatus,
  });

  /// `cs_…`.
  final String id;

  final bool livemode;

  /// The intent this session drives, expanded — every [PaymentIntent] field,
  /// its own `client_secret` included (D2 item 1: the polling credential).
  final PaymentIntent paymentIntent;

  final CheckoutUiMode uiMode;

  final CheckoutSessionStatus status;

  /// See [CheckoutSessionPaymentStatus]'s own doc comment.
  final CheckoutSessionPaymentStatus paymentStatus;

  /// Hosted mode only; `null` on an embedded session. May carry the literal
  /// `{CHECKOUT_SESSION_ID}` (D2) — unsubstituted here, exactly as the
  /// merchant wrote it; substitution happens in `checkout_controller.dart`.
  final String? successUrl;

  /// Hosted mode only; `null` on an embedded session. Same substitution
  /// rule as [successUrl].
  final String? cancelUrl;

  /// The page vpay serves for a hosted session — carries the session's
  /// `client_secret` in its fragment (D6); `null` when embedded.
  final String? url;

  /// Unix **seconds**. 24 h from create (D10).
  final int expiresAt;

  /// Unix **seconds**.
  final int created;

  /// `cs_…_secret_…` — the credential [BrowserClient.retrieveCheckoutSession]
  /// was called with. Never log this — see [toString].
  ///
  /// **Not a wire field, on purpose, and this was a real bug against the
  /// real server until 2026-09-14** (found by
  /// `sdks/flutter/vpay_checkout_flutter/test_e2e/real_stack_e2e_test.dart`,
  /// which is exactly what a `MockClient`-only suite cannot catch: every
  /// fixture in `test/` had been fabricating this key). vpay's own handler
  /// never re-serves the session's own secret — only the intent's
  /// (`vpay_api::browser::checkout_sessions::retrieve`'s doc comment: "the
  /// credential exists so this page can drive confirm ... the page loses
  /// nothing: it read the secret on its first call"). Requiring it in
  /// [fromJson] made every real pre-flight fail with `unexpected_response`.
  /// The caller already holds this value — it is what authenticated the
  /// request — so [fromJson] takes it as a parameter instead of reading it
  /// off a body that will never carry it.
  final String clientSecret;

  /// The server-driven rail spec (#186, #189): one entry per code in the
  /// intent's `payment_method_types`, in that order. Always present as an
  /// array, never omitted, even when empty — `CheckoutSessionForPayer::rails`'s
  /// own doc: this is "here is what to render a picker from", not "the
  /// merchant configured nothing", so an absent array would be a version
  /// skew, not a legitimate empty state.
  final List<RailSpec> rails;

  /// [clientSecret] is **not** read from [json] — see that field's doc
  /// comment for why the server never sends it back — so the caller passes
  /// the value it already authenticated with.
  factory CheckoutSession.fromJson(
    Map<String, Object?> json, {
    required String clientSecret,
  }) => CheckoutSession(
    id: json['id']! as String,
    livemode: json['livemode']! as bool,
    paymentIntent: PaymentIntent.fromJson(
      json['payment_intent']! as Map<String, Object?>,
    ),
    uiMode: CheckoutUiMode.fromWire(json['ui_mode']! as String),
    status: CheckoutSessionStatus.fromWire(json['status']! as String),
    successUrl: json['success_url'] as String?,
    cancelUrl: json['cancel_url'] as String?,
    url: json['url'] as String?,
    expiresAt: json['expires_at']! as int,
    created: json['created']! as int,
    clientSecret: clientSecret,
    rails: (json['rails']! as List<Object?>)
        .map(
          (Object? e) => RailSpec.fromJson((e! as Map).cast<String, Object?>()),
        )
        .toList(growable: false),
    paymentStatus: CheckoutSessionPaymentStatus.fromWire(
      json['payment_status']! as String,
    ),
  );

  static bool isCheckoutSessionJson(Object? body) =>
      body is Map &&
      body['object'] == 'checkout.session' &&
      body['id'] is String;

  /// D6: redacts [url] (carries the session secret in its fragment) and
  /// [clientSecret]. [paymentIntent] renders through its own overridden
  /// [PaymentIntent.toString], so its secret is redacted too rather than
  /// relying on this method to remember to do it a second time.
  @override
  String toString() =>
      'CheckoutSession(id: $id, status: $status, url: ${redacted(url)}, '
      'clientSecret: ${redacted(clientSecret)}, paymentIntent: '
      '$paymentIntent)';
}
