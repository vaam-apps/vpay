/// The rail's own words, shown *beside* the translated failure — the Dart
/// port of `frontends/apps/checkout/src/lib/failures.ts`'s `providerReason`
/// half.
///
/// `FAILURE_MESSAGES`/`failureMessage` (the `FailureCode` -> `MessageKey`
/// table) is **not** ported here: that lookup exists to translate a code
/// into a sentence, which needs the i18n catalogue this lane deliberately
/// does not have (issue #189, "out of scope for this lane" — i18n
/// catalogue). The screen-state machine (`checkout_screen.dart`) carries the
/// raw [FailureCode] through unchanged, exactly as `machine.ts`'s own
/// `Outcome.failure` does, and leaves translating it to whichever later lane
/// owns the dictionary.
///
/// What *is* ported is the part that is not translation: cleaning a rail's
/// free-text message so it is safe to render as data next to that
/// translated line, never instead of it — see [providerReason]'s doc
/// comment for why that distinction matters.
library;

/// The cap `failures.ts` applies to a cleaned provider reason.
const int maxProviderReasonLength = 300;

/// Every C0 and C1 control character, plus the Unicode line separators —
/// `failures.ts`'s `CONTROL` regex, restated. Naming the control characters
/// is the whole job: this is what strips them out of a rail's message
/// before it is ever rendered.
final RegExp _control = RegExp('[\u0000-\u001f\u007f-\u009f\u2028\u2029]+');

final RegExp _whitespaceRun = RegExp(r'\s+');

/// The rail's own sentence, where the API gave one, bounded and cleaned.
///
/// **Shown as data beside the translated message, never instead of it.**
/// `last_payment_error.message` is written by whatever wrote the rail's
/// response; it arrives in a language nobody chose, it is not part of the
/// closed [FailureCode] vocabulary, and this package does not control a
/// word of it. It is rendered anyway because a payer standing in a shop
/// with a generic "payment not completed" on screen, when the rail actually
/// said something specific ("insufficient funds (MTN-4001)"), is a payer
/// this package would otherwise be withholding the useful half from.
///
/// What this function guarantees to whatever renders its result: at most
/// [maxProviderReasonLength] characters, no control characters, internal
/// whitespace collapsed to single spaces, and `null` rather than an empty
/// string. It does not escape anything for a particular render target —
/// Flutter's own text widgets already treat a `String` as data, never as
/// markup, so there is nothing here to add on top of that.
String? providerReason(String? message) {
  if (message == null) {
    return null;
  }
  final String cleaned = message
      .replaceAll(_control, ' ')
      .replaceAll(_whitespaceRun, ' ')
      .trim();
  if (cleaned.isEmpty) {
    return null;
  }
  if (cleaned.length <= maxProviderReasonLength) {
    return cleaned;
  }
  return '${cleaned.substring(0, maxProviderReasonLength - 1)}…';
}
