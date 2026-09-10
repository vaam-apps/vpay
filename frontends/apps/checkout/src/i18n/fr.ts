/**
 * The French dictionary. Cameroon-first: French is the default this page
 * falls back to when `Accept-Language` expresses no preference, because the
 * deployment this repository is written for serves Cameroon and Orange's own
 * hosted page is French by default.
 *
 * Typed as `Record<MessageKey, string>` so a missing key is a compile error
 * rather than a blank line on a payment page.
 */
import type { MessageKey } from "./en";

export const fr: Record<MessageKey, string> = {
  "page.title": "Paiement",
  "page.pay_to": "Payer {merchant}",
  "page.pay_to_unnamed": "Paiement à régler",
  "page.amount_label": "Montant",
  "page.reference_label": "Référence",
  "page.testmode": "Mode test — aucun argent ne circule sur ce déploiement.",
  "page.operator_logo_alt": "Logo",
  "page.support": "Assistance : {contact}",

  "locale.label": "Langue",
  "locale.en": "English",
  "locale.fr": "Français",

  "rail.legend": "Choisissez votre moyen de paiement",
  "rail.mtn_momo": "MTN Mobile Money",
  "rail.orange_money": "Orange Money",
  "rail.continue": "Continuer",
  "rail.unsupported":
    "Cette page ne peut pas encaisser un paiement via {rail}.",
  "rail.none":
    "Ce paiement ne propose aucun moyen de paiement que cette page sait afficher.",

  "msisdn.label": "Numéro MTN MoMo",
  "msisdn.hint":
    "Votre numéro MTN camerounais, par exemple +237 6 71 23 45 67.",
  "msisdn.invalid":
    "Saisissez un numéro mobile camerounais : 6 suivi de 8 chiffres, avec ou sans +237.",
  "msisdn.submit": "Payer {amount}",
  "msisdn.back": "Choisir un autre moyen de paiement",

  "state.loading": "Chargement du paiement…",
  "state.confirming": "Envoi de votre demande de paiement…",
  "state.waiting_title": "Consultez votre téléphone",
  "state.waiting_body":
    "Validez {amount} sur votre combiné. Cette page se met à jour toute seule.",
  "state.redirecting_title": "Redirection vers Orange Money",
  "state.redirecting_body":
    "Vous reviendrez ici une fois le paiement effectué.",

  "outcome.succeeded_title": "Paiement reçu",
  "outcome.succeeded_body":
    "{merchant} a été informé que vous avez payé {amount}.",
  "outcome.succeeded_body_unnamed":
    "Le marchand a été informé que vous avez payé {amount}.",
  "outcome.failed_title": "Paiement non abouti",
  "outcome.canceled_title": "Paiement annulé",
  "outcome.canceled_body": "Ce paiement a été annulé. Rien n’a été prélevé.",
  "outcome.back_to": "Retour vers {merchant}",
  "outcome.back_to_unnamed": "Retour à la boutique",
  "outcome.no_destination":
    "Ce paiement est terminé. Vous pouvez fermer cette page.",
  "outcome.provider_said": "Ce qu’a répondu l’opérateur",

  "state.forwarding_title": "Retour en cours",
  "state.forwarding_body": "Nous vous ramenons vers {merchant}.",
  "state.forwarding_body_unnamed": "Nous vous ramenons vers la boutique.",

  "failure.insufficient_funds": "Le solde du compte était insuffisant.",
  "failure.payer_timeout": "Vous n’avez pas validé le paiement à temps.",
  "failure.payer_declined": "Vous avez refusé le paiement.",
  "failure.invalid_payer": "Ce compte ne peut pas être débité.",
  "failure.payer_limit_reached":
    "Le compte a atteint sa limite de transactions.",
  "failure.payer_account_blocked": "Le compte est bloqué.",
  "failure.invalid_payee":
    "Le compte du marchand ne peut pas recevoir ce paiement.",
  "failure.payee_account_blocked": "Le compte du marchand est bloqué.",
  "failure.provider_account_blocked": "L’opérateur a refusé ce marchand.",
  "failure.provider_unavailable": "L’opérateur est injoignable.",
  "failure.provider_error":
    "L’opérateur a refusé le paiement sans en donner la raison.",
  "failure.unknown": "Le paiement n’a pas abouti.",

  "expired.title": "Cette page de paiement a expiré",
  "expired.body": "Retournez sur {merchant} et recommencez.",
  "expired.body_unnamed":
    "Retournez sur la boutique d’où vous venez et recommencez.",

  "error.title": "Cette page ne peut pas continuer",
  "error.session_not_found":
    "Ce lien de paiement n’est pas valide, ou il a déjà été utilisé.",
  "error.network":
    "vpay est injoignable. Vérifiez votre connexion et réessayez.",
  "error.unexpected": "Un incident est survenu de notre côté.",
  "error.missing_key":
    "Il manque à ce lien la clé publiable dont vpay a besoin pour identifier le marchand.",
  "error.missing_secret":
    "Il manque à ce lien l’identifiant qui déverrouille le paiement.",
  "error.missing_return_token": "Il manque son jeton à ce lien de retour.",
  "error.retry": "Réessayer",

  "memory.remember_number": "Mémoriser ce numéro sur cet appareil",
  "memory.remember_method": "Mémoriser {rail} sur cet appareil",
  "memory.warning":
    "Toute autre personne qui utilise cet appareil le verra. Ne cochez pas cette case sur un téléphone partagé ou emprunté.",
  "memory.forget": "Oublier ce que cet appareil a mémorisé",
  "memory.forgotten": "Cet appareil ne mémorise plus rien.",
  "memory.last_used": "Dernier utilisé",

  "refusal.embed_title": "Cette page ne s’affichera pas ici",
  "refusal.embed_body":
    "vpay n’affiche une page de paiement intégrée que sur un site enregistré par le marchand. Demandez au marchand d’ajouter ce site à ses origines de paiement.",
};
