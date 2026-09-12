/**
 * Design tokens.
 *
 * Payment status is the one place colour carries meaning in this product, so
 * the mapping lives here rather than being re-invented per component.
 */

export const PAYMENT_STATUS = [
  "requires_payment_method",
  "requires_action",
  "processing",
  "succeeded",
  "canceled",
] as const;

export type PaymentStatus = (typeof PAYMENT_STATUS)[number];

/**
 * The three ends a payer's checkout can reach, as the checkout page names
 * them.
 *
 * A **separate** vocabulary from {@link PAYMENT_STATUS} on purpose. That one
 * is the operator's: it is a PaymentIntent's own status, read on a dashboard
 * by someone watching a payment move, and `requires_payment_method` there
 * means "awaiting a payment method" rather than "this failed". These three
 * are what one payer is told at the end of one attempt.
 */
export const CHECKOUT_OUTCOME = ["succeeded", "failed", "canceled"] as const;

export type CheckoutOutcome = (typeof CHECKOUT_OUTCOME)[number];

/**
 * daisyUI semantic colour per outcome, for the payer-facing screen.
 *
 * **Added 2026-09-07 because the screen was wrong.** The checkout page took
 * its outcome colour from `statusTone` through the intent status each outcome
 * implies, which mapped `failed` onto `requires_payment_method` and therefore
 * onto `neutral` — so a payer whose payment FAILED read a grey box while a
 * payer who CANCELED read a red one. That inversion was on the committed
 * screenshots (`docs/plans/exp21-checkout-page-notes/outcomes-hosted.png`).
 * Routing an operator's status palette through a payer's screen was the
 * defect; `statusTone` itself is right for what it is for and is unchanged,
 * so the dashboard is untouched.
 *
 * **`canceled` moved from `error` to `warning` on 2026-09-07 (decision D4,
 * docs/plans/2026-09-07-ui-revamp.md §9), taken by the maintainer's
 * delegate.** The design call this comment used to defer — whether a
 * payer's own cancellation should read less alarming than a payment that
 * failed for a reason outside their control — is now taken: `canceled` is
 * the payer's own action, and `failed` is not. `failed` stays `error`.
 */
export const checkoutOutcomeTone: Record<
  CheckoutOutcome,
  "neutral" | "info" | "warning" | "success" | "error"
> = {
  succeeded: "success",
  failed: "error",
  canceled: "warning",
};

/**
 * `checkoutOutcomeTone` expressed in `@vaam-apps/ui`'s `InlineBanner` variant
 * vocabulary.
 *
 * **Added 2026-09-12, the `@vaam-apps/ui` cutover.** `InlineBanner`'s
 * variants are `"neutral" | "danger" | "warning" | "success" | "uncertain" |
 * "plain"` — daisyUI's semantic four plus two more, and no `"info"`. The
 * checkout's outcome screen only ever needs three of them, so this is a
 * narrow, hand-written translation rather than a general tone→variant
 * mapper: `error` becomes `danger`; `warning` and `success` keep their
 * names because the two vocabularies happen to share them.
 *
 * This is `checkoutOutcomeTone` re-expressed for the new component library,
 * **not re-decided** — decision D4 (2026-09-07, docs/plans/2026-09-07-ui-revamp.md
 * §9) lives in that table's history and this one must keep agreeing with it.
 * `index.test.ts` asserts the two tables agree on which outcome is alarming
 * and which is a success.
 */
export const checkoutOutcomeVariant: Record<
  CheckoutOutcome,
  "danger" | "warning" | "success"
> = {
  succeeded: "success",
  failed: "danger",
  canceled: "warning",
};

/**
 * Copy shown to an operator.
 *
 * `processing` deliberately does not say "pending" — the whole point of that
 * state is that nothing has been decided, and the dashboard must not imply
 * a payment is nearly done. See docs/flows/payment-lifecycle.md.
 *
 * **As of 2026-09-12 this is this package's only `PaymentStatus` export.**
 * `statusTone` (daisyUI semantic colour per status) was deleted in the
 * `@vaam-apps/ui` cutover: its one consumer, `@vpay/ui`'s `StatusBadge`, was
 * deleted with it, and nothing else read it. The dashboard's status
 * colour and glyph now come from
 * `frontends/apps/dashboard/src/payment-status.ts`'s `defineStatusSystem`
 * table, which reads `label` from here so operator-facing copy still has
 * exactly one source.
 */
export const statusLabel: Record<PaymentStatus, string> = {
  requires_payment_method: "Awaiting payment method",
  requires_action: "Awaiting payer on the rail",
  processing: "In flight — not yet decided",
  succeeded: "Succeeded",
  canceled: "Canceled",
};
