// @vitest-environment jsdom
/**
 * Axe contrast checks on the four outcome screens — issue #73.
 *
 * **CRITICAL LIMITATION: jsdom does not compute CSS colours.** This test runs
 * axe-core's `color-contrast` rule against the rendered outcome screens
 * (succeeded, failed, canceled, and the return screen outcome), but jsdom
 * parses no paint and computes no layout. A violation that exists in the
 * rendered page will **not** be reported here; a violation reported here is
 * impossible (jsdom would have had to compute the contrast, which it cannot).
 *
 * "A green jsdom contrast run is not evidence." (plan §7 row 6) — this test
 * documents the _requirement_ to check, and aspirationally asserts zero
 * violations. **The real verification is in a real browser.** This test will
 * fail when a real browser test (cypress-axe, plan §7 row 6) is built, because
 * that is when actual colour rendering can be measured.
 *
 * The @vpay/ui library's `theme-contrast.test.ts` measures contrast by
 * compiling the stylesheet with PostCSS and Tailwind and computing WCAG
 * ratios from the generated CSS variables — a measured check of the theme
 * palette, not of specific component compositions. What this suite checks is
 * whether the outcome screens (the highest-stakes screens — failure and
 * success, where text colour and background tone must be readable) are
 * rendered with accessible contrast when composed into the actual page.
 *
 * The decision (issue #73) is to "run axe on the four outcome screens under
 * bumblebee in test-web", and this is test-web's answer. When a real browser
 * can measure contrast, this will be superseded by that test, and this one
 * can be deleted.
 */
import { render } from "@testing-library/react";
import { axeContrastViolations } from "@vpay/ui/testing";
import { describe, expect, it } from "vitest";

import { LOCALES, translator, type Locale } from "../i18n/index";
import type { CheckoutState } from "../lib/machine";
import { makeBranding, makeMemoryControls } from "../testing/fixtures";
import { CHECKOUT_SCREENS, RETURN_SCREENS } from "../testing/screen-states";
import { CheckoutView, type CheckoutViewProps } from "./checkout-view";
import { ReturnView } from "./return-view";

const NOOP = () => undefined;

/** The same props `checkout-view.test.tsx`'s own `renderState` uses. */
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

describe("outcome screens, contrast under bumblebee (issue #73)", () => {
  describe("checkout outcomes", () => {
    for (const locale of LOCALES) {
      it(`succeeded outcome has no color-contrast violations (${locale})`, async () => {
        const { container, unmount } = renderCheckout(
          CHECKOUT_SCREENS.outcome_succeeded,
          locale,
        );
        const violations = await axeContrastViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });

      it(`failed outcome has no color-contrast violations (${locale})`, async () => {
        const { container, unmount } = renderCheckout(
          CHECKOUT_SCREENS.outcome_failed,
          locale,
        );
        const violations = await axeContrastViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });

      it(`canceled outcome has no color-contrast violations (${locale})`, async () => {
        const { container, unmount } = renderCheckout(
          CHECKOUT_SCREENS.outcome_canceled,
          locale,
        );
        const violations = await axeContrastViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });
    }
  });

  describe("return screen outcomes", () => {
    for (const locale of LOCALES) {
      it(`return succeeded outcome has no color-contrast violations (${locale})`, async () => {
        const { container, unmount } = renderReturn(
          RETURN_SCREENS.outcome_succeeded,
          locale,
        );
        const violations = await axeContrastViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });

      it(`return failed outcome has no color-contrast violations (${locale})`, async () => {
        const { container, unmount } = renderReturn(
          RETURN_SCREENS.outcome_failed,
          locale,
        );
        const violations = await axeContrastViolations(container);
        expect(
          violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`),
        ).toEqual([]);
        unmount();
      });
    }
  });
});
