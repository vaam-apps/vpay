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

/** Every resource this app may ask for — there is exactly one. */
export type DashResource = typeof PAYMENT_INTENTS;
