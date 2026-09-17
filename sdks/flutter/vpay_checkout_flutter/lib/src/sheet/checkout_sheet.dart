/// The native checkout sheet — issue #189, Lane 2: `VpayCheckoutSheet`, the
/// widget every one of the 13 screen states above renders as, plus the two
/// presentation helpers the maintainer asked for ("both" — a
/// `showModalBottomSheet` at a large detent *and* a full-route push).
///
/// **Theming: inherits `ThemeData`.** Nothing here reads a
/// `VpayCheckoutTheme` or paints a fixed vpay palette — every colour comes
/// from `Theme.of(context)` (`Material`/`Text`/`ElevatedButton` defaults),
/// so the sheet looks like the host app.
///
/// **No rail code is branched on here either** — `test/sheet/no_rail_code_branching_test.dart`
/// scans this file the same way it scans every other one under `lib/`.
/// Every rail-specific string this file shows comes from [RailSpec.labelKey]
/// through [railLabelFor], never from an `if` on [RailSpec.code].
library;

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:http/http.dart' as http;

import '../browser_client.dart';
import '../errors.dart';
import '../models.dart';
import '../result.dart';
import 'checkout_screen.dart';
import 'i18n.dart';
import 'money_format.dart';
import 'rails.dart';
import 'sheet_controller.dart';

/// `{base}/c/{cs_id}?key=…#{cs_secret}`'s fragment — the Dart port of
/// `vpay_checkout.dart`'s own (private) `_SessionUrl.parse`, restated here
/// rather than made public there: this is the sheet's own entry point, and
/// [SheetController] takes a `client_secret` directly rather than a whole
/// URL, exactly as `CheckoutController.preflight` already does.
String _sessionClientSecretFromUrl(String sessionUrl) {
  final int hash = sessionUrl.indexOf('#');
  if (hash == -1 || hash == sessionUrl.length - 1) {
    return '';
  }
  return sessionUrl.substring(hash + 1);
}

/// The native checkout sheet.
///
/// Reads `rails` off the session, renders the declared fields, confirms
/// through [BrowserClient], polls through [SheetController]'s own jittered
/// loop, and hands a `flow: "redirect"` rail off to the platform browser
/// host — see [SheetController]'s own module doc comment for the ordering
/// guarantees.
///
/// Works equally inside [showVpayCheckoutSheet] (a large-detent, draggable
/// `showModalBottomSheet`) and [showVpayCheckoutSheetRoute] (a full route)
/// — nothing here assumes a fixed height or a particular `Navigator`
/// ancestor beyond "this widget is on one and can be popped".
class VpayCheckoutSheet extends StatefulWidget {
  const VpayCheckoutSheet({
    super.key,
    required this.sessionUrl,
    required this.baseUrl,
    required this.publishableKey,
    this.allowInsecureBaseUrl = false,
    this.merchantName,
    this.allowedMethods,
    this.locale = VpayLocale.fallback,
    this.showDragHandle = false,
    this.httpClient,
  });

  /// `{base}/c/{cs_id}?key=…#{cs_secret}` — the same string
  /// `VpayCheckout.start` takes.
  final String sessionUrl;
  final String baseUrl;
  final String publishableKey;
  final bool allowInsecureBaseUrl;

  /// A caller-supplied merchant display name — [CheckoutContext.merchantName]'s
  /// own doc comment: `CheckoutSession` carries none today.
  final String? merchantName;
  final List<String>? allowedMethods;

  /// French by default (issue #189: "French is Cameroon's and Orange's
  /// language — defaulting to English would be a regression").
  final VpayLocale locale;

  /// Whether to paint a drag handle at the top — `true` from
  /// [showVpayCheckoutSheet], `false` from [showVpayCheckoutSheetRoute],
  /// where a full route already has its own chrome for "this can be
  /// dismissed".
  final bool showDragHandle;

  /// Injectable, exactly as [VpayCheckout]'s own constructor takes one —
  /// `null` builds a real `http.Client()`. A widget test substitutes a
  /// `MockClient` here rather than this widget ever reaching the network.
  final http.Client? httpClient;

  @override
  State<VpayCheckoutSheet> createState() => _VpayCheckoutSheetState();
}

/// A `showModalBottomSheet` at a large (~90%), draggable detent — the
/// "popup, not iframe" native equivalent: the merchant app stays visible
/// behind it (`isScrollControlled` + a `DraggableScrollableSheet` rather
/// than a fixed-height sheet, so the payer can see and drag past it).
///
/// Resolves once the sheet is popped — either by [VpayCheckoutSheet]
/// itself, once the payer presses the outcome screen's one button, or by
/// the payer dismissing the sheet (a swipe-away, tapping outside, the
/// system back gesture), in which case [SheetController.dismiss] decides
/// the answer (D4).
Future<VpayCheckoutResult> showVpayCheckoutSheet(
  BuildContext context, {
  required String sessionUrl,
  required String baseUrl,
  required String publishableKey,
  bool allowInsecureBaseUrl = false,
  String? merchantName,
  List<String>? allowedMethods,
  VpayLocale locale = VpayLocale.fallback,
}) async {
  final GlobalKey<_VpayCheckoutSheetState> sheetKey =
      GlobalKey<_VpayCheckoutSheetState>();
  final VpayCheckoutResult? popped =
      await showModalBottomSheet<VpayCheckoutResult>(
        context: context,
        isScrollControlled: true,
        useSafeArea: true,
        shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(16)),
        ),
        builder: (BuildContext sheetContext) => DraggableScrollableSheet(
          initialChildSize: 0.9,
          minChildSize: 0.5,
          maxChildSize: 0.95,
          expand: false,
          builder: (BuildContext _, ScrollController scrollController) =>
              VpayCheckoutSheet(
                key: sheetKey,
                sessionUrl: sessionUrl,
                baseUrl: baseUrl,
                publishableKey: publishableKey,
                allowInsecureBaseUrl: allowInsecureBaseUrl,
                merchantName: merchantName,
                allowedMethods: allowedMethods,
                locale: locale,
                showDragHandle: true,
              ),
        ),
      );
  if (popped != null) {
    return popped;
  }
  // The sheet closed without an explicit pop(result) — a swipe-away, a tap
  // outside, the system back gesture. `dismiss()` decides the answer (D4):
  // never a synthesised `canceled`.
  final _VpayCheckoutSheetState? state = sheetKey.currentState;
  if (state != null) {
    return state._controller.dismiss();
  }
  // The widget never even built (a build error before `initState`, or the
  // sheet was closed before the very first frame) — nothing was observed.
  return VpayCheckoutUnresolved(
    sessionId: '',
    paymentIntentId: '',
    error: VpayError.sheetDismissedBeforeConfirm(),
  );
}

/// The full-route presentation — a plain [MaterialPageRoute] rather than a
/// bottom sheet, for a host that wants the checkout to occupy the whole
/// screen. Same widget, same resolution rule as [showVpayCheckoutSheet].
Future<VpayCheckoutResult> showVpayCheckoutSheetRoute(
  BuildContext context, {
  required String sessionUrl,
  required String baseUrl,
  required String publishableKey,
  bool allowInsecureBaseUrl = false,
  String? merchantName,
  List<String>? allowedMethods,
  VpayLocale locale = VpayLocale.fallback,
}) async {
  final GlobalKey<_VpayCheckoutSheetState> sheetKey =
      GlobalKey<_VpayCheckoutSheetState>();
  final VpayCheckoutResult? popped = await Navigator.of(context).push(
    MaterialPageRoute<VpayCheckoutResult>(
      builder: (BuildContext _) => VpayCheckoutSheet(
        key: sheetKey,
        sessionUrl: sessionUrl,
        baseUrl: baseUrl,
        publishableKey: publishableKey,
        allowInsecureBaseUrl: allowInsecureBaseUrl,
        merchantName: merchantName,
        allowedMethods: allowedMethods,
        locale: locale,
      ),
    ),
  );
  if (popped != null) {
    return popped;
  }
  final _VpayCheckoutSheetState? state = sheetKey.currentState;
  if (state != null) {
    return state._controller.dismiss();
  }
  return VpayCheckoutUnresolved(
    sessionId: '',
    paymentIntentId: '',
    error: VpayError.sheetDismissedBeforeConfirm(),
  );
}

class _VpayCheckoutSheetState extends State<VpayCheckoutSheet> {
  late final SheetController _controller;
  late final VpayCheckoutStrings _t;
  final FocusNode _headingFocusNode = FocusNode();
  final TextEditingController _msisdnController = TextEditingController();
  bool _msisdnManuallyEdited = false;
  String? _lastAnnouncedScreen;
  bool _popRequested = false;

  bool get _frenchLocale => widget.locale == VpayLocale.fr;

  @override
  void initState() {
    super.initState();
    _t = VpayCheckoutStrings(widget.locale);
    _controller = SheetController(
      client: BrowserClient(
        baseUrl: widget.baseUrl,
        publishableKey: widget.publishableKey,
        allowInsecureBaseUrl: widget.allowInsecureBaseUrl,
        httpClient: widget.httpClient,
      ),
      sessionClientSecret: _sessionClientSecretFromUrl(widget.sessionUrl),
      merchantName: widget.merchantName,
      allowedMethods: widget.allowedMethods,
    );
    _controller.addListener(_onControllerChanged);
    unawaited(_controller.start());
  }

  @override
  void dispose() {
    _controller.removeListener(_onControllerChanged);
    _controller.dispose();
    _headingFocusNode.dispose();
    _msisdnController.dispose();
    super.dispose();
  }

  void _onControllerChanged() {
    final String screen = _screenTag(_controller.state);
    if (screen != _lastAnnouncedScreen) {
      _lastAnnouncedScreen = screen;
      _msisdnManuallyEdited = false;
      final String? prefill = _controller.defaultMsisdn;
      if (prefill != null &&
          !_msisdnManuallyEdited &&
          _msisdnController.text.isEmpty) {
        _msisdnController.text = prefill;
      }
      final String heading = _titleFor(_controller.state, widget.locale);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) {
          return;
        }
        // Each screen heading is focused on transition (issue #189's own
        // bar item) — the Flutter equivalent of the hosted page's
        // `document.querySelector('[data-screen]')?.focus()`.
        _headingFocusNode.requestFocus();
        // The live region (below, in `build`) is mounted once and never
        // unmounted; this is the *announcement* half —
        // `SemanticsService.sendAnnouncement` for a state-driven transition
        // text, since a region created together with its own text is not
        // reliably announced either.
        SemanticsService.sendAnnouncement(
          View.of(context),
          heading,
          TextDirection.ltr,
        );
      });
    }
    if (!_popRequested && _controller.state is CheckoutForwarding) {
      _popRequested = true;
      unawaited(_popAfterForwarding());
    }
    setState(() {});
  }

  Future<void> _popAfterForwarding() async {
    final VpayCheckoutResult result = await _controller.result;
    // A brief, real delay — not a countdown a payer watches (issue #189:
    // "no countdown timer" is about the *outcome* screen's own control,
    // which stays one button with no timer beside it; this is the
    // forwarding screen's own transient display before this widget closes
    // itself, mirroring how fast a real top-level navigation is on the web).
    await Future<void>.delayed(const Duration(milliseconds: 350));
    if (!mounted) {
      return;
    }
    Navigator.of(context).maybePop(result);
  }

  @override
  Widget build(BuildContext context) {
    return Material(
      color: Theme.of(context).scaffoldBackgroundColor,
      child: SafeArea(
        top: false,
        child: Semantics(
          // Present from first render, never unmounted — a live region
          // created together with its own text is not announced.
          container: true,
          liveRegion: true,
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(20, 12, 20, 20),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[
                if (widget.showDragHandle) _dragHandle(context),
                _buildScreen(context),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _dragHandle(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 12),
    child: Center(
      child: Container(
        width: 36,
        height: 4,
        decoration: BoxDecoration(
          color: Theme.of(context).dividerColor,
          borderRadius: BorderRadius.circular(2),
        ),
      ),
    ),
  );

  Widget _heading(String text) => Focus(
    focusNode: _headingFocusNode,
    child: Text(
      text,
      style: Theme.of(context).textTheme.titleLarge
          ?.copyWith(fontWeight: FontWeight.w600),
    ),
  );

  Widget _testModeBanner(bool livemode) {
    if (livemode) {
      return const SizedBox.shrink();
    }
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Text(
        _t.t('page.testmode'),
        style: Theme.of(context).textTheme.bodySmall
            ?.copyWith(fontWeight: FontWeight.w600),
      ),
    );
  }

  Widget _summary(CheckoutContext context) {
    final CheckoutSession session = context.session;
    final PaymentIntent intent = context.intent;
    final String amount = formatAmountForDisplay(
      intent.amount,
      intent.currency,
      _frenchLocale,
    );
    return Card(
      margin: const EdgeInsets.only(bottom: 16),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: <Widget>[
            _testModeBanner(session.livemode),
            Text(
              _merchantLine(
                context.merchantName,
                'page.pay_to',
                'page.pay_to_unnamed',
              ),
              style: Theme.of(this.context).textTheme.bodyMedium,
            ),
            const SizedBox(height: 4),
            Text(
              amount,
              style: Theme.of(this.context).textTheme.headlineSmall
                  ?.copyWith(fontWeight: FontWeight.w700),
            ),
            const SizedBox(height: 4),
            Text(
              '${_t.t('page.reference_label')}: ${session.id}',
              style: Theme.of(this.context).textTheme.bodySmall,
            ),
          ],
        ),
      ),
    );
  }

  String _merchantLine(
    String? merchant,
    String namedKey,
    String unnamedKey, [
    Map<String, String> values = const <String, String>{},
  ]) => merchant == null
      ? _t.t(unnamedKey, values)
      : _t.t(namedKey, {...values, 'merchant': merchant});

  Widget _buildScreen(BuildContext context) {
    final CheckoutScreenState s = _controller.state;
    final CheckoutContext? ctx = contextOfCheckoutScreen(s);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        if (ctx != null) _summary(ctx),
        switch (s) {
          CheckoutLoading() => _statusPanel(
            title: _t.t('state.loading'),
            body: null,
          ),
          CheckoutLoadError(:final error) => _noticePanel(
            title: _t.t('error.title'),
            body: _t.t(errorMessageKey(error)),
            code: error.code,
          ),
          CheckoutRefused(:final reason) =>
            reason == RefusalReason.embedNotAllowed
                ? _noticePanel(
                    title: _t.t('refusal.embed_title'),
                    body: _t.t('refusal.embed_body'),
                  )
                : _noticePanel(
                    title: _t.t('error.title'),
                    body: _t.t('rail.none'),
                  ),
          CheckoutExpired() => _noticePanel(
            title: _t.t('expired.title'),
            body: _merchantLine(
              ctx?.merchantName,
              'expired.body',
              'expired.body_unnamed',
            ),
          ),
          CheckoutSelectRail(:final rails) => _railSelector(rails),
          CheckoutCollectMsisdn(
            :final rail,
            :final rails,
            :final problem,
            :final context,
          ) =>
            _msisdnForm(
              rail: rail,
              problem: problem,
              canGoBack: rails.supported.length > 1,
              amount: formatAmountForDisplay(
                context.intent.amount,
                context.intent.currency,
                _frenchLocale,
              ),
            ),
          CheckoutReadyRedirect(
            :final rail,
            :final rails,
            :final problem,
            :final context,
          ) =>
            _redirectPrompt(
              rail: rail,
              problem: problem,
              canGoBack: rails.supported.length > 1,
              amount: formatAmountForDisplay(
                context.intent.amount,
                context.intent.currency,
                _frenchLocale,
              ),
            ),
          CheckoutConfirming() => _statusPanel(
            title: _t.t('state.confirming'),
            body: null,
          ),
          CheckoutWaiting(:final notice, :final context) => _statusPanel(
            title: _t.t('state.waiting_title'),
            body: _t.t('state.waiting_body', {
              'amount': formatAmountForDisplay(
                context.intent.amount,
                context.intent.currency,
                _frenchLocale,
              ),
            }),
            notice: notice,
            onRetry: notice == null ? null : _controller.retryPoll,
          ),
          CheckoutRedirecting() => _statusPanel(
            title: _t.t('state.redirecting_title'),
            body: _t.t('state.redirecting_body'),
          ),
          CheckoutOutcome() => _outcomePanel(s),
          CheckoutForwarding() => _statusPanel(
            title: _t.t('state.forwarding_title'),
            body: _merchantLine(
              ctx?.merchantName,
              'state.forwarding_body',
              'state.forwarding_body_unnamed',
            ),
          ),
        },
      ],
    );
  }

  Widget _statusPanel({
    required String title,
    required String? body,
    String? notice,
    VoidCallback? onRetry,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: <Widget>[
      _heading(title),
      const SizedBox(height: 12),
      if (body != null) Text(body),
      const SizedBox(height: 16),
      const Center(child: CircularProgressIndicator()),
      if (notice != null) ...<Widget>[
        const SizedBox(height: 16),
        Semantics(
          liveRegion: true,
          child: Text(
            _t.t(notice),
            style: const TextStyle(fontWeight: FontWeight.w600),
          ),
        ),
        const SizedBox(height: 8),
        if (onRetry != null)
          OutlinedButton(onPressed: onRetry, child: Text(_t.t('error.retry'))),
      ],
    ],
  );

  Widget _noticePanel({
    required String title,
    required String body,
    String? code,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: <Widget>[
      _heading(title),
      const SizedBox(height: 12),
      Container(
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.errorContainer,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Text(
          body,
          style: TextStyle(
            color: Theme.of(context).colorScheme.onErrorContainer,
          ),
        ),
      ),
    ],
  );

  Widget _railSelector(RailChoices rails) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: <Widget>[
      _heading(_t.t('rail.legend')),
      const SizedBox(height: 12),
      for (final SupportedRail rail in rails.supported) ...<Widget>[
        OutlinedButton(
          onPressed: () => _controller.chooseRail(rail),
          child: Text(_labelFor(rail)),
        ),
        const SizedBox(height: 8),
      ],
      if (rails.unsupported.isNotEmpty) ...<Widget>[
        const SizedBox(height: 8),
        for (final UnsupportedRail u in rails.unsupported)
          Text(
            _t.t('rail.unsupported', {'rail': u.code}),
            style: Theme.of(context).textTheme.bodySmall,
          ),
      ],
    ],
  );

  /// [RailSpec.labelKey] resolved through the catalogue, falling back to
  /// the deployment's own [RailDisplayName] and then the raw code — never
  /// a rail code branched on to choose between them (`railLabelFor`'s own
  /// doc comment).
  String _labelFor(SupportedRail rail) => railLabelFor(
    locale: widget.locale,
    labelKey: rail.spec.labelKey,
    code: rail.code,
    configuredEn: rail.spec.displayName?.en,
    configuredFr: rail.spec.displayName?.fr,
  );

  Widget _rememberControl({required String label}) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: <Widget>[
      // `CheckboxListTile` merges its `title`/`subtitle` into ONE
      // semantics node's accessible label (Flutter's own
      // `ListTile`/`MergeSemantics` behaviour) — this is how the
      // shared-phone warning ends up *inside* the accessible label
      // rather than as a separate, easily-missed description, the same
      // property `screens.tsx`'s own `CheckboxField` composition
      // documents choosing for the same reason.
      CheckboxListTile(
        value: _controller.rememberChecked,
        onChanged: (bool? value) =>
            _controller.setRememberChecked(value ?? false),
        controlAffinity: ListTileControlAffinity.leading,
        contentPadding: EdgeInsets.zero,
        title: Text(label),
        subtitle: Text(_t.t('memory.warning')),
      ),
      if (_controller.hasRememberedRecord)
        Align(
          alignment: Alignment.centerLeft,
          child: TextButton(
            onPressed: () async {
              await _controller.forgetRemembered();
              setState(() {});
            },
            child: Text(_t.t('memory.forget')),
          ),
        ),
      if (_controller.forgotten)
        Padding(
          padding: const EdgeInsets.only(left: 12),
          child: Text(
            _t.t('memory.forgotten'),
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ),
    ],
  );

  Widget _msisdnForm({
    required SupportedRail rail,
    required String? problem,
    required bool canGoBack,
    required String amount,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: <Widget>[
      _heading(_labelFor(rail)),
      const SizedBox(height: 12),
      TextField(
        controller: _msisdnController,
        keyboardType: TextInputType.phone,
        autofillHints: const <String>[AutofillHints.telephoneNumber],
        onChanged: (_) => _msisdnManuallyEdited = true,
        decoration: InputDecoration(
          labelText: _t.t('msisdn.label'),
          helperText: _t.t('msisdn.hint'),
          errorText: problem == null ? null : _t.t(problem),
        ),
      ),
      const SizedBox(height: 12),
      _rememberControl(label: _t.t('memory.remember_number')),
      const SizedBox(height: 12),
      ElevatedButton(
        onPressed: () => _controller.submitMsisdn(_msisdnController.text),
        child: Text(_t.t('msisdn.submit', {'amount': amount})),
      ),
      if (canGoBack) ...<Widget>[
        const SizedBox(height: 8),
        TextButton(
          onPressed: _controller.back,
          child: Text(_t.t('msisdn.back')),
        ),
      ],
    ],
  );

  Widget _redirectPrompt({
    required SupportedRail rail,
    required String? problem,
    required bool canGoBack,
    required String amount,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: <Widget>[
      _heading(_labelFor(rail)),
      const SizedBox(height: 8),
      Text(_t.t('state.redirecting_body')),
      if (problem != null) ...<Widget>[
        const SizedBox(height: 8),
        Text(
          _t.t(problem),
          style: TextStyle(color: Theme.of(context).colorScheme.error),
        ),
      ],
      const SizedBox(height: 12),
      _rememberControl(
        label: _t.t('memory.remember_method', {'rail': _labelFor(rail)}),
      ),
      const SizedBox(height: 12),
      ElevatedButton(
        onPressed: _controller.startRedirect,
        child: Text(_t.t('msisdn.submit', {'amount': amount})),
      ),
      if (canGoBack) ...<Widget>[
        const SizedBox(height: 8),
        TextButton(
          onPressed: _controller.back,
          child: Text(_t.t('msisdn.back')),
        ),
      ],
    ],
  );

  Widget _outcomePanel(CheckoutOutcome outcome) {
    final String title = switch (outcome.kind) {
      OutcomeKind.succeeded => _t.t('outcome.succeeded_title'),
      OutcomeKind.canceled => _t.t('outcome.canceled_title'),
      OutcomeKind.failed => _t.t('outcome.failed_title'),
    };
    final String amount = formatAmountForDisplay(
      outcome.context.intent.amount,
      outcome.context.intent.currency,
      _frenchLocale,
    );
    final String body = switch (outcome.kind) {
      OutcomeKind.succeeded => _merchantLine(
        outcome.context.merchantName,
        'outcome.succeeded_body',
        'outcome.succeeded_body_unnamed',
        {'amount': amount},
      ),
      OutcomeKind.canceled => _t.t('outcome.canceled_body'),
      OutcomeKind.failed => _t.t(
        failureMessageKey(outcome.failure) ?? 'failure.unknown',
      ),
    };
    final String? reason = outcome.reason;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _heading(title),
        const SizedBox(height: 12),
        Semantics(
          liveRegion: true,
          child: Container(
            padding: const EdgeInsets.all(12),
            decoration: BoxDecoration(
              color: _outcomeColor(context, outcome.kind),
              borderRadius: BorderRadius.circular(8),
            ),
            child: Text(body),
          ),
        ),
        // `providerReason` is shown BESIDE, never instead of, the
        // translated failure line above — issue #189's own bar item.
        if (reason != null) ...<Widget>[
          const SizedBox(height: 8),
          Text(
            '${_t.t('outcome.provider_said')}: $reason',
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ],
        const SizedBox(height: 16),
        // The hosted page's "outcome with no destination" screen
        // (`outcome.no_destination`, shown when neither `success_url` nor
        // `cancel_url` is configured, so the page has genuinely nowhere to
        // navigate) has no equivalent here, by deliberate decision: a
        // native sheet always has a way back — dismissing it, which this
        // button does via `returnToMerchant` — regardless of whether the
        // session configured a URL. `outcome.no_destination` stays in the
        // catalogue (so a caller that does need it, e.g. a future
        // "session has no destination" refinement, has the string) but this
        // widget never reaches it. Recorded in
        // `docs/flows/mobile-checkout.md` rather than dropped silently.
        //
        // No countdown timer beside this either — issue #189's own bar
        // item. One button, and the payer decides when to press it.
        ElevatedButton(
          onPressed: _controller.returnToMerchant,
          child: Text(
            _merchantLine(
              outcome.context.merchantName,
              'outcome.back_to',
              'outcome.back_to_unnamed',
            ),
          ),
        ),
      ],
    );
  }

  Color _outcomeColor(BuildContext context, OutcomeKind kind) {
    final ColorScheme scheme = Theme.of(context).colorScheme;
    return switch (kind) {
      OutcomeKind.succeeded => scheme.primaryContainer,
      OutcomeKind.canceled => scheme.surfaceContainerHighest,
      OutcomeKind.failed => scheme.errorContainer,
    };
  }
}

/// A stable identity per screen state, for [_VpayCheckoutSheetState]'s own
/// focus-on-transition and announce logic — the Dart analogue of
/// `screens.tsx`'s `data-screen` attribute, which the hosted page's own
/// `ScreenHeading` focuses by querying.
String _screenTag(CheckoutScreenState s) => switch (s) {
  CheckoutLoading() => 'loading',
  CheckoutLoadError() => 'error',
  CheckoutRefused(:final reason) =>
    reason == RefusalReason.embedNotAllowed ? 'refused_embed' : 'refused_rail',
  CheckoutExpired() => 'expired',
  CheckoutSelectRail() => 'select_rail',
  CheckoutCollectMsisdn() => 'collect_msisdn',
  CheckoutReadyRedirect() => 'ready_redirect',
  CheckoutConfirming() => 'confirming',
  CheckoutWaiting() => 'waiting',
  CheckoutRedirecting() => 'redirecting',
  CheckoutOutcome() => 'outcome',
  CheckoutForwarding() => 'forwarding',
};

/// The heading text for [s] — used only for [SemanticsService.sendAnnouncement];
/// kept intentionally small and separate from [_VpayCheckoutSheetState._buildScreen]'s
/// own titles so a caller reading just the announce logic does not have to
/// read the whole render switch. A momentary mismatch between the two would
/// degrade an announcement's wording, never break navigation or a payment.
String _titleFor(CheckoutScreenState s, VpayLocale locale) {
  final VpayCheckoutStrings t = VpayCheckoutStrings(locale);
  return switch (s) {
    CheckoutLoading() => t.t('state.loading'),
    CheckoutLoadError() => t.t('error.title'),
    CheckoutRefused(:final reason) =>
      reason == RefusalReason.embedNotAllowed
          ? t.t('refusal.embed_title')
          : t.t('error.title'),
    CheckoutExpired() => t.t('expired.title'),
    CheckoutSelectRail() => t.t('rail.legend'),
    CheckoutCollectMsisdn() => t.t('msisdn.label'),
    CheckoutReadyRedirect() => t.t('state.redirecting_title'),
    CheckoutConfirming() => t.t('state.confirming'),
    CheckoutWaiting() => t.t('state.waiting_title'),
    CheckoutRedirecting() => t.t('state.redirecting_title'),
    CheckoutOutcome(:final kind) => switch (kind) {
      OutcomeKind.succeeded => t.t('outcome.succeeded_title'),
      OutcomeKind.canceled => t.t('outcome.canceled_title'),
      OutcomeKind.failed => t.t('outcome.failed_title'),
    },
    CheckoutForwarding() => t.t('state.forwarding_title'),
  };
}
