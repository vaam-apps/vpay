/**
 * Builds a `Vpay-Signature` header the way vpay's worker does
 * (docs/flows/webhooks.md: `t=<unix seconds>,v1=<hex hmac of "<t>.<body>">`).
 *
 * Written out here rather than borrowed from the SDK on purpose: the SDK is
 * the *verifier* under test, and signing with the verifier's own helper would
 * be a test that proves a function agrees with itself. This is an independent
 * implementation of the documented grammar.
 *
 * Test-only.
 */
import { createHmac } from "node:crypto";

export function signWebhook(
  rawBody: string,
  secret: string,
  timestamp: number,
): string {
  const mac = createHmac("sha256", secret)
    .update(`${timestamp}.${rawBody}`, "utf8")
    .digest("hex");
  return `t=${timestamp},v1=${mac}`;
}
