import { NextResponse } from "next/server";

import { runtimeConfig } from "../../../src/config/runtime";

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
  const { branding, checkout } = runtimeConfig();
  return NextResponse.json({
    object: "checkout_page_config",
    version: 1,
    branding: {
      display_name: branding.displayName,
      logo_url: branding.logoUrl,
      primary_color: branding.primaryColor,
      support_contact: branding.supportContact,
    },
    checkout: {
      public_base_url: checkout.publicBaseUrl,
      allowed_methods: checkout.allowedMethods,
      features: { page_memory: checkout.features.pageMemory },
    },
  });
}
