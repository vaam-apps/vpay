/// The state machine (design doc, "The shape", D1/D2/D4). **Pure**: no HTTP
/// of its own (every network call goes through the injected [BrowserClient])
/// and no ambient timer (every wait goes through the injected [VpayClock]) —
/// the same choice `frontends/apps/checkout/src/lib/machine.ts` made with its
/// reducer, restated here as dependency injection rather than as a
/// zero-argument pure function, because this machine's job (poll until a
/// budget elapses) is inherently about time, and a reducer that could not
/// mention time at all would have nowhere to put that rule.
///
/// Two independent properties this file exists to hold, both named in the
/// implementation brief because both are parity rows on their own:
///
/// - **D1: the outcome is never read off a URL.** [resolveAfterStopUrlReached]
///   does not accept or consult the URL that was reached — see
///   `pigeons/checkout.dart`'s [CheckoutWindowEvent.reachedUrl] doc comment,
///   which says the same thing from the platform side. Only
///   [BrowserClient.retrievePaymentIntent] decides.
/// - **D4: dismissal polls before it reports.** [resolveAfterDismissal] runs
///   the exact same poll loop [resolveAfterStopUrlReached] does, on a
///   shorter budget — there is no separate "just say canceled" code path for
///   a dismissal to fall into.
library;

import 'browser_client.dart';
import 'errors.dart';
import 'models.dart';
import 'result.dart';

/// Injected so every wait in this file is fake in a test and real on a
/// device — the "no timers of its own" half of this file's purity.
abstract class VpayClock {
  const VpayClock();

  DateTime now();

  Future<void> delay(Duration duration);
}

/// The real clock. What `CheckoutController` uses unless a test replaces it.
final class SystemClock extends VpayClock {
  const SystemClock();

  @override
  DateTime now() => DateTime.now();

  @override
  Future<void> delay(Duration duration) => Future<void>.delayed(duration);
}

/// One stop URL, normalised: scheme, host, port and path only — query and
/// fragment are never compared (D2), so they are not even kept.
final class StopUrlSpec {
  const StopUrlSpec({
    required this.scheme,
    required this.host,
    required this.port,
    required this.path,
  });

  final String scheme;
  final String host;
  final int port;
  final String path;

  static int _defaultPortFor(String scheme) => switch (scheme) {
    'https' => 443,
    'http' => 80,
    _ => 0,
  };

  /// Substitutes `{CHECKOUT_SESSION_ID}` (D2) and parses `raw` into a spec,
  /// or `null` when `raw` is absent or not an absolute URL — a `null`
  /// `success_url`/`cancel_url` on an embedded session is expected, not an
  /// error.
  static StopUrlSpec? fromConfigured(String? raw, {required String sessionId}) {
    if (raw == null) {
      return null;
    }
    final String substituted = raw.replaceAll(
      '{CHECKOUT_SESSION_ID}',
      sessionId,
    );
    final Uri? uri = Uri.tryParse(substituted);
    if (uri == null || !uri.hasScheme || uri.host.isEmpty) {
      return null;
    }
    return StopUrlSpec(
      scheme: uri.scheme,
      host: uri.host,
      port: uri.hasPort ? uri.port : _defaultPortFor(uri.scheme),
      path: uri.path,
    );
  }

  /// `true` when `uri`'s scheme, host, port and path match this spec —
  /// query and fragment are ignored (D2), so a merchant's own tracking
  /// parameters on `success_url` cannot break the match.
  bool matches(Uri uri) {
    final int port = uri.hasPort ? uri.port : _defaultPortFor(uri.scheme);
    return uri.scheme == scheme &&
        uri.host == host &&
        port == this.port &&
        uri.path == path;
  }
}

/// What the pre-flight (D2) bought, once a session read succeeded and the
/// session was not `embedded`.
final class CheckoutPreflightReady {
  const CheckoutPreflightReady({
    required this.sessionId,
    required this.paymentIntentId,
    required this.intentClientSecret,
    required this.successStopUrl,
    required this.cancelStopUrl,
  });

  final String sessionId;
  final String paymentIntentId;

  /// D2 item 1: the polling credential, read while the session is `open` —
  /// good for the rest of the intent's life thereafter.
  final String intentClientSecret;

  final StopUrlSpec? successStopUrl;
  final StopUrlSpec? cancelStopUrl;

  /// `null` when neither [successStopUrl] nor [cancelStopUrl] is
  /// configured — nothing for a platform host to watch for, though it may
  /// still watch for a dismissal.
  List<StopUrlSpec> get stopUrls => [
    if (successStopUrl != null) successStopUrl!,
    if (cancelStopUrl != null) cancelStopUrl!,
  ];
}

/// The pre-flight's outcome: either [CheckoutPreflightReady], or a typed
/// [VpayError] — a bad key, an expired link, or an `embedded` session
/// (D2 item 3), all refused before any window opens.
sealed class CheckoutPreflight {
  const CheckoutPreflight();
}

final class CheckoutPreflightSuccess extends CheckoutPreflight {
  const CheckoutPreflightSuccess(this.ready);

  final CheckoutPreflightReady ready;
}

final class CheckoutPreflightFailure extends CheckoutPreflight {
  const CheckoutPreflightFailure(this.error);

  final VpayError error;
}

/// The default budget [CheckoutController.resolveAfterStopUrlReached] polls
/// on — `sdks/stripe-js`'s `waitForPaymentIntent` default, reused rather
/// than invented: three minutes, at a two-second interval.
const Duration defaultStopUrlPollTimeout = Duration(minutes: 3);
const Duration defaultStopUrlPollInterval = Duration(seconds: 2);

/// D4: "a **short** poll (a few seconds)" — before reporting anything for a
/// dismissal.
const Duration defaultDismissalPollTimeout = Duration(seconds: 5);
const Duration defaultDismissalPollInterval = Duration(seconds: 1);

/// The pure state machine: pre-flight, and the one poll loop both a reached
/// stop URL and a dismissal resolve through.
final class CheckoutController {
  const CheckoutController({
    required this.client,
    this.clock = const SystemClock(),
  });

  final BrowserClient client;
  final VpayClock clock;

  /// D2: one pre-flight session read; stop URLs derived, not configured.
  /// `sessionClientSecret` is the `cs_…_secret_…` read out of the session
  /// `url`'s fragment (D6) before this is called.
  Future<CheckoutPreflight> preflight(String sessionClientSecret) async {
    final CheckoutSessionResult result = await client.retrieveCheckoutSession(
      sessionClientSecret,
    );
    if (result.isError) {
      return CheckoutPreflightFailure(result.error!);
    }
    final CheckoutSession session = result.checkoutSession!;
    if (session.uiMode == CheckoutUiMode.embedded) {
      return CheckoutPreflightFailure(
        VpayError.embeddedSessionNotSupported(session.id),
      );
    }
    return CheckoutPreflightSuccess(
      CheckoutPreflightReady(
        sessionId: session.id,
        paymentIntentId: session.paymentIntent.id,
        intentClientSecret: session.paymentIntent.clientSecret,
        successStopUrl: StopUrlSpec.fromConfigured(
          session.successUrl,
          sessionId: session.id,
        ),
        cancelStopUrl: StopUrlSpec.fromConfigured(
          session.cancelUrl,
          sessionId: session.id,
        ),
      ),
    );
  }

  /// D1: called once the platform host reports a navigation matched a stop
  /// URL. Deliberately takes no URL parameter — see this file's doc
  /// comment — and resolves purely from polling
  /// [BrowserClient.retrievePaymentIntent].
  Future<VpayCheckoutResult> resolveAfterStopUrlReached(
    CheckoutPreflightReady ready, {
    Duration timeout = defaultStopUrlPollTimeout,
    Duration interval = defaultStopUrlPollInterval,
  }) => _resolve(ready, timeout: timeout, interval: interval);

  /// D4: called once the platform host reports the payer dismissed the
  /// window. Runs the identical poll loop [resolveAfterStopUrlReached] does,
  /// only shorter — there is no separate "report canceled" branch here.
  Future<VpayCheckoutResult> resolveAfterDismissal(
    CheckoutPreflightReady ready, {
    Duration timeout = defaultDismissalPollTimeout,
    Duration interval = defaultDismissalPollInterval,
  }) => _resolve(ready, timeout: timeout, interval: interval);

  Future<VpayCheckoutResult> _resolve(
    CheckoutPreflightReady ready, {
    required Duration timeout,
    required Duration interval,
  }) async {
    final DateTime deadline = clock.now().add(timeout);
    for (;;) {
      final PaymentIntentResult result = await client.retrievePaymentIntent(
        ready.intentClientSecret,
      );
      if (result.isError) {
        return VpayCheckoutUnresolved(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          error: result.error!,
        );
      }
      final PaymentIntent intent = result.paymentIntent!;
      if (intent.id != ready.paymentIntentId) {
        // The request was addressed by `ready.intentClientSecret`, whose own
        // prefix *is* `ready.paymentIntentId`, so these can only disagree if
        // the server answered about a different object. Reporting that
        // object's `succeeded` as this session's outcome is precisely the
        // "claims success it did not observe" failure D1 exists to refuse,
        // so it is an unresolved, not a result. No id is interpolated: one
        // of them came off the wire.
        return VpayCheckoutUnresolved(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          error: VpayError.unexpectedResponse(200),
        );
      }
      if (intent.hasStoppedMoving) {
        return _resultFor(ready, intent);
      }
      final Duration remaining = deadline.difference(clock.now());
      if (remaining <= Duration.zero) {
        return VpayCheckoutPending(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
        );
      }
      await clock.delay(remaining < interval ? remaining : interval);
    }
  }

  VpayCheckoutResult _resultFor(
    CheckoutPreflightReady ready,
    PaymentIntent intent,
  ) {
    switch (intent.status) {
      case PaymentIntentStatus.succeeded:
        return VpayCheckoutSucceeded(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
        );
      case PaymentIntentStatus.canceled:
        return VpayCheckoutCanceled(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
        );
      case PaymentIntentStatus.requiresPaymentMethod:
        return VpayCheckoutFailed(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
          code: intent.lastPaymentError?.code,
          providerMessage: intent.lastPaymentError?.message,
        );
      case PaymentIntentStatus.requiresAction:
      case PaymentIntentStatus.processing:
      case PaymentIntentStatus.unknown:
        // hasStoppedMoving is false for these, so `_resolve` never reaches
        // here with one of them — kept exhaustive rather than a `default`
        // so a fifth status added to the enum fails this `switch` at
        // compile time instead of silently falling through.
        return VpayCheckoutPending(
          sessionId: ready.sessionId,
          paymentIntentId: ready.paymentIntentId,
        );
    }
  }
}
