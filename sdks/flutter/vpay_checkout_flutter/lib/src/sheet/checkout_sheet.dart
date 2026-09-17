/// The native checkout sheet — issue #189, Lane 2: `VpayCheckoutSheet`, the
/// widget every one of the 13 screen states above renders as, plus the two
/// presentation helpers the maintainer asked for ("both" — a
/// `showModalBottomSheet` at a large detent *and* a full-route push).
///
/// **Theming: Material 3, through [VpayCheckoutTheme].** The sheet still
/// paints no vpay palette of its own — there is no brand colour anywhere
/// in this file, and every colour it draws is a [ColorScheme] role or a
/// [TextTheme] role resolved from the ambient theme.
///
/// What changed on 2026-09-17 is only *whose* colour wins. This comment
/// used to say "nothing here reads a `VpayCheckoutTheme`"; one now exists,
/// and a deployment's `primary_color` may seed the scheme. The precedence
/// — explicit scheme, explicit seed, deployment colour, then the host
/// app's own `ThemeData` unchanged — is written out in
/// [VpayCheckoutTheme.resolve]. A host that passes no theme and deploys no
/// brand colour gets the inherited appearance it always got.
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
import '../config/checkout_page_config.dart';
import '../errors.dart';
import '../models.dart';
import '../result.dart';
import 'checkout_screen.dart';
import 'checkout_theme.dart';
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
    this.configStore,
    this.locale = VpayLocale.fallback,
    this.showDragHandle = false,
    this.theme = const VpayCheckoutTheme(),
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

  /// A caller's own rail allow-list. Issue #193: this is no longer the
  /// final word — it is narrowed further against
  /// `checkout.allowed_methods`, the operator's own floor, read out of the
  /// locally-cached [CheckoutPageConfig] before this sheet's controller is
  /// started (see `_bootstrap` below and [narrowAllowedMethods]). `null`
  /// here means "this caller has no opinion", not "allow everything" —
  /// the operator's floor still applies.
  final List<String>? allowedMethods;

  /// Where the cached [CheckoutPageConfig] [prepareCheckout] wrote is read
  /// from. `null` uses [defaultCheckoutPageConfigStore] — the same
  /// package-level in-memory store a `prepareCheckout` call made with no
  /// `store` argument writes to, so the zero-configuration path (call
  /// `prepareCheckout()`, then open this sheet, neither naming a store)
  /// shares one cache without a caller wiring anything together itself.
  final VpayCheckoutConfigStore? configStore;

  /// French by default (issue #189: "French is Cameroon's and Orange's
  /// language — defaulting to English would be a regression").
  final VpayLocale locale;

  /// Whether to paint a drag handle at the top — `true` from
  /// [showVpayCheckoutSheet], `false` from [showVpayCheckoutSheetRoute],
  /// where a full route already has its own chrome for "this can be
  /// dismissed".
  final bool showDragHandle;

  /// How the sheet looks: the Material 3 colour, shape and density
  /// surface. The default is the stock M3 sheet, seeded by the
  /// deployment's `primary_color` when one is published — see
  /// [VpayCheckoutTheme.resolve] for the precedence.
  final VpayCheckoutTheme theme;

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
  VpayCheckoutConfigStore? configStore,
  VpayLocale locale = VpayLocale.fallback,
  VpayCheckoutTheme theme = const VpayCheckoutTheme(),
  BorderRadiusGeometry? borderRadius,
}) async {
  final GlobalKey<_VpayCheckoutSheetState> sheetKey =
      GlobalKey<_VpayCheckoutSheetState>();
  final VpayCheckoutResult? popped =
      await showModalBottomSheet<VpayCheckoutResult>(
        context: context,
        isScrollControlled: true,
        useSafeArea: true,
        // Load-bearing, and its absence is why this sheet had square
        // corners despite already setting `shape`: `showModalBottomSheet`
        // does not clip its child to `shape` unless asked. The sheet's own
        // `Material` then paints an opaque rectangle straight over the
        // rounded outline, so the radius exists in the widget tree and
        // never on screen.
        clipBehavior: Clip.antiAlias,
        shape: RoundedRectangleBorder(
          // `borderRadius` stays the sharper override of the two: a
          // caller that passes it is naming the exact geometry, so it
          // wins over the theme's scalar.
          borderRadius:
              borderRadius ??
              BorderRadius.vertical(
                top: Radius.circular(theme.sheetCornerRadius),
              ),
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
                configStore: configStore,
                locale: locale,
                showDragHandle: true,
                theme: theme,
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
  VpayCheckoutConfigStore? configStore,
  VpayLocale locale = VpayLocale.fallback,
  VpayCheckoutTheme theme = const VpayCheckoutTheme(),
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
        configStore: configStore,
        locale: locale,
        theme: theme,
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

  /// `branding.support_contact`, read out of the locally-cached
  /// [CheckoutPageConfig] by [_bootstrap] — `null` until that read settles
  /// (almost always within the same frame; it is a local cache read, never
  /// a network call) and whenever the document carries none.
  String? _supportContact;

  /// `branding.primary_color` from the same cached [CheckoutPageConfig]
  /// [_bootstrap] reads, or `null` when the deployment published none.
  /// Seeds the M3 [ColorScheme] unless the caller's [VpayCheckoutTheme]
  /// outranks it — [VpayCheckoutTheme.resolve] owns that decision.
  ///
  /// Null on the first frame, like [_supportContact]: the config read is
  /// a local cache hit but still a `Future`, so the very first paint uses
  /// the host app's own scheme and the branded one arrives with the
  /// `setState` below. A payer sees a theme settle, never a blank sheet.
  Color? _brandSeed;

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
    unawaited(_bootstrap());
  }

  /// Resolves the cached [CheckoutPageConfig] and applies it — narrowing
  /// [SheetController.allowedMethods] to the operator's own floor,
  /// picking up `support_contact` for [_supportContact] — **before**
  /// [SheetController.start] ever reads either, and only then starts it.
  ///
  /// [resolveCheckoutPageConfig] only ever reads a local cache
  /// ([widget.configStore] or the package's shared in-memory default); it
  /// never touches the network — `prepareCheckout` is this SDK's one and
  /// only network fetch for this document (issue #193's governing rule:
  /// the sheet must still work with an empty cache and no network at all,
  /// which holds here because this `await` cannot fail on either). A
  /// caller that never called `prepareCheckout` simply gets
  /// [CheckoutPageConfig.defaults] back, promptly.
  Future<void> _bootstrap() async {
    final CheckoutPageConfig config = await resolveCheckoutPageConfig(
      store: widget.configStore,
    );
    if (!mounted) {
      return;
    }
    _supportContact = config.branding.supportContact;
    _brandSeed = config.branding.primaryColor;
    _controller.allowedMethods = narrowAllowedMethods(
      explicit: widget.allowedMethods,
      operatorFloor: config.checkout.allowedMethods,
    );
    await _controller.start();
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

  /// The sheet's own Material 3 theme, resolved once per build.
  ///
  /// Everything below takes its `BuildContext` from the [Builder] under
  /// this [Theme], never from [State.context]. That is not a style
  /// preference: `State.context` sits *above* the `Theme` this method
  /// installs, so a helper reading `Theme.of(this.context)` would quietly
  /// get the host app's scheme and ignore the seeded one — the sheet
  /// would look themed in the widget tree and unthemed on screen.
  ThemeData _sheetTheme(BuildContext context) =>
      widget.theme.resolve(Theme.of(context), deploymentBrandColor: _brandSeed);

  @override
  Widget build(BuildContext context) {
    final ThemeData theme = _sheetTheme(context);
    return Theme(
      data: theme,
      child: Material(
        // `surfaceContainerLow`, not `scaffoldBackgroundColor`: M3 gives
        // a bottom sheet a container role a step up the tonal ladder from
        // the page behind it, which is what separates the two surfaces
        // without a border or a shadow.
        color: theme.colorScheme.surfaceContainerLow,
        child: SafeArea(
          top: false,
          child: Semantics(
            // Present from first render, never unmounted — a live region
            // created together with its own text is not announced.
            container: true,
            liveRegion: true,
            child: Builder(
              builder: (BuildContext inner) => SingleChildScrollView(
                padding: widget.theme.contentPadding,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    if (widget.showDragHandle) _dragHandle(inner),
                    _buildScreen(inner),
                    _supportLine(inner),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// M3's own drag-handle geometry: 32×4, `onSurfaceVariant` at the
  /// opacity the framework's `BottomSheet` uses for its built-in handle.
  Widget _dragHandle(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 16),
    child: Center(
      child: Container(
        width: 32,
        height: 4,
        decoration: BoxDecoration(
          color: Theme.of(context).colorScheme.onSurfaceVariant
              .withValues(alpha: 0.4),
          borderRadius: BorderRadius.circular(2),
        ),
      ),
    ),
  );

  /// Every screen's title.
  ///
  /// `headlineSmall` bare — no `fontWeight` override. The weight used to
  /// be hardcoded here, which is exactly the habit that makes a sheet
  /// ignore a host app's typography: M3's type scale already encodes
  /// "this is a screen title", and a host that supplies its own
  /// [TextTheme] should be able to change it.
  Widget _heading(BuildContext context, String text) => Focus(
    focusNode: _headingFocusNode,
    child: Text(text, style: Theme.of(context).textTheme.headlineSmall),
  );

  /// The test-mode marker, as an M3 tonal badge rather than a line of
  /// bold text — a payer glances at this once and should be able to tell
  /// it apart from the merchant's own copy without reading it.
  Widget _testModeBanner(BuildContext context, bool livemode) {
    if (livemode) {
      return const SizedBox.shrink();
    }
    final ThemeData theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Align(
        alignment: AlignmentDirectional.centerStart,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
          decoration: BoxDecoration(
            color: theme.colorScheme.tertiaryContainer,
            borderRadius: BorderRadius.circular(8),
          ),
          child: Text(
            _t.t('page.testmode'),
            style: theme.textTheme.labelSmall?.copyWith(
              color: theme.colorScheme.onTertiaryContainer,
            ),
          ),
        ),
      ),
    );
  }

  /// `page.support` — `i18n.dart`'s own doc comment named this a "dead
  /// translation key, because the sheet had no way to learn the contact"
  /// before issue #193. Rendered once, at the bottom of every screen
  /// (mirroring `screens.tsx`'s `SupportLine`, which the hosted page shows
  /// wherever branding shows), and only when [_supportContact] is
  /// non-null — never a blank caption reserving space for nothing.
  Widget _supportLine(BuildContext context) {
    final String? contact = _supportContact;
    if (contact == null) {
      return const SizedBox.shrink();
    }
    final ThemeData theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.only(top: 24),
      child: Text(
        _t.t('page.support', {'contact': contact}),
        textAlign: TextAlign.center,
        style: theme.textTheme.bodySmall?.copyWith(
          color: theme.colorScheme.onSurfaceVariant,
        ),
      ),
    );
  }

  /// What is being paid, and to whom — the one block on screen for the
  /// whole checkout.
  ///
  /// `Card.filled` on `surfaceContainerHigh`: a tonal step above the
  /// sheet, which is how M3 separates a container from its background
  /// without the drop shadow an elevated card would add inside an
  /// already-raised sheet.
  Widget _summary(BuildContext context, CheckoutContext checkout) {
    final CheckoutSession session = checkout.session;
    final PaymentIntent intent = checkout.intent;
    final ThemeData theme = Theme.of(context);
    final String amount = formatAmountForDisplay(
      intent.amount,
      intent.currency,
      _frenchLocale,
    );
    return Padding(
      padding: const EdgeInsets.only(bottom: 24),
      child: Card.filled(
        color: theme.colorScheme.surfaceContainerHigh,
        child: Padding(
          padding: const EdgeInsets.all(20),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              _testModeBanner(context, session.livemode),
              Text(
                _merchantLine(
                  checkout.merchantName,
                  'page.pay_to',
                  'page.pay_to_unnamed',
                ),
                style: theme.textTheme.labelLarge?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
              const SizedBox(height: 6),
              // The largest thing on the sheet, deliberately: the amount
              // is the one fact a payer must not misread.
              Text(amount, style: theme.textTheme.displaySmall),
              const SizedBox(height: 10),
              Text(
                '${_t.t('page.reference_label')}: ${session.id}',
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
            ],
          ),
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
        if (ctx != null) _summary(context, ctx),
        switch (s) {
          CheckoutLoading() => _statusPanel(
            context,
            title: _t.t('state.loading'),
            body: null,
          ),
          CheckoutLoadError(:final error) => _noticePanel(
            context,
            title: _t.t('error.title'),
            body: _t.t(errorMessageKey(error)),
            code: error.code,
          ),
          CheckoutRefused(:final reason) =>
            reason == RefusalReason.embedNotAllowed
                ? _noticePanel(
                    context,
                    title: _t.t('refusal.embed_title'),
                    body: _t.t('refusal.embed_body'),
                  )
                : _noticePanel(
                    context,
                    title: _t.t('error.title'),
                    body: _t.t('rail.none'),
                  ),
          CheckoutExpired() => _noticePanel(
            context,
            title: _t.t('expired.title'),
            body: _merchantLine(
              ctx?.merchantName,
              'expired.body',
              'expired.body_unnamed',
            ),
          ),
          CheckoutSelectRail(:final rails) => _railSelector(context, rails),
          CheckoutCollectMsisdn(
            :final rail,
            :final rails,
            :final problem,
            context: final checkout,
          ) =>
            _msisdnForm(
              context,
              rail: rail,
              problem: problem,
              canGoBack: rails.supported.length > 1,
              amount: formatAmountForDisplay(
                checkout.intent.amount,
                checkout.intent.currency,
                _frenchLocale,
              ),
            ),
          CheckoutReadyRedirect(
            :final rail,
            :final rails,
            :final problem,
            context: final checkout,
          ) =>
            _redirectPrompt(
              context,
              rail: rail,
              problem: problem,
              canGoBack: rails.supported.length > 1,
              amount: formatAmountForDisplay(
                checkout.intent.amount,
                checkout.intent.currency,
                _frenchLocale,
              ),
            ),
          CheckoutConfirming() => _statusPanel(
            context,
            title: _t.t('state.confirming'),
            body: null,
          ),
          CheckoutWaiting(:final notice, context: final checkout) =>
            _statusPanel(
              context,
              title: _t.t('state.waiting_title'),
              body: _t.t('state.waiting_body', {
                'amount': formatAmountForDisplay(
                  checkout.intent.amount,
                  checkout.intent.currency,
                  _frenchLocale,
                ),
              }),
              notice: notice,
              onRetry: notice == null ? null : _controller.retryPoll,
            ),
          CheckoutRedirecting() => _statusPanel(
            context,
            title: _t.t('state.redirecting_title'),
            body: _t.t('state.redirecting_body'),
          ),
          CheckoutOutcome() => _outcomePanel(context, s),
          CheckoutForwarding() => _statusPanel(
            context,
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

  /// Anything the payer waits through: loading, confirming, polling,
  /// redirecting, forwarding.
  ///
  /// Centred rather than left-aligned, unlike every other screen. A
  /// waiting screen has no content to read and no control to reach for —
  /// centring it stops the sheet looking like a form that failed to load.
  Widget _statusPanel(
    BuildContext context, {
    required String title,
    required String? body,
    String? notice,
    VoidCallback? onRetry,
  }) {
    final ThemeData theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        Align(
          alignment: AlignmentDirectional.centerStart,
          child: _heading(context, title),
        ),
        const SizedBox(height: 24),
        Center(
          child: SizedBox(
            width: 44,
            height: 44,
            child: CircularProgressIndicator(
              strokeWidth: 4,
              // M3 draws the unfilled part of the track too; without it
              // a thin indicator on a tonal surface reads as an artefact.
              backgroundColor: theme.colorScheme.surfaceContainerHighest,
            ),
          ),
        ),
        if (body != null) ...<Widget>[
          const SizedBox(height: 24),
          Text(
            body,
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
        if (notice != null) ...<Widget>[
          const SizedBox(height: 24),
          Semantics(
            liveRegion: true,
            child: Container(
              padding: const EdgeInsets.all(16),
              decoration: BoxDecoration(
                color: theme.colorScheme.secondaryContainer,
                borderRadius: BorderRadius.circular(
                  widget.theme.surfaceCornerRadius,
                ),
              ),
              child: Text(
                _t.t(notice),
                textAlign: TextAlign.center,
                // Was a bare `TextStyle(fontWeight: w600)`, which dropped
                // the host app's font entirely — a literal `TextStyle` with
                // no `textTheme` base inherits nothing.
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: theme.colorScheme.onSecondaryContainer,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
          ),
          if (onRetry != null) ...<Widget>[
            const SizedBox(height: 12),
            OutlinedButton(
              onPressed: onRetry,
              child: Text(_t.t('error.retry')),
            ),
          ],
        ],
      ],
    );
  }

  /// A dead end the payer cannot act on: a load error, a refusal, an
  /// expired session.
  Widget _noticePanel(
    BuildContext context, {
    required String title,
    required String body,
    String? code,
  }) {
    final ThemeData theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: <Widget>[
        _heading(context, title),
        const SizedBox(height: 16),
        Container(
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: theme.colorScheme.errorContainer,
            borderRadius: BorderRadius.circular(
              widget.theme.surfaceCornerRadius,
            ),
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: <Widget>[
              Icon(
                Icons.error_outline,
                size: 22,
                color: theme.colorScheme.onErrorContainer,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  body,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onErrorContainer,
                  ),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }

  /// How the payer picks a rail.
  ///
  /// Tappable M3 tiles rather than a column of `OutlinedButton`s: each
  /// row now carries a leading icon and a chevron, which is what tells a
  /// payer that choosing it goes somewhere rather than paying at once.
  ///
  /// The icon is chosen from [RailSpec.flow] — **not** from the rail
  /// code. `test/sheet/no_rail_code_branching_test.dart` scans this file
  /// for exactly that, and the flow is the honest signal anyway: it says
  /// whether the payer is about to leave the app, which is the one thing
  /// the two rails genuinely differ on from here.
  Widget _railSelector(BuildContext context, RailChoices rails) {
    final ThemeData theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _heading(context, _t.t('rail.legend')),
        const SizedBox(height: 16),
        for (final SupportedRail rail in rails.supported) ...<Widget>[
          Card.outlined(
            margin: EdgeInsets.zero,
            // Without this the `InkWell` below splashes past the rounded
            // corners — `Card`'s own default is `Clip.none`, and the ink
            // is painted by the Material underneath, not by the shape.
            clipBehavior: Clip.antiAlias,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(
                widget.theme.surfaceCornerRadius,
              ),
              side: BorderSide(color: theme.colorScheme.outlineVariant),
            ),
            child: InkWell(
              onTap: () => _controller.chooseRail(rail),
              borderRadius: BorderRadius.circular(
                widget.theme.surfaceCornerRadius,
              ),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 16,
                  vertical: 14,
                ),
                child: Row(
                  children: <Widget>[
                    Icon(
                      rail.spec.flow == RailFlow.redirect
                          ? Icons.open_in_new
                          : Icons.smartphone,
                      color: theme.colorScheme.primary,
                    ),
                    const SizedBox(width: 16),
                    Expanded(
                      child: Text(
                        _labelFor(rail),
                        style: theme.textTheme.titleMedium,
                      ),
                    ),
                    Icon(
                      Icons.chevron_right,
                      color: theme.colorScheme.onSurfaceVariant,
                    ),
                  ],
                ),
              ),
            ),
          ),
          const SizedBox(height: 12),
        ],
        if (rails.unsupported.isNotEmpty) ...<Widget>[
          const SizedBox(height: 4),
          for (final UnsupportedRail u in rails.unsupported)
            Padding(
              padding: const EdgeInsets.only(top: 4),
              child: Text(
                _t.t('rail.unsupported', {'rail': u.code}),
                style: theme.textTheme.bodySmall?.copyWith(
                  color: theme.colorScheme.onSurfaceVariant,
                ),
              ),
            ),
        ],
      ],
    );
  }

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

  Widget _rememberControl(BuildContext context, {required String label}) {
    final ThemeData theme = Theme.of(context);
    return Column(
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
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(
              widget.theme.surfaceCornerRadius,
            ),
          ),
          title: Text(label, style: theme.textTheme.bodyMedium),
          subtitle: Text(
            _t.t('memory.warning'),
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
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
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ),
      ],
    );
  }

  Widget _msisdnForm(
    BuildContext context, {
    required SupportedRail rail,
    required String? problem,
    required bool canGoBack,
    required String amount,
  }) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: <Widget>[
      _heading(context, _labelFor(rail)),
      const SizedBox(height: 16),
      // Still a `TextField`. Its Material 3 look now comes from the
      // `InputDecorationTheme` `VpayCheckoutTheme.resolve` installs
      // (filled, real shape) rather than from whatever the host app
      // happened to set — which, for a host with no theme at all, was
      // M2's underline.
      TextField(
        controller: _msisdnController,
        keyboardType: TextInputType.phone,
        autofillHints: const <String>[AutofillHints.telephoneNumber],
        onChanged: (_) => _msisdnManuallyEdited = true,
        decoration: InputDecoration(
          labelText: _t.t('msisdn.label'),
          helperText: _t.t('msisdn.hint'),
          errorText: problem == null ? null : _t.t(problem),
          prefixIcon: const Icon(Icons.phone_outlined),
        ),
      ),
      const SizedBox(height: 16),
      _rememberControl(context, label: _t.t('memory.remember_number')),
      const SizedBox(height: 20),
      // `FilledButton`, not `ElevatedButton`: M3's highest-emphasis
      // button is the filled one, and this is the only action on screen
      // that moves money.
      FilledButton(
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

  Widget _redirectPrompt(
    BuildContext context, {
    required SupportedRail rail,
    required String? problem,
    required bool canGoBack,
    required String amount,
  }) {
    final ThemeData theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _heading(context, _labelFor(rail)),
        const SizedBox(height: 12),
        Text(
          _t.t('state.redirecting_body'),
          style: theme.textTheme.bodyMedium?.copyWith(
            color: theme.colorScheme.onSurfaceVariant,
          ),
        ),
        if (problem != null) ...<Widget>[
          const SizedBox(height: 12),
          Text(
            _t.t(problem),
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.error,
            ),
          ),
        ],
        const SizedBox(height: 16),
        _rememberControl(
          context,
          label: _t.t('memory.remember_method', {'rail': _labelFor(rail)}),
        ),
        const SizedBox(height: 20),
        FilledButton(
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
  }

  Widget _outcomePanel(BuildContext context, CheckoutOutcome outcome) {
    final ThemeData theme = Theme.of(context);
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
    final (Color container, Color onContainer, IconData icon) = _outcomeRole(
      theme.colorScheme,
      outcome.kind,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        _heading(context, title),
        const SizedBox(height: 16),
        Semantics(
          liveRegion: true,
          child: Container(
            padding: const EdgeInsets.all(20),
            decoration: BoxDecoration(
              color: container,
              borderRadius: BorderRadius.circular(
                widget.theme.surfaceCornerRadius,
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: <Widget>[
                Icon(icon, size: 32, color: onContainer),
                const SizedBox(height: 12),
                Text(
                  body,
                  style: theme.textTheme.bodyLarge?.copyWith(
                    color: onContainer,
                  ),
                ),
              ],
            ),
          ),
        ),
        // `providerReason` is shown BESIDE, never instead of, the
        // translated failure line above — issue #189's own bar item.
        if (reason != null) ...<Widget>[
          const SizedBox(height: 12),
          Text(
            '${_t.t('outcome.provider_said')}: $reason',
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
        ],
        const SizedBox(height: 24),
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
        FilledButton(
          onPressed: _controller.returnToMerchant,
          // Not `_merchantLine`: the button no longer names the merchant,
          // because the payer is already in the merchant's app.
          child: Text(_t.t('outcome.done')),
        ),
      ],
    );
  }

  /// The container/label/icon triple for an outcome.
  ///
  /// `primaryContainer` for success rather than a hardcoded green: the
  /// sheet has no palette of its own, and on a seeded scheme the
  /// merchant's own brand colour is the right "this worked" surface.
  (Color, Color, IconData) _outcomeRole(ColorScheme scheme, OutcomeKind kind) =>
      switch (kind) {
        OutcomeKind.succeeded => (
          scheme.primaryContainer,
          scheme.onPrimaryContainer,
          Icons.check_circle_outline,
        ),
        OutcomeKind.canceled => (
          scheme.surfaceContainerHighest,
          scheme.onSurfaceVariant,
          Icons.cancel_outlined,
        ),
        OutcomeKind.failed => (
          scheme.errorContainer,
          scheme.onErrorContainer,
          Icons.error_outline,
        ),
      };
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
