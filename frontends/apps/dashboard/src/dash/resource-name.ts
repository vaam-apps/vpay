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
 * The four procedure-backed read slices.
 *
 * Each is a CrateStack **procedure** rather than a REST route, so the name
 * here is the resource Refine routes on, and `PROCEDURE_RESOURCES` below
 * says which they are.
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
 * The resources a CrateStack **procedure** serves — every one but
 * `payment_intents`, which is `GET /dash/v1/payment_intents`, a cursor-paged
 * REST route shared with the merchant API, and stays on its own provider.
 *
 * Which procedure serves which is no longer a string map here. Until
 * 2026-10-26 `PROCEDURE_OF` held `"searchRefunds"` and its siblings as text,
 * and a misspelling was a `404` nobody noticed until a screen was empty. The
 * mapping is now a call into the generated client
 * (`src/dash/cratestack-client.ts`'s `LIST_OF`), so a procedure renamed in
 * `schemas/vpay.cstack` is a compile error after `just gen-dash-client`.
 */
export const PROCEDURE_RESOURCES = [
  REFUNDS,
  WEBHOOK_DELIVERIES,
  CUSTOMERS,
  CHECKOUT_SESSIONS,
] as const;

export type ProcedureResource = (typeof PROCEDURE_RESOURCES)[number];

export function isProcedureResource(
  resource: string,
): resource is ProcedureResource {
  return (PROCEDURE_RESOURCES as readonly string[]).includes(resource);
}

/**
 * Where the browser reads each one: `app/api/dash/<path>/route.ts`.
 *
 * Two names differ from the resource — `deliveries` and `checkouts` are the
 * screens' own words — which is why this is a map and not the resource name
 * spliced into a URL.
 */
export const BFF_PATH_OF: Readonly<Record<ProcedureResource, string>> = {
  [REFUNDS]: "refunds",
  [WEBHOOK_DELIVERIES]: "deliveries",
  [CUSTOMERS]: "customers",
  [CHECKOUT_SESSIONS]: "checkouts",
};
