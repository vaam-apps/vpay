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
 * The rule set is `@vpay/ui`'s own `axeViolations` — the fifteen DOM-structure
 * and ARIA checks plan §7 row 5 names — imported rather than restated, so the
 * two suites cannot drift. Contrast is deliberately NOT among them: jsdom
 * computes nothing from the built stylesheet, and "a green jsdom contrast run
 * is not evidence" (plan §7 row 6). The theme's contrast is measured for real
 * in `@vpay/ui`'s `theme-contrast.test.ts`.
 *
 * Every screen in both state maps, in both locales — the screens are cheap and
 * a violation that only appears in French is exactly the one a review misses.
 */
import { render } from '@testing-library/react';
import { axeViolations } from '@vpay/ui/testing';
import { describe, expect, it } from 'vitest';

import { LOCALES, translator, type Locale } from '../i18n/index';
import type { CheckoutState } from '../lib/machine';
import { makeBranding, makeMemoryControls } from '../testing/fixtures';
import { CHECKOUT_SCREENS, RETURN_SCREENS } from '../testing/screen-states';
import { CheckoutView, type CheckoutViewProps } from './checkout-view';
import { ReturnView } from './return-view';

const NOOP = () => undefined;

/** The same props `checkout-view.test.tsx`'s own `renderState` uses, so the two suites render the same page. */
function renderCheckout(state: CheckoutState, locale: Locale) {
  const props: CheckoutViewProps = {
    state,
    t: translator(locale),
    locale,
    branding: makeBranding(),
    destination: 'https://shop.example/ok?sid=cs_test_fixture000000000001',
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

function renderReturn(state: React.ComponentProps<typeof ReturnView>['state'], locale: Locale) {
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

describe('every checkout screen, structurally (plan §7 row 5)', () => {
  for (const [name, state] of Object.entries(CHECKOUT_SCREENS)) {
    for (const locale of LOCALES) {
      it(`has no structural accessibility violation: ${name} (${locale})`, async () => {
        const { container, unmount } = renderCheckout(state, locale);
        const violations = await axeViolations(container);
        expect(violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`)).toEqual([]);
        unmount();
      });
    }
  }

  for (const [name, state] of Object.entries(RETURN_SCREENS)) {
    for (const locale of LOCALES) {
      it(`has no structural accessibility violation: return/${name} (${locale})`, async () => {
        const { container, unmount } = renderReturn(state, locale);
        const violations = await axeViolations(container);
        expect(violations.map((v) => `${v.id}: ${v.nodes.length} node(s)`)).toEqual([]);
        unmount();
      });
    }
  }
});
