/// Jittered poll delays — the Dart port of
/// `sdks/stripe-js/src/client.ts:57-58,682-694`'s `jitter`/poll-delay pair.
///
/// A page (or sheet) of payers polling on an identical fixed interval would
/// all hit the API in lockstep. `waitForPaymentIntent` spreads each poll's
/// delay across `[0.75, 1.25] × intervalMs` instead — the same three-minute
/// budget and two-second interval `checkout_controller.dart`'s
/// `CheckoutController` already polls on, jittered.
///
/// **This file does not poll anything.** It is the pure arithmetic a poll
/// loop needs — the delay for one attempt, given how much budget is left —
/// with the randomness injected as a [JitterSource] so a test can fix the
/// sequence and assert an exact value rather than a range. A later lane's
/// `SheetController` (the impure half that actually calls `BrowserClient`
/// and sleeps) is expected to use this rather than reinvent it; it is kept
/// separate from `checkout_controller.dart`'s own poll loop deliberately —
/// see that file's module doc comment for why its D1/D2/D4 logic is out of
/// scope for this lane to touch.
library;

import 'dart:math' as math;

/// A source of randomness a poll loop can be given a fixed sequence for in
/// tests. [SystemJitterSource.nextDouble] already answers `[0, 1)`, exactly
/// what `client.ts`'s own `jitter` draws from `Math.random()`.
abstract class JitterSource {
  double nextDouble();
}

/// The real source — a lazily-created, process-wide [math.Random]. What
/// production code uses unless a test replaces it with a fixed sequence.
final class SystemJitterSource implements JitterSource {
  const SystemJitterSource();

  static final math.Random _random = math.Random();

  @override
  double nextDouble() => _random.nextDouble();
}

/// A fixed, deterministic sequence — cycles once it runs out, so a test does
/// not have to size it to the exact number of poll attempts.
final class FixedJitterSource implements JitterSource {
  FixedJitterSource(this.values)
    : assert(values.isNotEmpty, 'values must not be empty');

  final List<double> values;
  int _index = 0;

  @override
  double nextDouble() {
    final double value = values[_index % values.length];
    _index += 1;
    return value;
  }
}

/// `client.ts`'s poll defaults: a two-second interval and a three-minute
/// budget — the same values `checkout_controller.dart`'s
/// `defaultStopUrlPollInterval`/`defaultStopUrlPollTimeout` already carry,
/// restated here so this file has no dependency on that one.
const Duration defaultPollInterval = Duration(milliseconds: 2000);
const Duration defaultPollBudget = Duration(minutes: 3);

/// The spread of the jitter multiplier around `1.0` — `client.ts`'s
/// `JITTER_SPREAD`. `[0.75, 1.25]` at the default of `0.5`.
const double pollJitterSpread = 0.5;

/// `[0.75, 1.25] × base`, rounded to the nearest millisecond —
/// `client.ts`'s `jitter`, ported line for line: `base * (1 - spread / 2 +
/// random() * spread)`. Deterministic given a [source] that answers a fixed
/// sequence, which is the whole reason this takes one rather than reading a
/// `Random` off a global.
Duration jitteredDelay(
  Duration base, {
  required JitterSource source,
  double spread = pollJitterSpread,
}) {
  final double factor = 1 - spread / 2 + source.nextDouble() * spread;
  final int millis = (base.inMilliseconds * factor).round();
  return Duration(milliseconds: millis < 0 ? 0 : millis);
}

/// The delay before a poll ladder's next attempt: the jittered [interval],
/// capped at whatever is left of the budget — `client.ts`'s
/// `Math.min(jitter(intervalMs), remaining)` in `waitForPaymentIntent`.
/// Never negative: a [remaining] at or below zero answers [Duration.zero],
/// which is the caller's cue that the budget is spent.
Duration nextPollDelay({
  required Duration interval,
  required Duration remaining,
  required JitterSource source,
}) {
  if (remaining <= Duration.zero) {
    return Duration.zero;
  }
  final Duration jittered = jitteredDelay(interval, source: source);
  return jittered < remaining ? jittered : remaining;
}
