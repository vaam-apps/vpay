/// The sheet's own i18n catalogue — the Dart port of
/// `frontends/apps/checkout/src/i18n/{en,fr}.ts`.
///
/// **French is the default** (issue #189: "French is Cameroon's and
/// Orange's language" — defaulting to English would be a regression), so
/// [VpayLocale.fr] is what [VpayCheckoutStrings.of] falls back to when a
/// caller does not name one and [defaultLocale] is what a merchant's app
/// gets if it never asks.
///
/// Both dictionaries carry the same 72 keys as `en.ts`/`fr.ts` — one key per
/// entry in that file, counted the same way (`grep -c` against the wire
/// source) — asserted by `test/sheet/i18n_test.dart`'s completeness check
/// rather than merely hoped for: a key present in one locale and missing
/// from the other is a compile-time-shaped bug this port cannot make the
/// compiler catch (both dictionaries are plain `Map<String, String>`, not a
/// shared `enum`), so a test stands in for the type check `fr.ts`'s
/// `Record<MessageKey, string>` gets for free.
///
/// Interpolation is `{name}` placeholders, substituted by [VpayCheckoutStrings.t]
/// — never a function per string, so a translator only ever edits a plain
/// string and a test can enumerate every key.
library;

/// The two locales this sheet ships. Not an open set: a merchant embedding
/// this SDK cannot supply a third dictionary today, the same constraint
/// `en.ts`/`fr.ts` impose on the hosted page.
enum VpayLocale {
  fr,
  en;

  /// French — see this file's own doc comment for why.
  static const VpayLocale fallback = VpayLocale.fr;
}

/// `en.ts`, restated. Every value a plain string with `{name}` placeholders.
const Map<String, String> _en = <String, String>{
  'page.title': 'Checkout',
  'page.pay_to': 'Pay {merchant}',
  'page.pay_to_unnamed': 'Payment',
  'page.amount_label': 'Amount',
  'page.reference_label': 'Reference',
  'page.testmode': 'Test mode — no money moves on this deployment.',
  'page.operator_logo_alt': 'Logo',
  'page.support': 'Support: {contact}',

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
  'msisdn.invalid':
      'Enter a Cameroon mobile number: 6 followed by 8 digits, with or '
      'without +237.',
  'msisdn.submit': 'Pay {amount}',
  'msisdn.back': 'Choose another payment method',

  'state.loading': 'Loading this payment…',
  'state.confirming': 'Sending your payment request…',
  'state.waiting_title': 'Check your phone',
  'state.waiting_body':
      'Approve {amount} on your handset. This page updates on its own.',
  'state.redirecting_title': 'Taking you to Orange Money',
  'state.redirecting_body': 'You will come back here once you have paid.',

  // `requires_action`: the payer has a redirect to finish on the rail's own
  // page and is not on it. Deliberately NOT `state.waiting_*` — that pair
  // claims the payment is moving on its own, which is false here and
  // unresolvable without the payer's own next step. Same three keys, same
  // strings, as `frontends/apps/checkout/src/i18n/en.ts`.
  'state.resume_redirect_title': 'Payment not completed',
  'state.resume_redirect_body':
      "You did not finish this payment on the provider's page. Nothing has "
      'been taken.',
  'state.resume_redirect_continue': 'Return to the payment page',

  'outcome.succeeded_title': 'Payment received',
  'outcome.succeeded_body': '{merchant} has been told you paid {amount}.',
  'outcome.succeeded_body_unnamed':
      'The merchant has been told you paid {amount}.',
  'outcome.failed_title': 'Payment not completed',
  'outcome.canceled_title': 'Payment canceled',
  'outcome.canceled_body': 'This payment was canceled. Nothing was taken.',
  // "Done", not "Back to {merchant}": the payer never left. This is a
  // native sheet over the merchant's own app, so "back to the shop" names a
  // journey that did not happen — it is the hosted *web* page's wording,
  // where the payer really was on another origin and really did have to
  // travel back. Dismissing the sheet reveals the app that was behind it
  // the whole time. `{merchant}` is therefore unused here on purpose.
  'outcome.done': 'Done',
  'outcome.no_destination':
      'This payment is finished. You can close this page.',
  'outcome.provider_said': 'What the payment provider said',

  'state.forwarding_title': 'Taking you back',
  'state.forwarding_body': 'Returning you to {merchant}.',
  'state.forwarding_body_unnamed': 'Returning you to the shop.',

  'failure.insufficient_funds': 'There was not enough money in the account.',
  'failure.payer_timeout': 'You did not approve the payment in time.',
  'failure.payer_declined': 'You declined the payment.',
  'failure.invalid_payer': 'That account cannot be charged.',
  'failure.payer_limit_reached':
      'The account has reached its transaction limit.',
  'failure.payer_account_blocked': 'The account is blocked.',
  'failure.invalid_payee': 'The merchant account cannot receive this payment.',
  'failure.payee_account_blocked': 'The merchant account is blocked.',
  'failure.provider_account_blocked':
      'The payment provider refused this merchant.',
  'failure.provider_unavailable': 'The payment provider could not be reached.',
  'failure.provider_error':
      'The payment provider refused the payment and gave no reason.',
  'failure.unknown': 'The payment did not go through.',

  'expired.title': 'This payment page has expired',
  'expired.body': 'Go back to {merchant} and start again.',
  'expired.body_unnamed': 'Go back to the shop you came from and start again.',

  'error.title': 'This page cannot continue',
  'error.session_not_found':
      'This payment link is not valid, or it has already been used.',
  'error.network':
      'vpay could not be reached. Check your connection and try again.',
  'error.unexpected': 'Something went wrong on our side.',
  'error.missing_key':
      'This link is missing the publishable key vpay needs to identify '
      'the merchant.',
  'error.missing_secret':
      'This link is missing the credential that unlocks the payment.',
  'error.missing_return_token': 'This return link is missing its token.',
  'error.retry': 'Try again',

  'memory.remember_number': 'Remember this number on this device',
  'memory.remember_method': 'Remember {rail} on this device',
  'memory.warning':
      'Anyone else who uses this device will see it. Do not tick this on '
      'a shared or borrowed phone.',
  'memory.forget': 'Forget what this device remembers',
  'memory.forgotten': 'This device remembers nothing now.',
  'memory.last_used': 'Last used',

  'refusal.embed_title': 'This page will not load here',
  'refusal.embed_body':
      'vpay only shows an embedded payment page on a site the merchant has '
      'registered. Ask the merchant to add this site to its checkout '
      'origins.',
};

/// `fr.ts`, restated — same 72 keys as [_en].
const Map<String, String> _fr = <String, String>{
  'page.title': 'Paiement',
  'page.pay_to': 'Payer {merchant}',
  'page.pay_to_unnamed': 'Paiement à régler',
  'page.amount_label': 'Montant',
  'page.reference_label': 'Référence',
  'page.testmode': 'Mode test — aucun argent ne circule sur ce déploiement.',
  'page.operator_logo_alt': 'Logo',
  'page.support': 'Assistance : {contact}',

  'locale.label': 'Langue',
  'locale.en': 'English',
  'locale.fr': 'Français',

  'rail.legend': 'Choisissez votre moyen de paiement',
  'rail.mtn_momo': 'MTN Mobile Money',
  'rail.orange_money': 'Orange Money',
  'rail.continue': 'Continuer',
  'rail.unsupported':
      'Cette page ne peut pas encaisser un paiement via {rail}.',
  'rail.none':
      'Ce paiement ne propose aucun moyen de paiement que cette page sait '
      'afficher.',

  'msisdn.label': 'Numéro MTN MoMo',
  'msisdn.hint':
      'Votre numéro MTN camerounais, par exemple +237 6 71 23 45 67.',
  'msisdn.invalid':
      'Saisissez un numéro mobile camerounais : 6 suivi de 8 chiffres, '
      'avec ou sans +237.',
  'msisdn.submit': 'Payer {amount}',
  'msisdn.back': 'Choisir un autre moyen de paiement',

  'state.loading': 'Chargement du paiement…',
  'state.confirming': 'Envoi de votre demande de paiement…',
  'state.waiting_title': 'Consultez votre téléphone',
  'state.waiting_body':
      'Validez {amount} sur votre combiné. Cette page se met à jour toute '
      'seule.',
  'state.redirecting_title': 'Redirection vers Orange Money',
  'state.redirecting_body':
      'Vous reviendrez ici une fois le paiement effectué.',

  'state.resume_redirect_title': 'Paiement non abouti',
  'state.resume_redirect_body':
      'Vous n\u2019avez pas terminé ce paiement sur la page de '
      'l\u2019opérateur. Rien n\u2019a été prélevé.',
  'state.resume_redirect_continue': 'Retourner à la page de paiement',

  'outcome.succeeded_title': 'Paiement reçu',
  'outcome.succeeded_body':
      '{merchant} a été informé que vous avez payé {amount}.',
  'outcome.succeeded_body_unnamed':
      'Le marchand a été informé que vous avez payé {amount}.',
  'outcome.failed_title': 'Paiement non abouti',
  'outcome.canceled_title': 'Paiement annulé',
  'outcome.canceled_body': 'Ce paiement a été annulé. Rien n’a été prélevé.',
  // See the English entry: the payer never left the app.
  'outcome.done': 'Conclure',
  'outcome.no_destination':
      'Ce paiement est terminé. Vous pouvez fermer cette page.',
  'outcome.provider_said': 'Ce qu’a répondu l’opérateur',

  'state.forwarding_title': 'Retour en cours',
  'state.forwarding_body': 'Nous vous ramenons vers {merchant}.',
  'state.forwarding_body_unnamed': 'Nous vous ramenons vers la boutique.',

  'failure.insufficient_funds': 'Le solde du compte était insuffisant.',
  'failure.payer_timeout': 'Vous n’avez pas validé le paiement à temps.',
  'failure.payer_declined': 'Vous avez refusé le paiement.',
  'failure.invalid_payer': 'Ce compte ne peut pas être débité.',
  'failure.payer_limit_reached':
      'Le compte a atteint sa limite de transactions.',
  'failure.payer_account_blocked': 'Le compte est bloqué.',
  'failure.invalid_payee':
      'Le compte du marchand ne peut pas recevoir ce paiement.',
  'failure.payee_account_blocked': 'Le compte du marchand est bloqué.',
  'failure.provider_account_blocked': 'L’opérateur a refusé ce marchand.',
  'failure.provider_unavailable': 'L’opérateur est injoignable.',
  'failure.provider_error':
      'L’opérateur a refusé le paiement sans en donner la raison.',
  'failure.unknown': 'Le paiement n’a pas abouti.',

  'expired.title': 'Cette page de paiement a expiré',
  'expired.body': 'Retournez sur {merchant} et recommencez.',
  'expired.body_unnamed':
      'Retournez sur la boutique d’où vous venez et recommencez.',

  'error.title': 'Cette page ne peut pas continuer',
  'error.session_not_found':
      'Ce lien de paiement n’est pas valide, ou il a déjà été utilisé.',
  'error.network':
      'vpay est injoignable. Vérifiez votre connexion et réessayez.',
  'error.unexpected': 'Un incident est survenu de notre côté.',
  'error.missing_key':
      'Il manque à ce lien la clé publiable dont vpay a besoin pour '
      'identifier le marchand.',
  'error.missing_secret':
      'Il manque à ce lien l’identifiant qui déverrouille le paiement.',
  'error.missing_return_token': 'Il manque son jeton à ce lien de retour.',
  'error.retry': 'Réessayer',

  'memory.remember_number': 'Mémoriser ce numéro sur cet appareil',
  'memory.remember_method': 'Mémoriser {rail} sur cet appareil',
  'memory.warning':
      'Toute autre personne qui utilise cet appareil le verra. Ne cochez '
      'pas cette case sur un téléphone partagé ou emprunté.',
  'memory.forget': 'Oublier ce que cet appareil a mémorisé',
  'memory.forgotten': 'Cet appareil ne mémorise plus rien.',
  'memory.last_used': 'Dernier utilisé',

  'refusal.embed_title': 'Cette page ne s’affichera pas ici',
  'refusal.embed_body':
      'vpay n’affiche une page de paiement intégrée que sur un site '
      'enregistré par le marchand. Demandez au marchand d’ajouter ce site '
      'à ses origines de paiement.',
};

const Map<VpayLocale, Map<String, String>> _catalogue =
    <VpayLocale, Map<String, String>>{VpayLocale.en: _en, VpayLocale.fr: _fr};

/// The FAILURE_MESSAGES table — `failures.ts`'s
/// `Record<FailureCode, MessageKey>`, restated for Dart's open [String]
/// `FailureCode` (`models.dart`'s own note on why it is not a closed enum
/// here). A code neither dictionary names falls back to `failure.unknown`
/// — never the raw code rendered as if it were a sentence.
const Map<String, String> _failureMessageKeys = <String, String>{
  'insufficient_funds': 'failure.insufficient_funds',
  'payer_timeout': 'failure.payer_timeout',
  'payer_declined': 'failure.payer_declined',
  'invalid_payer': 'failure.invalid_payer',
  'payer_limit_reached': 'failure.payer_limit_reached',
  'payer_account_blocked': 'failure.payer_account_blocked',
  'invalid_payee': 'failure.invalid_payee',
  'payee_account_blocked': 'failure.payee_account_blocked',
  'provider_account_blocked': 'failure.provider_account_blocked',
  'provider_unavailable': 'failure.provider_unavailable',
  'provider_error': 'failure.provider_error',
};

/// [code]'s i18n key, or `null` when [code] itself is `null` — `failures.ts`'s
/// `failureMessage`. A [code] neither dictionary names answers
/// `'failure.unknown'`, never `null` and never the raw code.
String? failureMessageKey(String? code) {
  if (code == null) {
    return null;
  }
  return _failureMessageKeys[code] ?? 'failure.unknown';
}

/// One locale's resolved strings, with `{name}` interpolation —
/// `frontends/apps/checkout/src/i18n/index.ts`'s `Translate`, as a Dart
/// object rather than a closure so a widget can hold one in its build
/// method without capturing a locale in a closure's scope.
final class VpayCheckoutStrings {
  const VpayCheckoutStrings(this.locale);

  final VpayLocale locale;

  Map<String, String> get _dictionary => _catalogue[locale]!;

  /// Looks up [key] and substitutes every `{name}` in [values]. A [key]
  /// this catalogue does not carry renders as the bracketed key itself —
  /// never a thrown error on a payment screen — which is also what makes a
  /// missing-key mistake visible in a screenshot rather than a crash log.
  String t(String key, [Map<String, String> values = const {}]) {
    String template = _dictionary[key] ?? '{$key}';
    for (final MapEntry<String, String> entry in values.entries) {
      template = template.replaceAll('{${entry.key}}', entry.value);
    }
    return template;
  }
}

/// Every key both dictionaries must carry — for
/// `test/sheet/i18n_test.dart`'s completeness check, and the one place a
/// caller can ask "does this catalogue know this key" without reaching into
/// a private map.
Set<String> get vpayCheckoutMessageKeys => _en.keys.toSet();

/// [_fr]'s own keys, for the same completeness check from the other side.
Set<String> get vpayCheckoutMessageKeysFr => _fr.keys.toSet();

/// A rail's display label, resolved the way `models.dart`'s [RailSpec.labelKey]
/// doc comment specifies: the catalogue first, then the deployment's own
/// configured [RailDisplayName] for [locale], then the raw [code] — never a
/// blank control, and never a rail code branched on to pick between them.
String railLabelFor({
  required VpayLocale locale,
  required String labelKey,
  required String code,
  String? configuredEn,
  String? configuredFr,
}) {
  final String? catalogued = _catalogue[locale]![labelKey];
  if (catalogued != null) {
    return catalogued;
  }
  final String? configured = locale == VpayLocale.fr
      ? configuredFr
      : configuredEn;
  if (configured != null && configured.isNotEmpty) {
    return configured;
  }
  return code;
}
