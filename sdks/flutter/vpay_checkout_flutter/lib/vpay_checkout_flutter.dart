/// A payer-facing checkout surface for vpay (`docs/adr/0021-flutter-checkout-plugin.md`).
///
/// **Before using [VpayCheckoutMode.externalBrowser] on iOS or opening a
/// vpay checkout for any digital-goods purchase, read this package's
/// README's "App Store and Play policy" section.** It is not a way around
/// Apple's or Google's in-app purchase rules — see D9 in
/// `docs/plans/2026-09-13-flutter-plugin.md`.
library;

export 'src/browser_client.dart'
    show BrowserClient, CheckoutSessionResult, PaymentIntentResult;
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
        CheckoutSessionStatus,
        CheckoutUiMode,
        FailureCode,
        LastPaymentError,
        PaymentIntent,
        PaymentIntentStatus;
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
export 'src/vpay_checkout.dart' show VpayCheckout, VpayCheckoutMode;
