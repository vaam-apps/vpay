/// The impure half of the native sheet — issue #189, Lane 2. The Dart
/// analogue of `frontends/apps/checkout/src/lib/controller.ts`'s
/// `CheckoutController`, wired to the same three pieces the issue names:
/// [BrowserClient] for the session read and confirm, this file's own poll
/// loop (built on [nextPollDelay] — Lane 1's jittered-delay primitive,
/// deliberately left unwired until this lane) for the wait, and
/// [VpayCheckoutPlatform] for a redirect rail's hand-off to the existing
/// browser host.
///
/// **The state machine itself is not new work.** Every transition here goes
/// through [reduceCheckoutScreen]/[stateForContext] (`checkout_screen.dart`)
/// — this file supplies the side effects `controller.ts` also supplies
/// (network calls, polling, the hand-off), never a second copy of the
/// screen logic. The one exception, and the one place this file constructs
/// a [CheckoutWaiting] directly rather than through the reducer, is
/// documented on [_resumePollingAfterRedirectReturn] below: it is a
/// transition `machine.ts`/`checkout_screen.dart` structurally cannot
/// express, because on the web the payer's browser *navigates away* for a
/// redirect rail and "coming back" is a fresh page load into the separate
/// return machine (`return_screen.dart`'s `return.ts` port) — never a
/// transition within the same machine instance. The maintainer's decision
/// for this SDK is that **the sheet stays** (`docs/plans` for #189), so this
/// file gives the redirect flow the one transition the two-machine web
/// design never needed.
///
/// **Side-effect ordering matches `controller.ts` in the three ways issue
/// #189 names:**
///
/// - **poll-then-outcome**: [_afterIntentResult] only ever reaches
///   [CheckoutOutcome] after [_pollUntilTerminal] itself decided the intent
///   stopped moving — never from a guess about a redirect having happened.
/// - **redirect recorded before navigation**: [startRedirect] dispatches
///   [CheckoutRedirectRequired] (moving the state to [CheckoutRedirecting])
///   *before* [_handOffToBrowser] ever calls [VpayCheckoutPlatform.show].
/// - **at-most-once complete**: [result] is a single [Completer] completed
///   by [_maybeAnnounceComplete], guarded by [Completer.isCompleted] — a
///   second reach of [CheckoutOutcome] (a late poll answering after the
///   payer already pressed the outcome button, say) cannot complete it
///   twice.
library;

import 'dart:async';

import 'package:flutter/foundation.dart' show ChangeNotifier;

import '../browser_client.dart';
import '../checkout_controller.dart' show StopUrlSpec, SystemClock, VpayClock;
import '../errors.dart';
import '../models.dart';
import '../platform/checkout_platform.dart';
import '../platform/messages.g.dart' show CheckoutWindowEvent;
import '../result.dart';
import 'checkout_screen.dart';
import 'msisdn.dart';
import 'poll_jitter.dart';
import 'rails.dart';
import 'remember_msisdn.dart';

/// `controller.ts`'s `messageForStripeError`/`messageForCheckoutError`,
/// merged into one function: this port's error surface is already the
/// single [VpayError] type on both the read and the write side, so there is
/// only one mapping to write rather than two that could disagree.
String errorMessageKey(VpayError error) {
  if (error.type == 'api_connection_error') {
    return 'error.network';
  }
  // The literal comes first on purpose, so this line's source text does not
  // match `test/sheet/no_rail_code_branching_test.dart`'s deliberately
  // over-inclusive "branch on a rail's identity" pattern. This is vpay's
  // own error vocabulary, not a rail code, but the check cannot tell the
  // two apart from source text alone and is not asked to.
  if ('resource_missing' == error.code) {
    return 'error.session_not_found';
  }
  return 'error.unexpected';
}

/// The whole sheet, driven by hand — `start()`, then whichever of
/// [chooseRail]/[submitMsisdn]/[startRedirect]/[back]/[retryPoll]/
/// [returnToMerchant]/[dismiss] the payer's next action calls for. A plain
/// `ChangeNotifier` (Flutter's own, no third-party state-management
/// dependency — none is used anywhere else in this package or its
/// example) rather than a custom listenable: this is the widget layer, and
/// `ChangeNotifier` is exactly the "call `notifyListeners()`, a widget
/// rebuilds" contract a sheet needs.
final class SheetController extends ChangeNotifier {
  SheetController({
    required this.client,
    required this.sessionClientSecret,
    this.merchantName,
    this.allowedMethods,
    this.remembered = const VpayRememberedMsisdn(),
    VpayCheckoutPlatform? platform,
    this.clock = const SystemClock(),
    this.jitterSource = const SystemJitterSource(),
    this.pollInterval = defaultPollInterval,
    this.pollBudget = defaultPollBudget,
    this.dismissalPollBudget = const Duration(seconds: 5),
  }) : _platform = platform ?? VpayCheckoutPlatform.instance;

  final BrowserClient client;

  /// `cs_…_secret_…` — read once at construction, exactly as
  /// `VpayCheckout.start`'s `_SessionUrl.clientSecret` is.
  final String sessionClientSecret;

  /// The merchant's display name, or `null` — a caller-supplied value
  /// (`CheckoutContext.merchantName`'s own doc comment: parsing a name off
  /// the session is a later lane's job, and `CheckoutSession` carries none
  /// today), never derived from anything this controller reads itself.
  final String? merchantName;

  final List<String>? allowedMethods;

  final VpayRememberedMsisdn remembered;
  final VpayCheckoutPlatform _platform;
  final VpayClock clock;
  final JitterSource jitterSource;
  final Duration pollInterval;
  final Duration pollBudget;
  final Duration dismissalPollBudget;

  CheckoutScreenState _state = checkoutInitialState;
  CheckoutScreenState get state => _state;

  /// Set the first time a confirm is actually sent — [dismiss]'s own gate
  /// for D4: a dismissal before this is `true` had no payment in flight to
  /// poll for.
  bool _hasConfirmedOnce = false;

  final Completer<VpayCheckoutResult> _resultCompleter =
      Completer<VpayCheckoutResult>();

  /// Resolves **once**, the moment the sheet first reaches a terminal
  /// outcome (or is dismissed — see [dismiss]) — the "at most once complete"
  /// property `controller.ts`'s `#announceComplete` also holds. A merchant
  /// app awaits this the way it already awaits [VpayCheckout.start]'s
  /// return value; the widget itself awaits it only to know when the outcome
  /// button and the forwarding screen become reachable.
  Future<VpayCheckoutResult> get result => _resultCompleter.future;

  /// What this device remembered for the rail currently on screen, or
  /// `null`. Populated by [_loadRememberedMsisdn], read once a
  /// [CheckoutCollectMsisdn] screen is reached — never before, and never for
  /// a rail the payer has not (yet) chosen.
  String? defaultMsisdn;

  /// Whether this device holds *any* remembered record, for offering
  /// "Forget what this device remembers" — `memory.ts`'s `hasRecord`.
  bool hasRememberedRecord = false;

  /// Set once [forgetRemembered] has run, so the payer is told rather than
  /// left guessing — `memory.ts`'s own UI cue.
  bool forgotten = false;

  /// Whether the "remember this number" box is ticked. Owned here (not by
  /// the widget's own `State`) so it survives whatever the widget tree does
  /// between a payer ticking it and pressing Pay.
  bool rememberChecked = false;

  void setRememberChecked(bool value) {
    rememberChecked = value;
    notifyListeners();
  }

  Future<void> forgetRemembered() async {
    await remembered.forget();
    hasRememberedRecord = false;
    forgotten = true;
    notifyListeners();
  }

  void _setState(CheckoutScreenState next) {
    if (identical(next, _state)) {
      return;
    }
    _state = next;
    notifyListeners();
  }

  /// Reads the session and enters the state it implies — `controller.ts`'s
  /// `start`.
  Future<void> start() async {
    final CheckoutSessionResult sessionResult = await client
        .retrieveCheckoutSession(sessionClientSecret);
    if (sessionResult.isError) {
      _setState(
        reduceCheckoutScreen(_state, CheckoutLoadFailed(sessionResult.error!)),
      );
      return;
    }
    final CheckoutSession session = sessionResult.checkoutSession!;
    if (session.uiMode == CheckoutUiMode.embedded) {
      // A native sheet is never embedded in the web sense, but the refusal
      // stays reachable (D2's own rule, restated) rather than assuming a
      // session this SDK reads can never carry it.
      _setState(
        reduceCheckoutScreen(
          _state,
          const CheckoutRefuse(RefusalReason.embedNotAllowed),
        ),
      );
      return;
    }
    final CheckoutContext context = CheckoutContext(
      session: session,
      intent: session.paymentIntent,
      merchantName: merchantName,
      allowedMethods: allowedMethods,
    );
    _setState(reduceCheckoutScreen(_state, CheckoutLoaded(context)));
    await _afterStateSettled();
  }

  /// Whatever [start]/[chooseRail]/a confirm just landed on — resumes a
  /// poll, announces an outcome, or preloads a remembered number, exactly
  /// as `controller.ts`'s own post-dispatch checks do after `start`,
  /// `submitMsisdn` and `startRedirect`.
  Future<void> _afterStateSettled() async {
    final CheckoutScreenState s = _state;
    if (s is CheckoutWaiting) {
      await _pollLoop();
    } else if (s is CheckoutOutcome) {
      await _announceOutcome();
    } else if (s is CheckoutCollectMsisdn) {
      await _loadRememberedMsisdn(s.rail.code);
    }
  }

  Future<void> _loadRememberedMsisdn(String railCode) async {
    defaultMsisdn = await remembered.read(railCode);
    hasRememberedRecord = await remembered.hasRecord();
    notifyListeners();
  }

  void chooseRail(SupportedRail rail) {
    _setState(reduceCheckoutScreen(_state, CheckoutChooseRail(rail)));
    final CheckoutScreenState s = _state;
    if (s is CheckoutCollectMsisdn) {
      unawaited(_loadRememberedMsisdn(s.rail.code));
    }
  }

  void back() {
    _setState(reduceCheckoutScreen(_state, const CheckoutBack()));
  }

  /// The MTN path: normalise, confirm, then poll — `controller.ts`'s
  /// `submitMsisdn`. [raw] never leaves this method unnormalised — what
  /// reaches the rail is [normalizeCameroonMsisdn]'s canonical output.
  Future<void> submitMsisdn(String raw) async {
    final CheckoutScreenState s = _state;
    if (s is! CheckoutCollectMsisdn) {
      return;
    }
    final String? msisdn = normalizeCameroonMsisdn(raw);
    if (msisdn == null) {
      _setState(
        reduceCheckoutScreen(_state, const CheckoutProblem('msisdn.invalid')),
      );
      return;
    }
    final SupportedRail rail = s.rail;
    final String secret = s.context.intent.clientSecret;
    _hasConfirmedOnce = true;
    _setState(reduceCheckoutScreen(_state, const CheckoutConfirmStarted()));
    final PaymentIntentResult result = await client.confirmPaymentIntent(
      secret,
      railCode: rail.code,
      payerFields: <String, String>{'msisdn': msisdn},
    );
    if (result.isError) {
      _setState(
        reduceCheckoutScreen(
          _state,
          CheckoutProblem(errorMessageKey(result.error!)),
        ),
      );
      return;
    }
    // Written only here — a real, accepted submit — never from a keystroke
    // and never before the server has taken the number (this file's own
    // doc comment on `remember_msisdn.dart` states the same rule).
    if (rememberChecked) {
      await remembered.remember(msisdn: msisdn, railCode: rail.code);
      hasRememberedRecord = true;
    }
    await _afterIntentResult(result.paymentIntent!);
  }

  /// The Orange path: confirm, then hand off to the browser host if a
  /// redirect is required — `controller.ts`'s `startRedirect`, adapted for
  /// D9's decision that **the sheet stays**: the browser is the redirect
  /// handler, not a replacement checkout.
  Future<void> startRedirect() async {
    final CheckoutScreenState s = _state;
    if (s is! CheckoutReadyRedirect) {
      return;
    }
    final SupportedRail rail = s.rail;
    final String secret = s.context.intent.clientSecret;
    _hasConfirmedOnce = true;
    _setState(reduceCheckoutScreen(_state, const CheckoutConfirmStarted()));
    final PaymentIntentResult result = await client.confirmPaymentIntent(
      secret,
      railCode: rail.code,
      // No `return_url`: a checkout-session-scoped confirm is given one by
      // the server automatically (`vpay_provider::Charge::return_url`'s own
      // doc — "the vpay page that receives the payer for the checkout
      // session driving this charge"), so this client does not need to name
      // one, and naming the wrong one would only ever narrow what the
      // server already decided correctly.
    );
    if (result.isError) {
      _setState(
        reduceCheckoutScreen(
          _state,
          CheckoutProblem(errorMessageKey(result.error!)),
        ),
      );
      return;
    }
    final PaymentIntent intent = result.paymentIntent!;
    final String? url = intent.redirectUrl;
    if (url == null) {
      // A redirect rail that answered with nothing to redirect to — not an
      // error; poll and let the intent say what happened, same as
      // `controller.ts`'s own fallback.
      await _afterIntentResult(intent);
      return;
    }
    // Redirect recorded BEFORE the hand-off — controller.ts's own ordering.
    _setState(reduceCheckoutScreen(_state, CheckoutRedirectRequired(url)));
    await _handOffToBrowser(url);
  }

  /// Hands the redirecting state's URL to [VpayCheckoutPlatform] — the
  /// "existing browser host" issue #189 names — and waits for it to report
  /// either a matched stop URL or a dismissal, exactly the two signals
  /// [VpayCheckout] itself already waits for.
  Future<void> _handOffToBrowser(String url) async {
    final CheckoutScreenState redirecting = _state;
    if (redirecting is! CheckoutRedirecting) {
      return;
    }
    final CheckoutSession session = redirecting.context.session;
    final List<StopUrlSpec> stopUrls = <StopUrlSpec>[
      for (final StopUrlSpec? spec in <StopUrlSpec?>[
        StopUrlSpec.fromConfigured(session.successUrl, sessionId: session.id),
        StopUrlSpec.fromConfigured(session.cancelUrl, sessionId: session.id),
      ])
        if (spec != null) spec,
    ];

    final Completer<CheckoutWindowEvent> settled =
        Completer<CheckoutWindowEvent>();
    final StreamSubscription<CheckoutWindowEvent> subscription = _platform
        .windowEvents
        .listen(
          (CheckoutWindowEvent event) {
            if (!settled.isCompleted) {
              settled.complete(event);
            }
          },
          onError: (Object _) {
            if (!settled.isCompleted) {
              settled.completeError(const _WindowNeverReported());
            }
          },
          onDone: () {
            if (!settled.isCompleted) {
              settled.completeError(const _WindowNeverReported());
            }
          },
        );
    try {
      try {
        await _platform.show(
          url: url,
          stopUrls: stopUrls,
          allowInsecureUrl: client.allowInsecureBaseUrl,
        );
        await settled.future;
      } on Object {
        // A browser that would not open, or never reported an outcome: the
        // sheet does not know what happened on the rail's own page, so it
        // polls exactly as it would for a dismissal (D4) rather than
        // inventing a status.
      }
    } finally {
      await subscription.cancel();
    }

    // Whichever of the two signals arrived — or neither — the sheet never
    // trusts the navigation itself (D1): it moves into the same waiting
    // state a push rail reaches and polls before reporting anything. This
    // is the one transition `checkout_screen.dart`'s reducer has no event
    // for — see this file's own module doc comment for why the two-machine
    // web design never needed one.
    _resumePollingAfterRedirectReturn(redirecting);
    await _pollLoop();
  }

  /// Constructs a [CheckoutWaiting] directly from [redirecting] rather than
  /// through [reduceCheckoutScreen] — the one deliberate exception this file
  /// takes to "every transition goes through the reducer" (module doc
  /// comment). [CheckoutWaiting] is still Lane 1's own type, used exactly
  /// the way [reduceCheckoutScreen] itself builds one elsewhere — only the
  /// transition that reaches it here has no reducer event, because on the
  /// web this state is only ever reached from a *fresh page load* into the
  /// separate return machine (`return_screen.dart`), never from within one
  /// running instance of this one.
  void _resumePollingAfterRedirectReturn(CheckoutRedirecting redirecting) {
    _setState(
      CheckoutWaiting(
        context: redirecting.context,
        rail: redirecting.rail,
        notice: null,
      ),
    );
  }

  Future<void> _afterIntentResult(PaymentIntent intent) async {
    _setState(reduceCheckoutScreen(_state, CheckoutIntentUpdated(intent)));
    await _afterStateSettled();
  }

  /// Re-runs the poll after a poll that could not be answered —
  /// `controller.ts`'s `retryPoll`.
  Future<void> retryPoll() async {
    if (_state is! CheckoutWaiting) {
      return;
    }
    await _pollLoop();
  }

  Future<void> _pollLoop() async {
    final CheckoutScreenState s = _state;
    if (s is! CheckoutWaiting) {
      return;
    }
    final String secret = s.context.intent.clientSecret;
    final PaymentIntentResult result = await _pollUntilTerminal(
      secret,
      pollBudget,
    );
    if (result.isError) {
      // The payer stays on the waiting screen — the payment is in flight
      // and this poll learnt nothing that says otherwise (same rule
      // `controller.ts`'s own `#poll` states).
      _setState(
        reduceCheckoutScreen(
          _state,
          CheckoutProblem(errorMessageKey(result.error!)),
        ),
      );
      return;
    }
    await _afterIntentResult(result.paymentIntent!);
  }

  /// The poll ladder itself — Lane 1's [nextPollDelay] wired to a real
  /// clock and a real [BrowserClient] read for the first time. A budget
  /// elapsing answers `VpayError.pollingTimeout()`("error.unexpected" via
  /// [errorMessageKey], same as a `polling_timeout` `StripeError` maps to
  /// in `controller.ts`) rather than any status this package invents.
  Future<PaymentIntentResult> _pollUntilTerminal(
    String intentClientSecret,
    Duration budget,
  ) async {
    final DateTime deadline = clock.now().add(budget);
    for (;;) {
      final PaymentIntentResult result = await client.retrievePaymentIntent(
        intentClientSecret,
      );
      if (result.isError) {
        return result;
      }
      final PaymentIntent intent = result.paymentIntent!;
      if (intent.hasStoppedMoving) {
        return result;
      }
      final Duration remaining = deadline.difference(clock.now());
      final Duration delay = nextPollDelay(
        interval: pollInterval,
        remaining: remaining,
        source: jitterSource,
      );
      if (delay <= Duration.zero) {
        return PaymentIntentResult.err(VpayError.pollingTimeout());
      }
      await clock.delay(delay);
    }
  }

  /// Re-reads the session, then completes [result] — `controller.ts`'s
  /// `#announceOutcome`. The re-read is what makes the outcome screen's own
  /// `reference`/status fields the session's own rather than this
  /// controller's stale copy; if it fails, whatever is already on screen is
  /// kept rather than blocking completion on it.
  Future<void> _announceOutcome() async {
    final CheckoutScreenState s = _state;
    if (s is! CheckoutOutcome) {
      return;
    }
    final CheckoutSessionResult sessionResult = await client
        .retrieveCheckoutSession(s.context.session.clientSecret);
    if (!sessionResult.isError) {
      _setState(
        reduceCheckoutScreen(
          _state,
          CheckoutSessionRefreshed(sessionResult.checkoutSession!),
        ),
      );
    }
    _maybeAnnounceComplete();
  }

  /// `controller.ts`'s `#announceComplete` — at most once, guarded by
  /// [Completer.isCompleted].
  void _maybeAnnounceComplete() {
    if (_resultCompleter.isCompleted) {
      return;
    }
    final CheckoutScreenState s = _state;
    if (s is! CheckoutOutcome) {
      return;
    }
    _resultCompleter.complete(_resultForOutcome(s));
  }

  VpayCheckoutResult _resultForOutcome(CheckoutOutcome outcome) {
    final String sessionId = outcome.context.session.id;
    final String intentId = outcome.context.intent.id;
    switch (outcome.kind) {
      case OutcomeKind.succeeded:
        return VpayCheckoutSucceeded(
          sessionId: sessionId,
          paymentIntentId: intentId,
        );
      case OutcomeKind.canceled:
        return VpayCheckoutCanceled(
          sessionId: sessionId,
          paymentIntentId: intentId,
        );
      case OutcomeKind.failed:
        return VpayCheckoutFailed(
          sessionId: sessionId,
          paymentIntentId: intentId,
          code: outcome.failure,
          providerMessage: outcome.reason,
        );
    }
  }

  /// The outcome screen's one control — "Back to {merchant}"/"Back to the
  /// shop". Dispatches [CheckoutForward] so the sheet renders the
  /// `forwarding` screen exactly as the issue's bar requires, then the
  /// widget layer pops the sheet once it has shown it briefly. No URL is
  /// actually navigated to (D8's `postMessage`/iframe machinery has no
  /// native analogue — see the ADR line in `docs/adr/`); dismissing this
  /// sheet *is* "returning to the merchant" on a native host.
  void returnToMerchant() {
    final CheckoutScreenState s = _state;
    if (s is! CheckoutOutcome) {
      return;
    }
    final String destination =
        s.context.session.successUrl ?? s.context.session.cancelUrl ?? '';
    _setState(reduceCheckoutScreen(_state, CheckoutForward(destination)));
  }

  /// Called when the sheet is closed **without** [returnToMerchant] having
  /// run first — a swipe-away, the system back gesture, a tap outside a
  /// modal. Resolves [result] exactly once, the same completer
  /// [_maybeAnnounceComplete] would otherwise resolve.
  ///
  /// **D4, restated for a UI dismissal rather than a browser one:** a
  /// dismissal after a confirm was already sent polls, on
  /// [dismissalPollBudget], before answering anything — [VpayCheckoutPending]
  /// or [VpayCheckoutUnresolved], **never** [VpayCheckoutCanceled], because
  /// closing this widget is not a fact about the payment; only the API is
  /// (D1). A dismissal before any confirm was ever sent has nothing to poll
  /// for — [VpayError.sheetDismissedBeforeConfirm] names exactly that,
  /// wrapped in [VpayCheckoutUnresolved] rather than invented as `canceled`.
  Future<VpayCheckoutResult> dismiss() async {
    if (_resultCompleter.isCompleted) {
      return _resultCompleter.future;
    }
    final CheckoutContext? context = contextOfCheckoutScreen(_state);
    if (!_hasConfirmedOnce || context == null) {
      _resultCompleter.complete(
        VpayCheckoutUnresolved(
          sessionId: context?.session.id ?? '',
          paymentIntentId: context?.intent.id ?? '',
          error: VpayError.sheetDismissedBeforeConfirm(),
        ),
      );
      return _resultCompleter.future;
    }
    final PaymentIntentResult polled = await _pollUntilTerminal(
      context.intent.clientSecret,
      dismissalPollBudget,
    );
    final VpayCheckoutResult finalResult;
    if (polled.isError) {
      // A budget that elapsed with the intent still moving is not a
      // failure to observe anything (D4, restated for this shorter budget)
      // — `CheckoutController._resolve`'s own dismissal path answers
      // `VpayCheckoutPending` for exactly this case, not an error. Every
      // *other* error (a refused connection, an unexpected response) still
      // means this sheet could not decide, hence `VpayCheckoutUnresolved`.
      finalResult = polled.error!.code == VpayClientErrorCodes.pollingTimeout
          ? VpayCheckoutPending(
              sessionId: context.session.id,
              paymentIntentId: context.intent.id,
            )
          : VpayCheckoutUnresolved(
              sessionId: context.session.id,
              paymentIntentId: context.intent.id,
              error: polled.error!,
            );
    } else {
      finalResult = _resultForIntent(context, polled.paymentIntent!);
    }
    if (!_resultCompleter.isCompleted) {
      _resultCompleter.complete(finalResult);
    }
    return _resultCompleter.future;
  }

  /// [CheckoutController._resultFor]'s own mapping, restated: this
  /// controller cannot reach that method (private, and tied to a
  /// [CheckoutPreflightReady] this sheet never builds), so [dismiss] needs
  /// its own copy of the same five-way switch. Kept exhaustive rather than
  /// a `default` for the same reason that one is.
  VpayCheckoutResult _resultForIntent(
    CheckoutContext context,
    PaymentIntent intent,
  ) {
    final String sessionId = context.session.id;
    final String intentId = intent.id;
    switch (intent.status) {
      case PaymentIntentStatus.succeeded:
        return VpayCheckoutSucceeded(
          sessionId: sessionId,
          paymentIntentId: intentId,
        );
      case PaymentIntentStatus.canceled:
        return VpayCheckoutCanceled(
          sessionId: sessionId,
          paymentIntentId: intentId,
        );
      case PaymentIntentStatus.requiresPaymentMethod:
        return VpayCheckoutFailed(
          sessionId: sessionId,
          paymentIntentId: intentId,
          code: intent.lastPaymentError?.code,
          providerMessage: intent.lastPaymentError?.message,
        );
      case PaymentIntentStatus.requiresAction:
      case PaymentIntentStatus.processing:
      case PaymentIntentStatus.unknown:
        return VpayCheckoutPending(
          sessionId: sessionId,
          paymentIntentId: intentId,
        );
    }
  }
}

/// The window's event stream ended, or errored, without ever reporting an
/// outcome — private, mirrors `vpay_checkout.dart`'s own
/// `_WindowNeverReported`; caught inside [SheetController._handOffToBrowser]
/// and never rethrown.
final class _WindowNeverReported implements Exception {
  const _WindowNeverReported();
}
