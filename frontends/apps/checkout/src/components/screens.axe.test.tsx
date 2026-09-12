// @vitest-environment jsdom
/**
 * Plan §7 row 5: axe-core over **every checkout screen**, structural rules
 * only, zero violations.
 *
 * `@vpay/ui` had this for its own components from Lane A; nothing ran it over
 * the screens those components are composed into, which is where the
 * interesting failures live. A `Field` that passes in isolation can still be
 * mounted without a label; a `Stack` that passes in isolation can still be
 * the `<header>` that isn't one any more. Both of those were real on this
 * branch.
 *
 * **The helper moved in-app (2026-09-12, the `@vaam-apps/ui` cutover).**
 * `@vpay/ui` is deleted in this change and this file was its one consumer
 * outside the package's own tests, so `axeViolations` now lives at
 * `../testing/axe`, ported verbatim (the same fifteen DOM-structure and
 * ARIA rule names, so the two suites — this one and the deleted package's
 * own — could not drift while both existed). Contrast is deliberately NOT
 * among them: jsdom computes nothing from the built stylesheet, and "a
 * green jsdom contrast run is not evidence" (plan §7 row 6). The theme's
 * contrast is measured for real in `outcome-contrast.test.ts`, from the
 * compiled sheet directly.
 *
 * **The anti-vacuity assertion below is new (2026-09-12).** `axe.run` over a
 * container that rendered nothing reports zero violations, indistinguishable
 * from a clean screen — the same failure mode `no-runtime-imports.test.ts`
 * guards against for an empty file list. `axeEvaluatedRuleCount` is asserted
 * nonzero, in every single test case, BEFORE the violations list is trusted.
 * Mutated and observed failing: replacing `renderCheckout`'s body with
 * `render(<></>)` makes every `axeEvaluatedRuleCount` assertion FAIL while
 * the (now vacuous) violations-array assertion would still have passed —
 * see this app's migration report for the recorded run.
 *
 * Every screen in both state maps, in both locales — the screens are cheap and
 * a violation that only appears in French is exactly the one a review misses.
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { axeEvaluatedRuleCount, axeViolations } from "../testing/axe";

import { LOCALES, translator, type Locale } from "../i18n/index";
import type { CheckoutState } from "../lib/machine";
import { makeBranding, makeMemoryControls } from "../testing/fixtures";
import { CHECKOUT_SCREENS, RETURN_SCREENS } from "../testing/screen-states";
import { CheckoutView, type CheckoutViewProps } from "./checkout-view";
import { ReturnView } from "./return-view";

const NOOP = () => undefined;

/** The same props `checkout-view.test.tsx`'s own `renderState` uses, so the two suites render the same page. */
function renderCheckout(state: CheckoutState, locale: Locale) {
  const props: CheckoutViewProps = {
    state,
    t: translator(locale),
    locale,
    branding: makeBranding(),
    destination: "https://shop.example/ok?sid=cs_test_fixture000000000001",
    defaultMsisdn: null,
    lastRail: null,
    memory: makeMemoryControls(),
    onChooseRail: NOOP,
    onBack: NOOP,
    onSubmitMsisdn: NOOP,
    onStartRedirect: NOOP,
    onRetryPoll: NOOP,
    onReturnToMerchant: NOOP,
    onLocaleChange: NOOP,
  };
  return render(<CheckoutView {...props} />);
}

function renderReturn(
  state: React.ComponentProps<typeof ReturnView>["state"],
  locale: Locale,
) {
  return render(
    <ReturnView
      state={state}
      t={translator(locale)}
      locale={locale}
      branding={makeBranding()}
      destination="https://shop.example/done"
      onReturnToMerchant={NOOP}
      onLocaleChange={NOOP}
    />,
  );
}

describe("every checkout screen, structurally (plan §7 row 5)", () => {
  // A guard on the screen count itself: `CHECKOUT_SCREENS`/`RETURN_SCREENS`
  // drive the `for` loops below, so a state map that silently emptied would
  // produce zero test cases and vitest would report green. The same shape
  // `no-runtime-imports.test.ts` already uses for its own file list.
  it("covers more than a handful of checkout states", () => {
    expect(Object.keys(CHECKOUT_SCREENS).length).toBeGreaterThan(8);
  });
  it("covers more than a handful of return states", () => {
    expect(Object.keys(RETURN_SCREENS).length).toBeGreaterThan(3);
  });

  for (const [name, state] of Object.entries(CHECKOUT_SCREENS)) {
    for (const locale of LOCALES) {
      it(`has no structural accessibility violation: ${name} (${locale})`, async () => {
        const { container, unmount } = renderCheckout(state, locale);
        // A run that inspected nothing reports no violations, which is
        // indistinguishable from a clean screen. Assert axe actually walked
        // the rules it was given before trusting an empty violation list.
        expect(
          await axeEvaluatedRuleCount(container),
          "axe evaluated no rule — the container rendered nothing",
        ).toBeGreaterThan(0);
        const violations = await axeViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });
    }
  }

  for (const [name, state] of Object.entries(RETURN_SCREENS)) {
    for (const locale of LOCALES) {
      it(`has no structural accessibility violation: return/${name} (${locale})`, async () => {
        const { container, unmount } = renderReturn(state, locale);
        expect(
          await axeEvaluatedRuleCount(container),
          "axe evaluated no rule — the container rendered nothing",
        ).toBeGreaterThan(0);
        const violations = await axeViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });
    }
  }
});
