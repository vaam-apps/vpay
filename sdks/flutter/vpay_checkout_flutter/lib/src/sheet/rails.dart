/// Which rails this sheet can drive, and how — the Dart port of
/// `frontends/apps/checkout/src/lib/rails.ts`'s job, rebuilt for a
/// server-driven spec.
///
/// **`rails.ts` itself is not portable line for line, and that is
/// deliberate.** It keyed a hard-coded `RAIL_PAGE_FLOWS` map by rail code —
/// the only option before #186, when the server did not yet say what each
/// rail needed. Now `CheckoutSession.rails` (`../models.dart`) carries the
/// flow and the fields directly, so this module asks a **structural**
/// question instead of a per-code one: is this [RailSpec]'s [RailFlow]
/// something a sheet knows how to drive, and does every one of its fields
/// have a [RailFieldKind] this sheet can render? A rail the map in
/// `rails.ts` never named — or one invented tomorrow — answers that
/// question from its own spec, with **no rail code branched on anywhere in
/// this file**.
///
/// That is D9, restated: a rail this sheet cannot drive is listed as
/// unsupported, with its own [RailSpec.code], never silently dropped and
/// never rendered as a control that would fail on submit. If it is the
/// *only* rail the intent offers, the caller refuses outright rather than
/// showing a shorter list with no explanation (`checkout_screen.dart`'s
/// `stateForContext`).
library;

import '../models.dart';

/// A rail this sheet can actually drive — [RailSpec.flow] is known, and
/// (for a `push` rail) every declared field has a [RailFieldKind] this
/// package can render natively. Carries the whole [spec] through rather
/// than re-declaring its members, so a caller reads [RailSpec.fields]
/// straight off it.
final class SupportedRail {
  const SupportedRail(this.spec);

  final RailSpec spec;

  String get code => spec.code;
}

/// A rail the intent offers that this sheet cannot drive, and why —
/// [reason] never quotes anything the rail itself has no business
/// influencing (a field name is fine; nothing here reads a payer-entered
/// value).
final class UnsupportedRail {
  const UnsupportedRail({required this.code, required this.reason});

  final String code;
  final UnsupportedRailReason reason;
}

enum UnsupportedRailReason {
  /// [RailSpec.flow] was not one this sheet knows — `RailFlow.unknown`.
  unknownFlow,

  /// A `push` rail declared at least one field whose [RailFieldKind] this
  /// sheet cannot render — `RailFieldKindUnknown`, which is also where a
  /// hypothetical card field lands (`RailFieldKind`'s own doc comment: cards
  /// are out of scope forever, and this is the mechanism that keeps them
  /// out without a single `if (field.type == "card")` anywhere).
  unrenderableField,
}

/// The split of one session's [RailSpec]s into what this sheet can and
/// cannot drive.
final class RailChoices {
  const RailChoices({required this.supported, required this.unsupported});

  /// In the session's own order — the order the merchant's intent named
  /// them.
  final List<SupportedRail> supported;

  final List<UnsupportedRail> unsupported;
}

/// Whether a [RailFieldKind] is one this sheet can render natively. The one
/// and only gate against a card field ever reaching a form: any [RailField]
/// whose [RailFieldKind] is not [RailFieldKindPhone] — [RailFieldKindUnknown]
/// included, a future `"card"` type among them — fails this check.
bool _isRenderableField(RailField field) => field.kind is RailFieldKindPhone;

/// Splits [rails] into what this sheet can and cannot drive, narrowed by
/// [allowedMethods] — a deployment's own `checkout.allowed_methods`
/// (`config.yaml`), or `null` for "no opinion".
///
/// [allowedMethods] can only ever **narrow**: [rails] is what the server
/// already reduced the merchant's intent to, and a code an operator allows
/// that the session does not offer adds nothing. A rail the session offers
/// and the operator excludes lands in [RailChoices.unsupported] with
/// [UnsupportedRailReason.unknownFlow] rather than [unrenderableField] would
/// be misleading; excluding by policy is closer to "this sheet does not
/// offer it" than "cannot render it", so it is folded into the same
/// structural bucket rather than growing a third reason that only ever
/// means "an operator said no" — a caller that needs to distinguish the two
/// can already tell: a code missing from [allowedMethods] is the operator's
/// decision, not this sheet's.
RailChoices railChoices(List<RailSpec> rails, {List<String>? allowedMethods}) {
  final List<SupportedRail> supported = [];
  final List<UnsupportedRail> unsupported = [];
  for (final RailSpec spec in rails) {
    if (allowedMethods != null && !allowedMethods.contains(spec.code)) {
      unsupported.add(
        UnsupportedRail(
          code: spec.code,
          reason: UnsupportedRailReason.unknownFlow,
        ),
      );
      continue;
    }
    if (spec.flow == RailFlow.unknown) {
      unsupported.add(
        UnsupportedRail(
          code: spec.code,
          reason: UnsupportedRailReason.unknownFlow,
        ),
      );
      continue;
    }
    if (spec.flow == RailFlow.push && !spec.fields.every(_isRenderableField)) {
      unsupported.add(
        UnsupportedRail(
          code: spec.code,
          reason: UnsupportedRailReason.unrenderableField,
        ),
      );
      continue;
    }
    supported.add(SupportedRail(spec));
  }
  return RailChoices(supported: supported, unsupported: unsupported);
}
