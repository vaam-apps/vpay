/**
 * Literal objects in the wire contract's shapes, for the tests that need a
 * state rather than a server.
 *
 * Test-only, like `browser-stub.ts` and `screen-states.ts` beside it:
 * nothing under `src/testing` is named by a file that ships — not `app/`,
 * not `middleware.ts`, not a component — so none of it can reach the built
 * page. `no-runtime-imports.test.ts` fails if that stops being true.
 * **Updated 2026-09-12:** a `.stories.tsx` used to be the one exception —
 * `checkout-screens.stories.tsx` named this file, built by Storybook
 * (`pnpm --filter @vpay/ui build-storybook`), not by `next build`. Both the
 * package hosting that Storybook install and the stories file itself were
 * deleted in the `@vaam-apps/ui` cutover; a chip is filed to restore
 * Storybook inside this app. Nothing names this file outside tests today.
 */
import type { PaymentIntent } from "@vaam-apps/vpay-stripe-js";

import type { MemoryControls } from "../components/screens";
import type { Branding } from "../config/settings";
import type { CheckoutContext } from "../lib/machine";
import type { CheckoutSession, PublicPaymentIntent } from "../lib/types";

export const SESSION_ID = "cs_test_fixture000000000001";
export const INTENT_ID = "pi_test_fixture000000000001";
export const INTENT_SECRET = `${INTENT_ID}_secret_${"a".repeat(32)}`;

export function makeSession(
  overrides: Partial<CheckoutSession> = {},
): CheckoutSession {
  return {
    id: SESSION_ID,
    object: "checkout.session",
    livemode: false,
    ui_mode: "hosted",
    status: "open",
    payment_status: "unpaid",
    success_url: "https://shop.example/ok?sid={CHECKOUT_SESSION_ID}",
    cancel_url: "https://shop.example/cancel",
    return_url: null,
    url: `https://checkout.example/c/${SESSION_ID}`,
    expires_at: 1_757_000_000,
    created: 1_756_913_600,
    ...overrides,
  };
}

export function makeIntent(
  overrides: Partial<PaymentIntent> = {},
): PaymentIntent {
  return {
    id: INTENT_ID,
    object: "payment_intent",
    amount: 5000,
    currency: "xaf",
    status: "requires_payment_method",
    payment_method_types: ["mtn_momo"],
    next_action: null,
    last_payment_error: null,
    metadata: {},
    description: null,
    created: 1_756_913_600,
    livemode: false,
    client_secret: INTENT_SECRET,
    ...overrides,
  };
}

/** The same intent as the return route renders it: no `client_secret`. */
export function makePublicIntent(
  overrides: Partial<PaymentIntent> = {},
): PublicPaymentIntent {
  const { client_secret: _secret, ...rest } = makeIntent(overrides);
  return rest;
}

/**
 * `merchantName: null` builds the context of a read that carried no usable
 * name; `allowedMethods` is the deployment's `config.yaml` policy, `null`
 * for a deployment with no opinion (which is every fixture but the ones
 * about the allow-list itself).
 */
export function makeContext(
  session: Partial<CheckoutSession> = {},
  intent: Partial<PaymentIntent> = {},
  merchantName: string | null = "Boutique Test",
  allowedMethods: readonly string[] | null = null,
): CheckoutContext {
  return {
    session: makeSession(session),
    intent: makeIntent(intent),
    merchant: merchantName === null ? null : { name: merchantName },
    allowedMethods,
  };
}

/**
 * `branding.yaml` as a deployment that configured nothing gets it.
 *
 * The default rather than a filled-in one, because that is what every
 * screen story and most assertions are about: the page as it renders with
 * no files mounted. The tests that are *about* branding pass overrides.
 */
export function makeBranding(overrides: Partial<Branding> = {}): Branding {
  return {
    displayName: null,
    logoUrl: null,
    primaryColor: null,
    supportContact: null,
    ...overrides,
  };
}

/** Page memory offered, nothing remembered, box unticked — the first visit. */
export function makeMemoryControls(
  overrides: Partial<MemoryControls> = {},
): MemoryControls {
  return {
    offered: true,
    remember: false,
    onRememberChange: () => undefined,
    hasRecord: false,
    onForget: () => undefined,
    forgotten: false,
    ...overrides,
  };
}
