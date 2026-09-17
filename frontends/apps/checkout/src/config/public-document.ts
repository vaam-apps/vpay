/**
 * The one public description of what this deployment looks like, built once
 * and served from two places for two different audiences.
 *
 * `GET /.well-known/vpay-checkout` is the **client contract** (#193). An SDK
 * fetches it, caches it locally, and renders from it — a support contact on
 * a payment sheet, the rails an operator narrowed to, the brand colour a
 * merchant's app should wear. Because an SDK caches it, its shape is a
 * promise: `version` exists so it can change without stranding an installed
 * app, and a field may be added but not silently repurposed.
 *
 * `GET /config/v1` is the **operator's verification surface**, and says so
 * in its own file. It answers "did the mount work, and did this pod read
 * what I think it read?" and is free to change shape whenever that question
 * is better answered differently.
 *
 * Both render this same document, from the same memoised startup read, so
 * an operator checking the second cannot be looking at something the first
 * would not say.
 *
 * # What is deliberately not here
 *
 * **Nothing payment-critical.** Rails, payer fields, validation regions and
 * flow all arrive per-session from `GET /v1/browser/checkout/sessions/{id}`,
 * adapter-declared and server-enforced (#186). That separation is what makes
 * this document safe to cache for a long time and safe to miss entirely: a
 * stale or absent copy changes how a checkout *looks*, never whether or how
 * a payment can happen. Putting a validation rule here would break that, by
 * creating a cached rule that can drift from the one the server enforces.
 *
 * **No file paths and no problem list.** Both are in the container's log,
 * where the person who can act on them is looking. On a public URL they
 * would tell a stranger how the deployment is laid out and how it is broken.
 *
 * Everything that *is* here is already on the payment page a payer can load,
 * so this discloses nothing new.
 */
import { runtimeConfig } from "./runtime";

/** The wire shape. `snake_case` because it crosses to non-TypeScript SDKs. */
export interface CheckoutPageConfigDocument {
  readonly object: "checkout_page_config";
  readonly version: 1;
  readonly branding: {
    readonly display_name: string | null;
    readonly logo_url: string | null;
    readonly primary_color: string | null;
    readonly support_contact: string | null;
  };
  readonly checkout: {
    readonly public_base_url: string | null;
    readonly allowed_methods: readonly string[] | null;
    readonly features: { readonly page_memory: boolean };
  };
}

/**
 * Builds the document from whatever this container actually loaded.
 *
 * `allowed_methods` is the operator's narrowing, and an SDK must treat it as
 * a **floor it may narrow further but never widen** — the same semantics the
 * page itself applies (`src/lib/rails.ts`'s `railChoices`, where an operator
 * allow-list can only ever remove a rail the intent offered). `null` means
 * the operator expressed no opinion, which is not the same as an empty list.
 */
export function checkoutPageConfigDocument(): CheckoutPageConfigDocument {
  const { branding, checkout } = runtimeConfig();
  return {
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
  };
}
