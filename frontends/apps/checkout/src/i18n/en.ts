/**
 * The English dictionary — and, because `fr.ts` is typed against it, the
 * definition of what a locale must cover.
 *
 * Every value is a plain string. Interpolation is `{name}` placeholders read
 * by {@link import('./format.js').format}, never a function, for two
 * reasons: `src/i18n/dictionary.test.ts` can then iterate the keys and
 * assert that both locales carry the *same* placeholders (a French string
 * that dropped `{amount}` would otherwise render a sentence with a hole in
 * it and no test would notice), and a translator never has to read
 * TypeScript.
 */
export const en = {
  'page.title': 'Checkout',
  'page.pay_to': 'Pay {merchant}',
  /*
   * The `_unnamed` twins below are what every merchant-name string reads
   * when the browser read carried no usable name (`merchantOf` said `null`).
   * They are written as their own sentences rather than produced by
   * substituting a stand-in for `{merchant}`: "Pay —", "Pay the merchant
   * Boutique?" or an id in a name's place would each read as data the page
   * actually has.
   */
  'page.pay_to_unnamed': 'Payment',
  'page.amount_label': 'Amount',
  'page.reference_label': 'Reference',
  'page.testmode': 'Test mode — no money moves on this deployment.',

  'locale.label': 'Language',
  'locale.en': 'English',
  'locale.fr': 'Français',

  'rail.legend': 'Choose how you want to pay',
  'rail.mtn_momo': 'MTN Mobile Money',
  'rail.orange_money': 'Orange Money',
  'rail.continue': 'Continue',
  'rail.unsupported': 'This page cannot take a payment on {rail}.',
  'rail.none': 'This payment offers no payment method this page can show.',

  'msisdn.label': 'MTN MoMo number',
  'msisdn.hint': 'Your Cameroon MTN number, for example +237 6 71 23 45 67.',
  'msisdn.invalid': 'Enter a Cameroon mobile number: 6 followed by 8 digits, with or without +237.',
  'msisdn.submit': 'Pay {amount}',
  'msisdn.back': 'Choose another payment method',

  'state.loading': 'Loading this payment…',
  'state.confirming': 'Sending your payment request…',
  'state.waiting_title': 'Check your phone',
  'state.waiting_body': 'Approve {amount} on your handset. This page updates on its own.',
  'state.redirecting_title': 'Taking you to Orange Money',
  'state.redirecting_body': 'You will come back here once you have paid.',

  'outcome.succeeded_title': 'Payment received',
  'outcome.succeeded_body': '{merchant} has been told you paid {amount}.',
  'outcome.succeeded_body_unnamed': 'The merchant has been told you paid {amount}.',
  'outcome.failed_title': 'Payment not completed',
  'outcome.canceled_title': 'Payment canceled',
  'outcome.canceled_body': 'This payment was canceled. Nothing was taken.',
  'outcome.continue': 'Continue',
  'outcome.auto_forward': 'Returning to {merchant} in {seconds} s.',
  'outcome.auto_forward_unnamed': 'Returning in {seconds} s.',
  'outcome.no_destination': 'This payment is finished. You can close this page.',

  'failure.insufficient_funds': 'There was not enough money in the account.',
  'failure.payer_timeout': 'You did not approve the payment in time.',
  'failure.payer_declined': 'You declined the payment.',
  'failure.invalid_payer': 'That account cannot be charged.',
  'failure.payer_limit_reached': 'The account has reached its transaction limit.',
  'failure.payer_account_blocked': 'The account is blocked.',
  'failure.invalid_payee': 'The merchant account cannot receive this payment.',
  'failure.payee_account_blocked': 'The merchant account is blocked.',
  'failure.provider_account_blocked': 'The payment provider refused this merchant.',
  'failure.provider_unavailable': 'The payment provider could not be reached.',
  'failure.provider_error': 'The payment provider refused the payment and gave no reason.',
  'failure.unknown': 'The payment did not go through.',

  'expired.title': 'This payment page has expired',
  'expired.body': 'Go back to {merchant} and start again.',
  'expired.body_unnamed': 'Go back to the shop you came from and start again.',

  'error.title': 'This page cannot continue',
  'error.session_not_found': 'This payment link is not valid, or it has already been used.',
  'error.network': 'vpay could not be reached. Check your connection and try again.',
  'error.unexpected': 'Something went wrong on our side.',
  'error.missing_key': 'This link is missing the publishable key vpay needs to identify the merchant.',
  'error.missing_secret': 'This link is missing the credential that unlocks the payment.',
  'error.missing_return_token': 'This return link is missing its token.',
  'error.retry': 'Try again',

  'refusal.embed_title': 'This page will not load here',
  'refusal.embed_body':
    'vpay only shows an embedded payment page on a site the merchant has registered. Ask the merchant to add this site to its checkout origins.',
} as const;

/** Every key any locale must carry. */
export type MessageKey = keyof typeof en;
