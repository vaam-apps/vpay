import { NextResponse } from "next/server";

import { checkoutPageConfigDocument } from "../../../src/config/public-document";

/**
 * `GET /config/v1` — what this container actually loaded.
 *
 * **A verification surface for an operator, not an API a browser uses.**
 * Every page already receives its branding and settings as props, injected
 * at render from the memoised startup read (`src/config/runtime.ts`), so
 * nothing on the payment path fetches this. What it answers is the question
 * a mounted-file feature always raises — *did the mount work, and did this
 * pod read what I think it read?* — without a shell in the container.
 *
 * **Versioned in the path** (`/config/v1`), so the shape can change without
 * breaking whatever an operator wired to it.
 *
 * **What is deliberately not here:** the file paths, and the problem list.
 * Both are in the container's own log, where the person who can act on them
 * is looking; on a public URL they would tell a stranger how the deployment
 * is laid out and how it is broken. Everything that *is* here is on the
 * payment page already — a name, a logo URL, a colour, a support contact,
 * the page's own base URL, the rails it will show — so this endpoint
 * discloses nothing a payer could not read off the page.
 *
 * `dynamic = 'force-dynamic'` for the same reason the pages set it: the
 * `no-store` `middleware.ts` sends should be the whole caching story.
 */
export const dynamic = "force-dynamic";

export function GET(): NextResponse {
  // The same document `/.well-known/vpay-checkout` serves (#193), so an
  // operator checking this surface cannot be reading something the SDK's
  // contract would not say. The two routes differ in audience and in what
  // they promise about shape — see `src/config/public-document.ts` — never
  // in content.
  return NextResponse.json(checkoutPageConfigDocument());
}
