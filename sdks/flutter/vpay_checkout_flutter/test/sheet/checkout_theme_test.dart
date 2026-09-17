import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group('VpayCheckoutTheme.resolve — colour precedence', () {
    test('a bare const VpayCheckoutTheme() with no deploymentBrandColor leaves '
        "base.colorScheme identical — the 'configures nothing, sees no "
        "change' promise", () {
      final ThemeData base = ThemeData(
        colorScheme: const ColorScheme.light(primary: Colors.teal),
      );

      final ThemeData resolved = const VpayCheckoutTheme().resolve(base);

      expect(resolved.colorScheme, base.colorScheme);
    });

    test('seedColor alone produces the same scheme as ColorScheme.fromSeed '
        "at the base's own brightness", () {
      final ThemeData base = ThemeData.light();
      const Color seed = Colors.deepPurple;

      final ThemeData resolved = const VpayCheckoutTheme(seedColor: seed)
          .resolve(base);

      expect(
        resolved.colorScheme,
        ColorScheme.fromSeed(seedColor: seed, brightness: base.brightness),
      );
    });

    test(
      'an explicit colorScheme wins over a seedColor when both are given',
      () {
        final ThemeData base = ThemeData.light();
        const ColorScheme explicitScheme = ColorScheme.light(
          primary: Colors.pink,
        );

        final ThemeData resolved = const VpayCheckoutTheme(
          colorScheme: explicitScheme,
          seedColor: Colors.deepPurple,
        ).resolve(base);

        expect(resolved.colorScheme, explicitScheme);
      },
    );

    test('an explicit seedColor wins over a deploymentBrandColor when both '
        'are given', () {
      final ThemeData base = ThemeData.light();
      const Color seed = Colors.deepPurple;

      final ThemeData resolved = const VpayCheckoutTheme(seedColor: seed)
          .resolve(base, deploymentBrandColor: Colors.orange);

      expect(
        resolved.colorScheme,
        ColorScheme.fromSeed(seedColor: seed, brightness: base.brightness),
      );
    });

    test('deploymentBrandColor alone (no colorScheme, no seedColor) seeds the '
        'scheme', () {
      final ThemeData base = ThemeData.light();
      const Color brandColor = Colors.orange;

      final ThemeData resolved = const VpayCheckoutTheme().resolve(
        base,
        deploymentBrandColor: brandColor,
      );

      expect(
        resolved.colorScheme,
        ColorScheme.fromSeed(
          seedColor: brandColor,
          brightness: base.brightness,
        ),
      );
    });

    test('useDeploymentBrandColor: false ignores deploymentBrandColor entirely '
        '— scheme stays base.colorScheme', () {
      final ThemeData base = ThemeData(
        colorScheme: const ColorScheme.light(primary: Colors.teal),
      );

      final ThemeData resolved = const VpayCheckoutTheme(
        useDeploymentBrandColor: false,
      ).resolve(base, deploymentBrandColor: Colors.orange);

      expect(resolved.colorScheme, base.colorScheme);
    });

    test('no colorScheme, no seedColor, no deploymentBrandColor — base.colorScheme '
        'untouched even with useDeploymentBrandColor at its true default', () {
      final ThemeData base = ThemeData(
        colorScheme: const ColorScheme.light(primary: Colors.teal),
      );

      final ThemeData resolved = const VpayCheckoutTheme().resolve(base);

      expect(resolved.colorScheme, base.colorScheme);
    });
  });

  group('VpayCheckoutTheme.resolve — brightness', () {
    test('a dark base plus a seedColor and no explicit brightness derives a '
        'dark scheme', () {
      final ThemeData base = ThemeData.dark();
      const Color seed = Colors.deepPurple;

      final ThemeData resolved = const VpayCheckoutTheme(seedColor: seed)
          .resolve(base);

      expect(resolved.colorScheme.brightness, Brightness.dark);
      expect(
        resolved.colorScheme,
        ColorScheme.fromSeed(seedColor: seed, brightness: Brightness.dark),
      );
    });

    test("an explicit brightness overrides the base's own brightness", () {
      final ThemeData base = ThemeData.light();
      const Color seed = Colors.deepPurple;

      final ThemeData resolved = const VpayCheckoutTheme(
        seedColor: seed,
        brightness: Brightness.dark,
      ).resolve(base);

      expect(resolved.colorScheme.brightness, Brightness.dark);
      expect(
        resolved.colorScheme,
        ColorScheme.fromSeed(seedColor: seed, brightness: Brightness.dark),
      );
    });
  });

  group('VpayCheckoutTheme.resolve — component themes', () {
    test('inputDecorationTheme.filled is true by default', () {
      final ThemeData resolved = const VpayCheckoutTheme().resolve(
        ThemeData.light(),
      );

      expect(resolved.inputDecorationTheme.filled, isTrue);
    });

    test('inputDecorationTheme.filled is false when filledFields is false', () {
      final ThemeData resolved = const VpayCheckoutTheme(filledFields: false)
          .resolve(ThemeData.light());

      expect(resolved.inputDecorationTheme.filled, isFalse);
    });

    test('every button theme carries minimumTapTarget as its minimumSize', () {
      const Size tapTarget = Size.fromHeight(60);
      final ThemeData resolved = const VpayCheckoutTheme(
        minimumTapTarget: tapTarget,
      ).resolve(ThemeData.light());

      final List<ButtonStyle?> styles = <ButtonStyle?>[
        resolved.filledButtonTheme.style,
        resolved.elevatedButtonTheme.style,
        resolved.outlinedButtonTheme.style,
        resolved.textButtonTheme.style,
      ];

      for (final ButtonStyle? style in styles) {
        expect(style?.minimumSize?.resolve(<WidgetState>{}), tapTarget);
      }
    });

    test('cardTheme.elevation is 0', () {
      final ThemeData resolved = const VpayCheckoutTheme().resolve(
        ThemeData.light(),
      );

      expect(resolved.cardTheme.elevation, 0);
    });
  });

  group('kVpayCheckoutSheetCornerRadius', () {
    test('is 28', () {
      expect(kVpayCheckoutSheetCornerRadius, 28);
    });

    test('is the default of sheetCornerRadius', () {
      expect(
        const VpayCheckoutTheme().sheetCornerRadius,
        kVpayCheckoutSheetCornerRadius,
      );
    });
  });

  group('VpayCheckoutTheme.copyWith', () {
    test('round-trips every field', () {
      const ColorScheme colorScheme = ColorScheme.light(primary: Colors.teal);
      const Color seedColor = Colors.deepPurple;
      const Brightness brightness = Brightness.dark;
      const bool useDeploymentBrandColor = false;
      const double sheetCornerRadius = 4;
      const double surfaceCornerRadius = 8;
      const double fieldCornerRadius = 2;
      const double buttonCornerRadius = 6;
      const Size minimumTapTarget = Size.fromHeight(60);
      const EdgeInsetsGeometry contentPadding = EdgeInsets.all(10);
      const bool filledFields = false;
      const TextTheme textTheme = TextTheme();

      final VpayCheckoutTheme theme = const VpayCheckoutTheme().copyWith(
        colorScheme: colorScheme,
        seedColor: seedColor,
        brightness: brightness,
        useDeploymentBrandColor: useDeploymentBrandColor,
        sheetCornerRadius: sheetCornerRadius,
        surfaceCornerRadius: surfaceCornerRadius,
        fieldCornerRadius: fieldCornerRadius,
        buttonCornerRadius: buttonCornerRadius,
        minimumTapTarget: minimumTapTarget,
        contentPadding: contentPadding,
        filledFields: filledFields,
        textTheme: textTheme,
      );

      expect(theme.colorScheme, colorScheme);
      expect(theme.seedColor, seedColor);
      expect(theme.brightness, brightness);
      expect(theme.useDeploymentBrandColor, useDeploymentBrandColor);
      expect(theme.sheetCornerRadius, sheetCornerRadius);
      expect(theme.surfaceCornerRadius, surfaceCornerRadius);
      expect(theme.fieldCornerRadius, fieldCornerRadius);
      expect(theme.buttonCornerRadius, buttonCornerRadius);
      expect(theme.minimumTapTarget, minimumTapTarget);
      expect(theme.contentPadding, contentPadding);
      expect(theme.filledFields, filledFields);
      expect(theme.textTheme, textTheme);
    });

    test('called with nothing keeps every field unchanged', () {
      const VpayCheckoutTheme original = VpayCheckoutTheme(
        seedColor: Colors.deepPurple,
        filledFields: false,
      );

      expect(original.copyWith(), original);
    });
  });

  group('VpayCheckoutTheme equality', () {
    test('two identical instances are equal, with equal hashCodes', () {
      const VpayCheckoutTheme a = VpayCheckoutTheme(
        seedColor: Colors.deepPurple,
        filledFields: false,
      );
      const VpayCheckoutTheme b = VpayCheckoutTheme(
        seedColor: Colors.deepPurple,
        filledFields: false,
      );

      expect(a, b);
      expect(a.hashCode, b.hashCode);
    });

    test('changing one field makes two instances unequal', () {
      const VpayCheckoutTheme a = VpayCheckoutTheme(
        seedColor: Colors.deepPurple,
      );
      final VpayCheckoutTheme b = a.copyWith(filledFields: false);

      expect(a, isNot(b));
    });
  });
}
