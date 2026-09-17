/// A payer-facing checkout surface for vpay (`docs/adr/0021-flutter-checkout-plugin.md`).
///
/// **Before opening a vpay checkout for any digital-goods purchase, read
/// this package's README's "App Store and Play policy" section.** It is
/// not a way around Apple's or Google's in-app purchase rules — see D9 in
/// `docs/plans/2026-09-13-flutter-plugin.md`. That section matters more
/// since D5's 2026-09-16 revision, not less: every checkout now opens the
/// payer's browser, which is exactly the shape those rules scrutinise.
library;

export 'src/browser_client.dart'
    show BrowserClient, CheckoutSessionResult, PaymentIntentResult;
export 'src/config/checkout_page_config.dart'
    show
        CheckoutPageBranding,
        CheckoutPageCheckoutSettings,
        CheckoutPageConfig,
        InMemoryVpayCheckoutConfigStore,
        VpayCheckoutConfigStore,
        checkoutPageConfigTtl,
        defaultCheckoutPageConfigStore,
        narrowAllowedMethods,
        prepareCheckout,
        resolveCheckoutPageConfig;
export 'src/checkout_controller.dart'
    show
        CheckoutController,
        CheckoutPreflight,
        CheckoutPreflightFailure,
        CheckoutPreflightReady,
        CheckoutPreflightSuccess,
        StopUrlSpec,
        SystemClock,
        VpayClock;
export 'src/errors.dart' show VpayClientErrorCodes, VpayError;
export 'src/models.dart'
    show
        CheckoutSession,
        CheckoutSessionPaymentStatus,
        CheckoutSessionStatus,
        CheckoutUiMode,
        FailureCode,
        LastPaymentError,
        PaymentIntent,
        PaymentIntentStatus,
        RailDisplayName,
        RailField,
        RailFieldKind,
        RailFieldKindPhone,
        RailFieldKindUnknown,
        RailFieldPhoneType,
        RailFlow,
        RailSpec;
export 'src/platform/checkout_platform.dart'
    show UnimplementedVpayCheckoutPlatform, VpayCheckoutPlatform;
export 'src/platform/messages.g.dart'
    show CheckoutStopUrl, CheckoutWindowEvent, CheckoutWindowOutcome;
export 'src/result.dart'
    show
        VpayCheckoutCanceled,
        VpayCheckoutFailed,
        VpayCheckoutPending,
        VpayCheckoutResult,
        VpayCheckoutSucceeded,
        VpayCheckoutUnresolved;
export 'src/sheet/checkout_screen.dart'
    show
        CheckoutBack,
        CheckoutChooseRail,
        CheckoutCollectMsisdn,
        CheckoutConfirmStarted,
        CheckoutConfirming,
        CheckoutContext,
        CheckoutEvent,
        CheckoutExpired,
        CheckoutForward,
        CheckoutForwarding,
        CheckoutIntentUpdated,
        CheckoutLoadError,
        CheckoutLoadFailed,
        CheckoutLoaded,
        CheckoutLoading,
        CheckoutOutcome,
        CheckoutProblem,
        CheckoutReadyRedirect,
        CheckoutRedirectRequired,
        CheckoutRedirecting,
        CheckoutRefuse,
        CheckoutRefused,
        CheckoutScreenState,
        CheckoutSelectRail,
        CheckoutSessionRefreshed,
        CheckoutWaiting,
        Outcome,
        OutcomeKind,
        RefusalReason,
        checkoutInitialState,
        contextOfCheckoutScreen,
        intentOutcome,
        reduceCheckoutScreen,
        stateForContext;
export 'src/sheet/checkout_sheet.dart'
    show
        VpayCheckoutSheet,
        kVpayCheckoutSheetCornerRadius,
        showVpayCheckoutSheet,
        showVpayCheckoutSheetRoute;
export 'src/sheet/failures.dart' show maxProviderReasonLength, providerReason;
export 'src/sheet/i18n.dart'
    show
        VpayCheckoutStrings,
        VpayLocale,
        failureMessageKey,
        railLabelFor,
        vpayCheckoutMessageKeys,
        vpayCheckoutMessageKeysFr;
export 'src/sheet/money.dart'
    show currencyExponent, minorUnitsToDecimalString, toDecimalString;
export 'src/sheet/money_format.dart' show formatAmountForDisplay;
export 'src/sheet/msisdn.dart'
    show formatCameroonMsisdn, normalizeCameroonMsisdn;
export 'src/sheet/poll_jitter.dart'
    show
        FixedJitterSource,
        JitterSource,
        SystemJitterSource,
        defaultPollBudget,
        defaultPollInterval,
        jitteredDelay,
        nextPollDelay,
        pollJitterSpread;
export 'src/sheet/rails.dart'
    show
        RailChoices,
        SupportedRail,
        UnsupportedRail,
        UnsupportedRailReason,
        railChoices;
export 'src/sheet/remember_msisdn.dart'
    show
        RememberedMsisdnRecord,
        SharedPreferencesRememberedMsisdnStore,
        VpayRememberedMsisdn,
        VpayRememberedMsisdnStore,
        rememberedMsisdnTtl;
export 'src/sheet/return_screen.dart'
    show
        ReturnContext,
        ReturnEvent,
        ReturnExpired,
        ReturnForward,
        ReturnForwarding,
        ReturnLoadError,
        ReturnLoading,
        ReturnOutcome,
        ReturnPaymentIntent,
        ReturnPollFailed,
        ReturnPolling,
        ReturnRead,
        ReturnReadFailed,
        ReturnScreenState,
        reduceReturn,
        returnInitialState,
        stateForReturn;
export 'src/sheet/sheet_controller.dart' show SheetController, errorMessageKey;
export 'src/vpay_checkout.dart' show VpayCheckout;
