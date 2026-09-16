import 'package:flutter_test/flutter_test.dart';
import 'package:vpay_checkout_flutter/vpay_checkout_flutter.dart';

void main() {
  group(
    'jitteredDelay — [0.75, 1.25] × base, deterministic given a fixed RNG',
    () {
      test('a source that always answers 0.0 gives the minimum, 0.75×', () {
        final Duration delay = jitteredDelay(
          const Duration(milliseconds: 2000),
          source: FixedJitterSource([0.0]),
        );
        expect(delay, const Duration(milliseconds: 1500));
      });

      test('a source that always answers 1.0 gives the maximum, 1.25×', () {
        final Duration delay = jitteredDelay(
          const Duration(milliseconds: 2000),
          source: FixedJitterSource([1.0]),
        );
        expect(delay, const Duration(milliseconds: 2500));
      });

      test('a source that always answers 0.5 gives exactly the base — no jitter at the midpoint', () {
        final Duration delay = jitteredDelay(
          const Duration(milliseconds: 2000),
          source: FixedJitterSource([0.5]),
        );
        expect(delay, const Duration(milliseconds: 2000));
      });

      test('two different draws from the same source answer two different delays — payers do not poll in lockstep', () {
        final FixedJitterSource source = FixedJitterSource([0.0, 1.0]);
        final Duration first = jitteredDelay(
          const Duration(milliseconds: 2000),
          source: source,
        );
        final Duration second = jitteredDelay(
          const Duration(milliseconds: 2000),
          source: source,
        );
        expect(first, isNot(second));
      });

      test('is deterministic: the same fixed sequence gives the same delay every time', () {
        Duration draw() => jitteredDelay(
          const Duration(milliseconds: 2000),
          source: FixedJitterSource([0.37]),
        );
        expect(draw(), draw());
      });
    },
  );

  group('nextPollDelay — jitter capped by the remaining budget', () {
    test('answers the jittered interval when the budget has plenty left', () {
      final Duration delay = nextPollDelay(
        interval: const Duration(milliseconds: 2000),
        remaining: const Duration(minutes: 3),
        source: FixedJitterSource([1.0]),
      );
      expect(delay, const Duration(milliseconds: 2500));
    });

    test(
      'caps at whatever remains when the jittered interval would overrun it',
      () {
        final Duration delay = nextPollDelay(
          interval: const Duration(milliseconds: 2000),
          remaining: const Duration(milliseconds: 800),
          source: FixedJitterSource([1.0]), // would otherwise be 2500ms
        );
        expect(delay, const Duration(milliseconds: 800));
      },
    );

    test('a budget already at or below zero answers Duration.zero without drawing further delay', () {
      final Duration delay = nextPollDelay(
        interval: const Duration(milliseconds: 2000),
        remaining: Duration.zero,
        source: FixedJitterSource([0.5]),
      );
      expect(delay, Duration.zero);
    });
  });

  group('FixedJitterSource', () {
    test('cycles once its values are exhausted, so a test need not size it to the attempt count', () {
      final FixedJitterSource source = FixedJitterSource([0.0, 1.0]);
      expect(source.nextDouble(), 0.0);
      expect(source.nextDouble(), 1.0);
      expect(source.nextDouble(), 0.0);
      expect(source.nextDouble(), 1.0);
    });
  });

  group('SystemJitterSource', () {
    test('answers a value in [0, 1)', () {
      const SystemJitterSource source = SystemJitterSource();
      for (int i = 0; i < 20; i += 1) {
        final double value = source.nextDouble();
        expect(value, greaterThanOrEqualTo(0.0));
        expect(value, lessThan(1.0));
      }
    });
  });

  group('defaults', () {
    test('match sdks/stripe-js/src/client.ts\'s own poll defaults', () {
      expect(defaultPollInterval, const Duration(milliseconds: 2000));
      expect(defaultPollBudget, const Duration(minutes: 3));
      expect(pollJitterSpread, 0.5);
    });
  });
}
