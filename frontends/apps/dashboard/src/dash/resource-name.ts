/**
 * The one resource name, in a module a **browser** may import.
 *
 * It lived in `provider.ts` until the payments screens began reading through
 * Refine. That module is server-side — it reaches `readDash`, the config and
 * `node:crypto` through them — so a client component importing the constant
 * from it dragged the whole server graph into the browser bundle and the
 * build failed with `Reading from "node:crypto" is not handled by plugins`.
 *
 * A constant is not a reason to bundle a credential path. `provider.ts`
 * re-exports it, so no caller has to know which of the two modules to reach
 * for and there is still exactly one spelling of the string.
 */
export const PAYMENT_INTENTS = "payment_intents";

/**
 * The three Lane D read slices.
 *
 * Each is a CrateStack **procedure** rather than a REST route, so the name
 * here is the resource Refine routes on and `PROCEDURE_OF` below maps it to
 * the procedure the transport actually mounts. Keeping both spellings in one
 * file is deliberate: a resource whose procedure name is guessed at the call
 * site is a 404 nobody notices until a screen is empty.
 */
export const REFUNDS = "refunds";
export const WEBHOOK_DELIVERIES = "webhook_deliveries";
export const CUSTOMERS = "customers";
export const CHECKOUT_SESSIONS = "checkout_sessions";

/** Every resource this app may ask for. */
export type DashResource =
  | typeof PAYMENT_INTENTS
  | typeof REFUNDS
  | typeof WEBHOOK_DELIVERIES
  | typeof CUSTOMERS
  | typeof CHECKOUT_SESSIONS;

/**
 * Which procedure serves which resource.
 *
 * `payment_intents` is absent on purpose: it is served by
 * `GET /dash/v1/payment_intents`, a cursor-paged REST route shared with the
 * merchant API, not by a procedure. A lookup that misses here means "this
 * resource is not a procedure", which is a different thing from "unknown
 * resource" and is why this is a partial map rather than a total one.
 */
export const PROCEDURE_OF: Readonly<Record<string, string>> = {
  [REFUNDS]: "searchRefunds",
  [WEBHOOK_DELIVERIES]: "searchWebhookDeliveries",
  [CUSTOMERS]: "searchCustomers",
  [CHECKOUT_SESSIONS]: "searchCheckoutSessions",
};
