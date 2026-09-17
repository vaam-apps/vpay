/// [VpayCheckoutTheme] — the sheet's Material 3 appearance, and the one
/// place a host app changes how the checkout looks.
///
/// # What changed, and why the old rule is gone
///
/// Until 2026-09-17 this package had **no** theming surface at all, on a
/// deliberate rule recorded in three places: the sheet "inherits
/// `ThemeData`… never a fixed vpay palette", and
/// `CheckoutPageBranding.primaryColor` was parsed out of
/// `/.well-known/vpay-checkout` and then pointedly *not* rendered.
///
/// The maintainer reversed that: a deployment's `primary_color` now seeds
/// the sheet's [ColorScheme]. The old rule is not weakened silently, so
/// here is what replaced it.
///
/// **The sheet still has no palette of its own.** There is no vpay blue
/// anywhere in this file. What changed is only *whose* colour wins when
/// more than one is on offer, and the order is written down in
/// [resolve]: an explicit [colorScheme], then an explicit [seedColor],
/// then the deployment's `primary_color`, then — unchanged from before —
/// the host app's own `ThemeData`. A host that passes nothing and deploys
/// no brand colour gets exactly the inherited theme it got before.
///
/// That ordering is the point. The deployment's colour is the *operator's*
/// brand, and it should lose to anything the app developer says
/// explicitly, because the app developer is the one looking at the screen.
///
/// # Material 3
///
/// `useMaterial3` is not set here, and setting it would be noise: it has
/// defaulted to `true` since Flutter 3.16 and this package's floor is
/// `>=3.47.0`. What this class actually does for M3 is supply the
/// component themes M3 wants and Flutter does not default for you —
/// filled text fields with a real shape, buttons on the M3 shape scale
/// with a finger-sized minimum, and cards that sit on a tonal surface
/// rather than an elevation shadow.
///
/// Every colour below is a [ColorScheme] role. No `Colors.*` constant and
/// no `Color(0x…)` literal appears in this file, which is the property
/// that makes the sheet legible on a dark host theme without a second
/// code path.
library;

import 'package:flutter/material.dart';

/// The sheet's top corner radius, and [VpayCheckoutTheme.sheetCornerRadius]'s
/// default.
///
/// 28, not the Material default: this sheet is frequently *replaced on
/// screen* by a system browser sheet — `SFSafariViewController` on iOS, a
/// partial Custom Tab on Android — when the payer picks a redirect rail
/// like Orange Money. Both of those are drawn by the OS with a much
/// rounder corner than Material's, and a squarer vpay sheet handing over
/// to a rounder system one reads as a glitch rather than a transition.
/// 28 sits close enough to both that the swap is not jarring.
///
/// Override it when the host app's own surfaces have a different language
/// — a merchant whose app is square everywhere should not get one rounded
/// rectangle in the middle of it.
const double kVpayCheckoutSheetCornerRadius = 28;

/// How the checkout sheet looks.
///
/// Pass one to `showVpayCheckoutSheet`, `showVpayCheckoutSheetRoute` or
/// `VpayCheckoutSheet` itself. Every field has a default that produces the
/// stock Material 3 sheet, so `const VpayCheckoutTheme()` is a no-op and
/// passing nothing at all is the same thing.
@immutable
class VpayCheckoutTheme {
  const VpayCheckoutTheme({
    this.colorScheme,
    this.seedColor,
    this.brightness,
    this.useDeploymentBrandColor = true,
    this.sheetCornerRadius = kVpayCheckoutSheetCornerRadius,
    this.surfaceCornerRadius = 16,
    this.fieldCornerRadius = 12,
    this.buttonCornerRadius = 16,
    this.minimumTapTarget = const Size.fromHeight(52),
    this.contentPadding = const EdgeInsets.fromLTRB(20, 12, 20, 24),
    this.filledFields = true,
    this.textTheme,
  });

  /// A complete [ColorScheme], used verbatim. The bluntest override and
  /// the highest precedence: a host that builds its own M3 scheme already
  /// knows better than any seed this package could derive.
  final ColorScheme? colorScheme;

  /// A brand colour to derive a full M3 scheme from, via
  /// [ColorScheme.fromSeed]. Ignored when [colorScheme] is given.
  ///
  /// A *seed* rather than a primary: M3 generates a tonal palette from it,
  /// so the result stays contrast-correct even when the brand colour
  /// itself would be unreadable as a button fill.
  final Color? seedColor;

  /// The brightness to derive the scheme at. `null` follows the host
  /// app's own `Theme.of(context).brightness`, which is what makes a
  /// seeded sheet respect a dark host without the caller re-deriving it.
  final Brightness? brightness;

  /// Whether a `primary_color` published at `/.well-known/vpay-checkout`
  /// may seed the scheme when neither [colorScheme] nor [seedColor] is
  /// given.
  ///
  /// Default `true` — the operator published it in order to be seen. Set
  /// it `false` to pin the sheet to the host app's theme regardless of
  /// what any deployment says.
  final bool useDeploymentBrandColor;

  /// The modal sheet's top corners. Only [showVpayCheckoutSheet] reads it;
  /// a full route has no sheet shape to round.
  final double sheetCornerRadius;

  /// Cards and tonal panels — the amount summary, the notice and outcome
  /// blocks, the rail tiles.
  final double surfaceCornerRadius;

  /// The MSISDN field.
  final double fieldCornerRadius;

  /// Buttons. M3's own default is a stadium (fully rounded); 16 is
  /// squarer on purpose, so the primary action reads as a *button* beside
  /// the rounded tiles above it rather than as another pill.
  final double buttonCornerRadius;

  /// The floor for every button's hit area.
  ///
  /// 52 is above both platform floors (Material's 48dp, Apple's 44pt)
  /// rather than at either, because those are minimums for *reachable*
  /// targets and these sit in a scrolling column near the bottom edge. A
  /// checkout sheet is a small surface a payer uses once, often
  /// one-handed, often on a cheap phone, and usually while slightly
  /// anxious about money — the wrong tap here costs a payment, not a
  /// scroll position.
  final Size minimumTapTarget;

  /// Padding around the sheet's scrolling content.
  final EdgeInsetsGeometry contentPadding;

  /// Whether the MSISDN field is M3-filled (`true`, the default) or
  /// outlined. M3 states a preference for filled; outlined is offered
  /// because a host app whose every other field is outlined should not
  /// get one filled field in the middle of a checkout.
  final bool filledFields;

  /// An optional [TextTheme] override. `null` keeps the host app's
  /// typography, which is almost always what a merchant wants — the sheet
  /// addresses type through M3 *roles* ([TextTheme.headlineSmall],
  /// [TextTheme.labelLarge] and so on), never through hardcoded sizes, so
  /// it inherits a font without any work here.
  final TextTheme? textTheme;

  /// Builds the [ThemeData] the sheet renders under.
  ///
  /// [deploymentBrandColor] is `branding.primary_color` from the locally
  /// cached deployment config, or `null` when the deployment published
  /// none, the cache is cold, or the fetch failed. Its precedence is the
  /// lowest of the three colour inputs, and it is skipped entirely when
  /// [useDeploymentBrandColor] is `false`.
  ///
  /// The resolution order, in one place so it can be read without
  /// tracing calls:
  ///
  /// 1. [colorScheme] — used verbatim.
  /// 2. [seedColor] — `ColorScheme.fromSeed`.
  /// 3. [deploymentBrandColor] — `ColorScheme.fromSeed`, if allowed.
  /// 4. `base.colorScheme` — the host app's, untouched.
  ThemeData resolve(ThemeData base, {Color? deploymentBrandColor}) {
    final Brightness targetBrightness = brightness ?? base.brightness;
    final Color? seed =
        seedColor ?? (useDeploymentBrandColor ? deploymentBrandColor : null);
    final ColorScheme scheme =
        colorScheme ??
        (seed == null
            ? base.colorScheme
            : ColorScheme.fromSeed(
                seedColor: seed,
                brightness: targetBrightness,
              ));

    final WidgetStateProperty<Size> minimum = WidgetStatePropertyAll<Size>(
      minimumTapTarget,
    );
    final RoundedRectangleBorder buttonShape = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(buttonCornerRadius),
    );
    ButtonStyle sized(ButtonStyle? style) =>
        (style ?? const ButtonStyle()).copyWith(
          minimumSize: minimum,
          shape: WidgetStatePropertyAll<OutlinedBorder>(buttonShape),
        );

    final OutlineInputBorder fieldBorder = OutlineInputBorder(
      borderRadius: BorderRadius.circular(fieldCornerRadius),
      // `BorderSide.none` only reads as "no outline" on a FILLED field;
      // on an outlined one it erases the outline that is the whole point.
      borderSide: filledFields ? BorderSide.none : const BorderSide(),
    );

    return base.copyWith(
      colorScheme: scheme,
      textTheme: textTheme ?? base.textTheme,
      // Padded, not shrink-wrapped: the point of `minimumTapTarget` is a
      // hit area, and `shrinkWrap` would give it back.
      materialTapTargetSize: MaterialTapTargetSize.padded,
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: sized(base.elevatedButtonTheme.style),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: sized(base.outlinedButtonTheme.style),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: sized(base.filledButtonTheme.style),
      ),
      textButtonTheme: TextButtonThemeData(
        style: sized(base.textButtonTheme.style),
      ),
      inputDecorationTheme: base.inputDecorationTheme.copyWith(
        filled: filledFields,
        fillColor: filledFields ? scheme.surfaceContainerHighest : null,
        border: fieldBorder,
        enabledBorder: fieldBorder,
        focusedBorder: fieldBorder.copyWith(
          borderSide: BorderSide(color: scheme.primary, width: 2),
        ),
        errorBorder: fieldBorder.copyWith(
          borderSide: BorderSide(color: scheme.error),
        ),
        focusedErrorBorder: fieldBorder.copyWith(
          borderSide: BorderSide(color: scheme.error, width: 2),
        ),
      ),
      cardTheme: base.cardTheme.copyWith(
        // Tonal, not floating. M3 prefers a surface-container tint over a
        // drop shadow, and a shadow under a card inside an already-raised
        // bottom sheet reads as two stacked elevations.
        elevation: 0,
        margin: EdgeInsets.zero,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(surfaceCornerRadius),
        ),
      ),
      progressIndicatorTheme: base.progressIndicatorTheme.copyWith(
        color: scheme.primary,
      ),
    );
  }

  VpayCheckoutTheme copyWith({
    ColorScheme? colorScheme,
    Color? seedColor,
    Brightness? brightness,
    bool? useDeploymentBrandColor,
    double? sheetCornerRadius,
    double? surfaceCornerRadius,
    double? fieldCornerRadius,
    double? buttonCornerRadius,
    Size? minimumTapTarget,
    EdgeInsetsGeometry? contentPadding,
    bool? filledFields,
    TextTheme? textTheme,
  }) => VpayCheckoutTheme(
    colorScheme: colorScheme ?? this.colorScheme,
    seedColor: seedColor ?? this.seedColor,
    brightness: brightness ?? this.brightness,
    useDeploymentBrandColor:
        useDeploymentBrandColor ?? this.useDeploymentBrandColor,
    sheetCornerRadius: sheetCornerRadius ?? this.sheetCornerRadius,
    surfaceCornerRadius: surfaceCornerRadius ?? this.surfaceCornerRadius,
    fieldCornerRadius: fieldCornerRadius ?? this.fieldCornerRadius,
    buttonCornerRadius: buttonCornerRadius ?? this.buttonCornerRadius,
    minimumTapTarget: minimumTapTarget ?? this.minimumTapTarget,
    contentPadding: contentPadding ?? this.contentPadding,
    filledFields: filledFields ?? this.filledFields,
    textTheme: textTheme ?? this.textTheme,
  );

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is VpayCheckoutTheme &&
          other.colorScheme == colorScheme &&
          other.seedColor == seedColor &&
          other.brightness == brightness &&
          other.useDeploymentBrandColor == useDeploymentBrandColor &&
          other.sheetCornerRadius == sheetCornerRadius &&
          other.surfaceCornerRadius == surfaceCornerRadius &&
          other.fieldCornerRadius == fieldCornerRadius &&
          other.buttonCornerRadius == buttonCornerRadius &&
          other.minimumTapTarget == minimumTapTarget &&
          other.contentPadding == contentPadding &&
          other.filledFields == filledFields &&
          other.textTheme == textTheme;

  @override
  int get hashCode => Object.hash(
    colorScheme,
    seedColor,
    brightness,
    useDeploymentBrandColor,
    sheetCornerRadius,
    surfaceCornerRadius,
    fieldCornerRadius,
    buttonCornerRadius,
    minimumTapTarget,
    contentPadding,
    filledFields,
    textTheme,
  );
}
