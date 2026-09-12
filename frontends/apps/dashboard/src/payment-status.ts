import { statusLabel, type PaymentStatus } from "@vpay/tokens";
import { createStatusPill, defineStatusSystem } from "@vaam-apps/ui";

/**
 * The dashboard's one PaymentStatus → glyph/hue/copy table.
 *
 * `label` reads from `@vpay/tokens`'s `statusLabel` so the operator-facing
 * copy has exactly one source and cannot drift between this table and
 * anywhere else that might want to name a status. Everything else here is
 * presentational only, per `@vaam-apps/ui`'s own warning on `StatusMeta`:
 * never use `family` — or anything else in this table — to decide whether an
 * action is permitted. Terminality is data the server owns; propose the
 * action and let the server refuse it.
 *
 * `PaymentStatus` is passed as an explicit type argument, not inferred, so a
 * status added to `@vpay/tokens` without a matching row here is a compile
 * error — the same guarantee `asPaymentStatus` and
 * `payments-filters.tsx`'s "one vocabulary" comment already assert in prose.
 *
 * **No state takes `hue: "danger"`.** A rail decline returns the intent to
 * `requires_payment_method` carrying `last_payment_error`; there is no
 * failure state for a PaymentIntent, and a red pill here would be a lie to
 * an operator.
 */
export const PAYMENT_STATUS_SYSTEM = defineStatusSystem<PaymentStatus>({
  requires_payment_method: {
    family: "in-flight",
    silhouette: "circle",
    mark: "pie-1",
    hue: "neutral",
    filled: false,
    attention: "quiet",
    label: statusLabel.requires_payment_method, // "Awaiting payment method"
    tooltip:
      "Created and not yet confirmed — the only status confirm and cancel are legal from. A rail that declines a charge also returns the intent here, carrying last_payment_error.",
  },
  requires_action: {
    family: "in-flight",
    silhouette: "circle",
    mark: "ring",
    hue: "warning",
    filled: false,
    attention: "quiet",
    label: statusLabel.requires_action, // "Awaiting payer on the rail"
    tooltip:
      "Redirect rails only. The payer has been handed to the rail's hosted page and vpay is waiting for them to act.",
  },
  // hue: "parked" / mark: "ring" per decision 5 (2026-09-12) — supersedes an
  // earlier "uncertain vs neutral" question. Both of those were wrong:
  // `uncertain` would make every normal in-flight payment read as a
  // problem, and `neutral` would make a state waiting for someone look
  // identical to one nobody needs to act on. This state is "handed off,
  // waiting for someone" — the payer, on the rail's handset — which is
  // exactly what `parked` (StatusHue's own doc comment) and the `ring` mark
  // ("a completed stroke with a hollow centre — handed off, awaiting an
  // external answer") both mean already, in the library's own vocabulary.
  processing: {
    family: "in-flight",
    silhouette: "circle",
    mark: "ring",
    hue: "parked",
    filled: false,
    attention: "quiet",
    label: statusLabel.processing, // "In flight — not yet decided"
    tooltip:
      "The rail has the charge and the reconciler owns what happens next. Nothing has been decided; this is not 'nearly done'.",
  },
  succeeded: {
    family: "terminal",
    silhouette: "circle",
    mark: "check",
    hue: "success",
    filled: true,
    attention: "quiet",
    label: statusLabel.succeeded, // "Succeeded"
    tooltip: "The money moved. Terminal.",
  },
  canceled: {
    family: "terminal",
    silhouette: "square",
    mark: "slash",
    hue: "warning",
    filled: true,
    attention: "quiet",
    label: statusLabel.canceled, // "Canceled"
    tooltip: "Cancelled before any rail saw it. Terminal.",
  },
});

/** The one place a PaymentIntent status is drawn. */
export const PaymentStatusPill = createStatusPill(PAYMENT_STATUS_SYSTEM);
