import { NextResponse } from "next/server";

import { checkoutPageConfigDocument } from "../../../src/config/public-document";

/**
 * `GET /.well-known/vpay-checkout` — what an SDK needs to know about this
 * deployment before a payer taps anything (#193).
 *
 * The payload and the reasoning live in
 * [`src/config/public-document.ts`](../../../src/config/public-document.ts);
 * this file is the route and the caching posture, which are the two things
 * that differ from `/config/v1`.
 *
 * # Why a well-known path
 *
 * An SDK is given a checkout base URL and nothing else. RFC 8615 is the
 * convention for "ask an origin about itself without being told a second
 * URL", and a single path segment (`vpay-checkout`) is how that registry is
 * shaped — not a vendor-nested one.
 *
 * # Caching, and why it is not done here
 *
 * `middleware.ts` puts `Cache-Control: no-store` on every path, and its own
 * comment defends the absence of exclusions: "the alternative is a matcher
 * whose exclusions are the one part of the security headers nobody
 * re-reads." That judgement is not worth reversing for this document, so it
 * is **not** reversed: the response stays `no-store` on the wire, and the
 * caching happens in the SDK, which fetches on `prepareCheckout()` at a
 * moment the integrator picks and keeps the answer under its own TTL.
 *
 * That places the cache where the staleness is survivable. This document
 * only affects presentation — nothing payment-critical is in it — so an SDK
 * holding a copy for a day is a cosmetic risk, while a shared handset
 * holding a cached *payment page* is not a risk anyone should take.
 *
 * `dynamic = 'force-dynamic'` for the same reason every other route here
 * sets it: whatever this pod loaded at startup is the answer, and a
 * build-time snapshot of it would be a different, quieter lie.
 */
export const dynamic = "force-dynamic";

export function GET(): NextResponse {
  return NextResponse.json(checkoutPageConfigDocument());
}
