// @vitest-environment jsdom
/**
 * Every screen, in both languages, plus the accessibility properties that
 * make the keyboard-only path possible.
 *
 * The locale assertions are generated from the dictionaries rather than
 * written out: the test looks up the string the screen is supposed to show
 * *in that locale's own dictionary* and asserts it is on screen. A test that
 * quoted the French sentence inline would pass just as happily if the page
 * rendered English.
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { DICTIONARIES, LOCALES, format, translator, type Locale } from '../i18n/index';
import type { CheckoutState } from '../lib/machine';
import { formatAmount } from '../lib/money';
import {
  SESSION_ID,
  makeBranding,
  makeContext,
  makeMemoryControls,
  makePublicIntent,
  makeSession,
} from '../testing/fixtures';
import { CheckoutView, type CheckoutViewProps } from './checkout-view';
import { ReturnView } from './return-view';
import { CHECKOUT_SCREENS, RETURN_SCREENS } from '../testing/screen-states';

const NOOP = () => undefined;

function renderState(state: CheckoutState, locale: Locale, overrides: Partial<CheckoutViewProps> = {}) {
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
    ...overrides,
  };
  return render(<CheckoutView {...props} />);
}

/** The return view's props, with the same defaults, so a story-shaped literal stays one line. */
function renderReturn(
  state: React.ComponentProps<typeof ReturnView>['state'],
  locale: Locale,
  overrides: Partial<React.ComponentProps<typeof ReturnView>> = {},
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
      {...overrides}
    />,
  );
}

/** The `data-screen` each state is expected to render, so a silent fallthrough fails. */
const EXPECTED_SCREEN: Record<string, string> = {
  loading: 'loading',
  error: 'error',
  refused_embed: 'refused_embed',
  refused_rail: 'refused_rail',
  expired: 'expired',
  select_rail: 'select_rail',
  collect_msisdn: 'collect_msisdn',
  collect_msisdn_invalid: 'collect_msisdn',
  ready_redirect: 'ready_redirect',
  confirming: 'confirming',
  waiting: 'waiting',
  waiting_notice: 'waiting',
  redirecting: 'redirecting',
  outcome_succeeded: 'outcome',
  outcome_failed: 'outcome',
  outcome_canceled: 'outcome',
  forwarding: 'forwarding',
};

describe('every screen renders in every locale', () => {
  it('covers each state the machine can be in', () => {
    expect(Object.keys(CHECKOUT_SCREENS).sort()).toEqual(Object.keys(EXPECTED_SCREEN).sort());
  });

  for (const [name, state] of Object.entries(CHECKOUT_SCREENS)) {
    for (const locale of LOCALES) {
      it(`renders ${name} in ${locale}`, () => {
        const { container, unmount } = renderState(state, locale);
        const heading = container.querySelector('[data-screen]');
        expect(heading?.getAttribute('data-screen')).toBe(EXPECTED_SCREEN[name]);
        expect(heading?.textContent?.trim()).not.toBe('');
        unmount();
      });
    }
  }
});

describe('the copy on screen is the chosen locale’s, not the other one’s', () => {
  const CASES: [string, keyof (typeof DICTIONARIES)['en']][] = [
    ['loading', 'state.loading'],
    ['error', 'error.session_not_found'],
    ['refused_embed', 'refusal.embed_title'],
    ['expired', 'expired.title'],
    ['select_rail', 'rail.legend'],
    ['collect_msisdn', 'msisdn.label'],
    ['collect_msisdn_invalid', 'msisdn.invalid'],
    ['ready_redirect', 'state.redirecting_body'],
    ['confirming', 'state.confirming'],
    ['waiting', 'state.waiting_title'],
    ['waiting_notice', 'error.network'],
    ['redirecting', 'state.redirecting_title'],
    ['outcome_succeeded', 'outcome.succeeded_title'],
    ['outcome_failed', 'failure.insufficient_funds'],
    ['outcome_canceled', 'outcome.canceled_title'],
  ];

  for (const [name, key] of CASES) {
    for (const locale of LOCALES) {
      it(`${name} shows the ${locale} value of ${key}`, () => {
        const state = CHECKOUT_SCREENS[name] as CheckoutState;
        const { container, unmount } = renderState(state, locale);
        const expected = format(DICTIONARIES[locale][key], {
          merchant: 'Boutique Test',
          seconds: 5,
        });
        expect(container.textContent).toContain(expected);
        unmount();
      });
    }
  }
});

describe('the summary', () => {
  it('shows the amount in minor units correctly — 5000 XAF is five thousand', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState, 'en');
    expect(screen.getByTestId('amount').textContent?.replace(/[^0-9.]/g, '')).toBe('5000');
    unmount();
  });

  it('shows the test-mode banner while livemode is false, and hides it otherwise', () => {
    const state = CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState;
    const { unmount } = renderState(state, 'fr');
    expect(screen.queryByTestId('testmode')).not.toBeNull();
    unmount();

    const live = structuredClone(state) as CheckoutState & { context: { session: { livemode: boolean } } };
    live.context.session.livemode = true;
    const second = renderState(live, 'fr');
    expect(second.queryByTestId('testmode')).toBeNull();
    second.unmount();
  });
});

describe('a session whose read carried no merchant name', () => {
  /** The same screens, with `context.merchant` as `merchantOf` renders a missing name. */
  const MTN = { code: 'mtn_momo', flow: 'mobile_money_push', label: 'rail.mtn_momo' } as const;
  const UNNAMED: Record<string, CheckoutState> = {
    collect_msisdn: {
      name: 'collect_msisdn',
      context: makeContext({}, {}, null),
      rails: { supported: [MTN], unsupported: [] },
      rail: MTN,
      problem: null,
    },
    expired: { name: 'expired', context: makeContext({ status: 'expired' }, {}, null) },
    outcome: {
      name: 'outcome',
      context: makeContext(
        { status: 'complete', payment_status: 'paid' },
        { status: 'succeeded' },
        null,
      ),
      kind: 'succeeded',
      failure: null,
      reason: null,
    },
  };

  /** Scoped to this render's own container: several of these mount two views. */
  function text(container: HTMLElement, testId: string): string {
    return container.querySelector(`[data-testid="${testId}"]`)?.textContent ?? '';
  }

  for (const locale of LOCALES) {
    it(`shows the neutral heading rather than a hole in a sentence (${locale})`, () => {
      const { container, unmount } = renderState(UNNAMED['collect_msisdn'] as CheckoutState, locale);
      expect(text(container, 'pay-to')).toBe(DICTIONARIES[locale]['page.pay_to_unnamed']);
      unmount();
    });
  }

  it('puts no identifier, and no stand-in that reads like data, where the name would be', () => {
    const { container, unmount } = renderState(UNNAMED['outcome'] as CheckoutState, 'en');
    for (const testId of ['pay-to', 'outcome-body']) {
      const rendered = text(container, testId);
      // No unfilled placeholder, no id standing in for a name, and no dash
      // or empty gap left by a sentence written for a name.
      expect(rendered, testId).not.toContain('{merchant}');
      expect(rendered, testId).not.toContain('cs_test');
      expect(rendered, testId).not.toContain('pi_test');
      expect(rendered, testId).not.toContain('—');
      expect(rendered.trim(), testId).not.toBe('');
    }
    // The reference line still shows the session id: that is a labelled
    // reference, not a merchant's name.
    expect(text(container, 'reference')).toContain(SESSION_ID);
    unmount();
  });

  it('says the neutral sentence on every screen the name appears in', () => {
    const expired = renderState(UNNAMED['expired'] as CheckoutState, 'en');
    expect(text(expired.container, 'notice-body')).toBe(DICTIONARIES.en['expired.body_unnamed']);
    expired.unmount();

    const outcome = renderState(UNNAMED['outcome'] as CheckoutState, 'en');
    const amount = formatAmount(5000, 'xaf', 'en');
    expect(text(outcome.container, 'outcome-body')).toBe(
      format(DICTIONARIES.en['outcome.succeeded_body_unnamed'], { amount }),
    );
    expect(outcome.container.querySelector('[data-outcome] button')?.textContent).toBe(
      DICTIONARIES.en['outcome.back_to_unnamed'],
    );
    outcome.unmount();
  });

  it('still shows the name when the read carried one, in both sentences', () => {
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState,
      'en',
    );
    expect(text(container, 'pay-to')).toContain('Boutique Test');
    expect(text(container, 'outcome-body')).toContain('Boutique Test');
    expect(container.querySelector('[data-outcome] button')?.textContent).toContain(
      'Boutique Test',
    );
    unmount();
  });

  it('does the same on the return page', () => {
    const { container, unmount } = renderReturn(
      {
        name: 'outcome',
        context: {
          session: makeSession({ status: 'complete', payment_status: 'paid' }),
          intent: makePublicIntent({ status: 'succeeded' }),
          merchant: null,
        },
        kind: 'succeeded',
        failure: null,
        reason: null,
      },
      'fr',
    );
    expect(text(container, 'pay-to')).toBe(DICTIONARIES.fr['page.pay_to_unnamed']);
    expect(text(container, 'outcome-body')).toBe(
      format(DICTIONARIES.fr['outcome.succeeded_body_unnamed'], {
        amount: formatAmount(5000, 'xaf', 'fr'),
      }),
    );
    unmount();
  });
});

describe('accessibility', () => {
  it('moves focus to the new screen’s heading', () => {
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState,
      'fr',
    );
    expect(document.activeElement).toBe(container.querySelector('[data-screen]'));
    unmount();
  });

  it('keeps a polite live region mounted on every screen, from first render', () => {
    for (const state of Object.values(CHECKOUT_SCREENS)) {
      const { unmount } = renderState(state, 'en');
      const region = screen.getByTestId('live-region');
      expect(region.getAttribute('aria-live')).toBe('polite');
      unmount();
    }
  });

  it('gives every control an accessible name and a keyboard-reachable element', () => {
    for (const [name, state] of Object.entries(CHECKOUT_SCREENS)) {
      const { container, unmount } = renderState(state, 'fr');
      // Native elements, plus anything wearing an interactive ARIA role.
      // The second half is not a formality: Base UI's checkbox renders a
      // `<span role="checkbox" tabindex="0">`, so a query for native tags
      // alone would have walked straight past the one control on this page
      // that is not one — see the case below.
      const controls = container.querySelectorAll(
        'button, input, select, a[href], [role="checkbox"], [role="button"], [role="radio"], [role="switch"]',
      );
      for (const control of controls) {
        // `aria-hidden` elements are not in the accessibility tree at all.
        // Base UI's checkbox puts one there — a native input carrying the
        // form value beside the `role="checkbox"` button that IS the
        // control — and naming it would be naming something no assistive
        // technology can reach.
        if (control.getAttribute('aria-hidden') === 'true') {
          expect(control.getAttribute('tabindex'), `${name}: ${control.outerHTML}`).toBe('-1');
          continue;
        }
        const labelledBy = control.getAttribute('aria-labelledby');
        const labelled =
          control.getAttribute('aria-label') ??
          (labelledBy === null
            ? null
            : labelledBy
                .split(/\s+/)
                .map((id) => container.querySelector(`#${id}`)?.textContent ?? '')
                .join(' ')) ??
          (control.id.length > 0
            ? container.querySelector(`label[for="${control.id}"]`)?.textContent
            : null) ??
          control.textContent;
        expect(labelled?.trim(), `${name}: ${control.outerHTML}`).not.toBe('');
        // Reachable by keyboard: a native control, or an element that put
        // itself in the tab order on purpose.
        const native = ['BUTTON', 'INPUT', 'SELECT', 'A'].includes(control.tagName);
        if (!native) {
          expect(control.getAttribute('tabindex'), `${name}: ${control.outerHTML}`).toBe('0');
          // And it must report its own state, or a screen reader announces a
          // checkbox with nothing to say about whether it is ticked.
          if (control.getAttribute('role') === 'checkbox') {
            expect(['true', 'false'], `${name}`).toContain(control.getAttribute('aria-checked'));
          }
        }
      }
      // Nothing pretending to be a control WITHOUT being one: an element with
      // a handler and no role, or a role and no way to reach it.
      for (const impostor of container.querySelectorAll('div[onclick], span[onclick]')) {
        expect(impostor.getAttribute('role'), name).not.toBeNull();
      }
      unmount();
    }
  });

  it('ties the MSISDN hint and error to the field, whatever ids Base UI mints', () => {
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['collect_msisdn_invalid'] as CheckoutState,
      'en',
    );
    const input = container.querySelector('input#vpay-msisdn');
    expect(input?.getAttribute('aria-invalid')).toBe('true');
    // The ids are Base UI's now, so the assertion is on the PROPERTY rather
    // than on three strings this file used to keep in step with the markup:
    // whatever `aria-describedby` points at must resolve, and between them
    // the targets must carry both the hint and the error.
    const ids = (input?.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(Boolean);
    expect(ids.length).toBeGreaterThanOrEqual(2);
    const described = ids
      .map((id) => {
        const target = container.querySelector(`#${id}`);
        expect(target, `aria-describedby names #${id}, which is not in the document`).not.toBeNull();
        return target?.textContent ?? '';
      })
      .join(' ');
    expect(described).toContain(DICTIONARIES.en['msisdn.hint']);
    expect(described).toContain(DICTIONARIES.en['msisdn.invalid']);
    // And the error announces itself, for a payer who is already past the field.
    expect(screen.getByTestId('msisdn-problem').getAttribute('role')).toBe('alert');
    unmount();
  });
});

describe('the controls do what the screen says', () => {
  it('submits the typed number, unmodified, to the handler', () => {
    const onSubmitMsisdn = vi.fn();
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState,
      'fr',
      { onSubmitMsisdn },
    );
    const input = container.querySelector('input#vpay-msisdn') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '+237 6 71 23 45 67' } });
    fireEvent.submit(container.querySelector('form') as HTMLFormElement);
    expect(onSubmitMsisdn).toHaveBeenCalledWith('+237 6 71 23 45 67');
    unmount();
  });

  it('reports the rail a payer picked, by code', () => {
    const onChooseRail = vi.fn();
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['select_rail'] as CheckoutState,
      'en',
      { onChooseRail },
    );
    fireEvent.click(container.querySelector('[data-rail="orange_money"]') as HTMLElement);
    expect(onChooseRail).toHaveBeenCalledWith(
      expect.objectContaining({ code: 'orange_money', flow: 'redirect' }),
    );
    unmount();
  });

  it('names a rail it cannot drive rather than offering a button that would fail (D9)', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['select_rail'] as CheckoutState, 'en');
    const unsupported = screen.getByTestId('unsupported-rails');
    expect(within(unsupported).getByText(/zzz_pay/)).toBeTruthy();
    unmount();
  });

  it('offers one named button back to the merchant, and no timer', () => {
    const onReturnToMerchant = vi.fn();
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState,
      'en',
      { onReturnToMerchant },
    );
    // The button names where it goes. A generic "Continue" on an outcome
    // screen is the one place a payer cannot guess the destination.
    fireEvent.click(screen.getByRole('button', { name: 'Back to Boutique Test' }));
    expect(onReturnToMerchant).toHaveBeenCalledTimes(1);
    // Nothing counts down, in either dictionary, on either page.
    expect(container.textContent).not.toMatch(/\d+\s*s\./);
    expect(screen.queryByTestId('countdown')).toBeNull();
    unmount();
  });

  it('does NOT render a failed payment in the neutral tone a cancelled one beats', () => {
    // The defect this replaced: the screen took its colour from the operator's
    // `statusTone` through the intent status a failure leaves behind
    // (`requires_payment_method`, which is legitimately NEUTRAL on a
    // dashboard), so a payer whose payment FAILED read a grey box while a
    // payer who cancelled read a red one. Asserted on the rendered class,
    // because that is what a payer sees.
    const failed = renderState(CHECKOUT_SCREENS['outcome_failed'] as CheckoutState, 'en');
    const failedAlert = failed.container.querySelector('[data-outcome="failed"] .alert');
    expect(failedAlert?.className, 'a failure must carry a tone').toContain('alert-error');
    failed.unmount();

    const succeeded = renderState(CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState, 'en');
    expect(
      succeeded.container.querySelector('[data-outcome="succeeded"] .alert')?.className,
    ).toContain('alert-success');
    succeeded.unmount();

    // D4 (2026-09-07, docs/plans/2026-09-07-ui-revamp.md §9): a canceled
    // payment tones warning, not error — it is the payer's own action,
    // unlike a failure. `@vpay/tokens`' checkoutOutcomeTone changed; this
    // assertion moved with it rather than staying pinned to the tone D4
    // deliberately replaced.
    const canceled = renderState(CHECKOUT_SCREENS['outcome_canceled'] as CheckoutState, 'en');
    expect(
      canceled.container.querySelector('[data-outcome="canceled"] .alert')?.className,
    ).toContain('alert-warning');
    canceled.unmount();
  });

  it('says so plainly when the session names nowhere to return to', () => {
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState,
      'en',
      { destination: null },
    );
    expect(screen.getByTestId('no-destination')).toBeTruthy();
    expect(container.querySelector('[data-outcome] button')).toBeNull();
    unmount();
  });

  it('switches locale without navigating, so the fragment survives', () => {
    const onLocaleChange = vi.fn();
    const { unmount } = renderState(CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState, 'fr', {
      onLocaleChange,
    });
    fireEvent.change(screen.getByLabelText(DICTIONARIES.fr['locale.label']), {
      target: { value: 'en' },
    });
    expect(onLocaleChange).toHaveBeenCalledWith('en');
    // No anchor anywhere: a link to `?lang=en` would drop `location.hash`.
    expect(document.querySelectorAll('a[href]').length).toBe(0);
    unmount();
  });
});

describe('the return view', () => {
  for (const [name, state] of Object.entries(RETURN_SCREENS)) {
    for (const locale of LOCALES) {
      it(`renders ${name} in ${locale}`, () => {
        const { container, unmount } = renderReturn(state, locale);
        expect(container.querySelector('[data-screen]')?.textContent?.trim()).not.toBe('');
        unmount();
      });
    }
  }

  it('shows the failure the intent reported, in the payer’s language', () => {
    const { container, unmount } = renderReturn(RETURN_SCREENS['outcome_failed']!, 'fr', {
      destination: null,
    });
    expect(container.textContent).toContain(DICTIONARIES.fr['failure.payer_timeout']);
    unmount();
  });
});

describe('branding, from the deployment’s own branding.yaml', () => {
  it('shows the plain page title when nothing is configured', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState, 'en');
    expect(screen.getByTestId('brand-name').textContent).toBe(DICTIONARIES.en['page.title']);
    expect(screen.queryByTestId('brand-logo')).toBeNull();
    expect(screen.queryByTestId('support-contact')).toBeNull();
    unmount();
  });

  it('shows the operator’s name, logo and support contact when they are', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState, 'en', {
      branding: makeBranding({
        displayName: 'Vaam Payments',
        logoUrl: 'https://cdn.example/logo.svg',
        supportContact: 'support@vaam.example',
      }),
    });
    expect(screen.getByTestId('brand-name').textContent).toBe('Vaam Payments');
    const logo = screen.getByTestId('brand-logo');
    expect(logo.getAttribute('src')).toBe('https://cdn.example/logo.svg');
    // The image is the only place that name appears if the heading is
    // hidden, so it carries it.
    expect(logo.getAttribute('alt')).toBe('Vaam Payments');
    expect(screen.getByTestId('support-contact').textContent).toContain('support@vaam.example');
    unmount();
  });

  it('never lets the operator’s name stand in for the merchant’s', () => {
    // The one substitution that would be a lie: telling a payer they are
    // paying whoever runs the deployment.
    const state: CheckoutState = {
      name: 'collect_msisdn',
      context: makeContext({}, {}, null),
      rails: {
        supported: [{ code: 'mtn_momo', flow: 'mobile_money_push', label: 'rail.mtn_momo' }],
        unsupported: [],
      },
      rail: { code: 'mtn_momo', flow: 'mobile_money_push', label: 'rail.mtn_momo' },
      problem: null,
    };
    const { container, unmount } = renderState(state, 'en', {
      branding: makeBranding({ displayName: 'Vaam Payments' }),
    });
    expect(screen.getByTestId('pay-to').textContent).toBe(DICTIONARIES.en['page.pay_to_unnamed']);
    expect(screen.getByTestId('pay-to').textContent).not.toContain('Vaam Payments');
    // It is in the header, and only there: nothing outside `brand-name`
    // carries it.
    const brand = screen.getByTestId('brand-name');
    const elsewhere = [...container.querySelectorAll('*')].filter(
      (node) => node !== brand && !brand.contains(node) && !node.contains(brand),
    );
    for (const node of elsewhere) {
      expect(node.textContent ?? '').not.toContain('Vaam Payments');
    }
    unmount();
  });

  it('gives the logo a word even when there is no name to use', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState, 'fr', {
      branding: makeBranding({ logoUrl: 'https://cdn.example/logo.svg' }),
    });
    expect(screen.getByTestId('brand-logo').getAttribute('alt')).toBe(
      DICTIONARIES.fr['page.operator_logo_alt'],
    );
    unmount();
  });
});

describe('the rail’s own words on a failure', () => {
  it('shows them under the translated sentence, never instead of it', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['outcome_failed'] as CheckoutState, 'en');
    // The translated message is still the headline.
    expect(screen.getByTestId('outcome-body').textContent).toBe(
      DICTIONARIES.en['failure.insufficient_funds'],
    );
    // And the provider's own text is beside it, labelled as theirs.
    const detail = screen.getByTestId('provider-reason');
    expect(detail.textContent).toContain(DICTIONARIES.en['outcome.provider_said']);
    expect(detail.textContent).toContain('MTN-4001');
    unmount();
  });

  it('shows nothing at all where the API gave no message', () => {
    const state = structuredClone(CHECKOUT_SCREENS['outcome_failed']) as CheckoutState & {
      reason: string | null;
    };
    state.reason = null;
    const { unmount } = renderState(state, 'en');
    expect(screen.queryByTestId('provider-reason')).toBeNull();
    unmount();
  });

  it('renders it as text, so a rail cannot put markup on vpay’s page', () => {
    const state = structuredClone(CHECKOUT_SCREENS['outcome_failed']) as CheckoutState & {
      reason: string | null;
    };
    state.reason = '<img src=x onerror=alert(1)>';
    const { container, unmount } = renderState(state, 'en');
    expect(screen.getByTestId('provider-reason').textContent).toContain('<img');
    expect(container.querySelector('img')).toBeNull();
    unmount();
  });
});

describe('page memory, on the entry screens', () => {
  function mtnState(): CheckoutState {
    return CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState;
  }

  it('offers the opt-in with the box clear and the cost stated on the control', () => {
    const { container, unmount } = renderState(mtnState(), 'en');
    // `getByRole`, not `getByTestId`: Base UI forwards every prop to both the
    // `role="checkbox"` button and the `aria-hidden` input beside it, and the
    // one this assertion is about is the one in the accessibility tree.
    const box = screen.getByRole('checkbox');
    expect(container.querySelectorAll('[data-testid="remember"]').length).toBe(1);
    expect(box.getAttribute('aria-checked')).toBe('false');
    // The warning is the checkbox's own description, not a tooltip.
    const describedBy = box.getAttribute('aria-describedby') ?? '';
    expect(document.getElementById(describedBy)?.textContent).toBe(
      DICTIONARIES.en['memory.warning'],
    );
    unmount();
  });

  it('shows nothing at all when config.yaml turned the feature off', () => {
    const { container, unmount } = renderState(mtnState(), 'en', {
      memory: makeMemoryControls({ offered: false }),
    });
    expect(container.querySelectorAll('[data-testid="remember"]').length).toBe(0);
    expect(screen.queryByTestId('remember-warning')).toBeNull();
    expect(screen.queryByTestId('forget')).toBeNull();
    expect(container.textContent).not.toContain(DICTIONARIES.en['memory.warning']);
    unmount();
  });

  it('reports a tick and an untick to the client that owns the store', () => {
    const onRememberChange = vi.fn();
    const { unmount } = renderState(mtnState(), 'en', {
      memory: makeMemoryControls({ onRememberChange }),
    });
    fireEvent.click(screen.getByRole('checkbox'));
    expect(onRememberChange).toHaveBeenCalledWith(true, expect.anything());
    unmount();
  });

  it('toggles from the sentence and from the keyboard, not only from the box', () => {
    // The box is a 16-pixel target on a phone. Both of these are measured
    // rather than assumed: Base UI's checkbox is a `<span role="checkbox">`,
    // so neither the label association nor the space key is something the
    // platform gives for free.
    const onRememberChange = vi.fn();
    const { container, unmount } = renderState(mtnState(), 'en', {
      memory: makeMemoryControls({ onRememberChange }),
    });
    fireEvent.click(container.querySelector('#vpay-remember-label') as HTMLElement);
    expect(onRememberChange).toHaveBeenCalledTimes(1);
    const box = screen.getByRole('checkbox');
    fireEvent.keyDown(box, { key: ' ' });
    fireEvent.keyUp(box, { key: ' ' });
    expect(onRememberChange).toHaveBeenCalledTimes(2);
    unmount();
  });

  it('prefills the number this device remembered, and leaves it editable', () => {
    const { container, unmount } = renderState(mtnState(), 'en', {
      defaultMsisdn: '237671234567',
      memory: makeMemoryControls({ remember: true, hasRecord: true }),
    });
    const input = container.querySelector('input#vpay-msisdn') as HTMLInputElement;
    expect(input.value).toBe('237671234567');
    fireEvent.change(input, { target: { value: '237680000000' } });
    expect(input.value).toBe('237680000000');
    unmount();
  });

  it('offers the way out only when there is something to forget', () => {
    const first = renderState(mtnState(), 'en');
    expect(first.queryByTestId('forget')).toBeNull();
    first.unmount();

    const onForget = vi.fn();
    const { unmount } = renderState(mtnState(), 'en', {
      memory: makeMemoryControls({ hasRecord: true, onForget }),
    });
    fireEvent.click(screen.getByTestId('forget'));
    expect(onForget).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('says so once the device has been cleared, rather than leaving the payer guessing', () => {
    const { unmount } = renderState(mtnState(), 'en', {
      memory: makeMemoryControls({ forgotten: true }),
    });
    expect(screen.getByTestId('forgotten').textContent).toBe(DICTIONARIES.en['memory.forgotten']);
    unmount();
  });

  it('offers the same opt-in on the redirect screen, naming the rail rather than a number', () => {
    const { unmount } = renderState(CHECKOUT_SCREENS['ready_redirect'] as CheckoutState, 'en');
    const box = screen.getByRole('checkbox');
    const labelId = box.getAttribute('aria-labelledby') ?? '';
    // "Remember Orange Money on this device" — the rail, because a redirect
    // rail collects the number on its own page and this one never sees it.
    expect(document.getElementById(labelId)?.textContent).toBe(
      format(DICTIONARIES.en['memory.remember_method'], {
        rail: DICTIONARIES.en['rail.orange_money'],
      }),
    );
    unmount();
  });

  it('marks the rail this device last used without choosing it', () => {
    const onChooseRail = vi.fn();
    const { unmount } = renderState(CHECKOUT_SCREENS['select_rail'] as CheckoutState, 'en', {
      lastRail: 'orange_money',
      onChooseRail,
    });
    const marked = screen.getByTestId('last-used');
    expect(marked.closest('button')?.getAttribute('data-rail')).toBe('orange_money');
    // Marked, not pressed: a page that advanced itself past a screen the
    // payer has not read is the mistake the countdown was.
    expect(onChooseRail).not.toHaveBeenCalled();
    unmount();
  });

  it('leaves the MSISDN form’s only submit button the submit button', () => {
    // The Cypress specs click `button[type="submit"]`, and the memory
    // controls must not add a second one.
    const { container, unmount } = renderState(mtnState(), 'en', {
      memory: makeMemoryControls({ hasRecord: true }),
    });
    expect(container.querySelectorAll('button[type="submit"]').length).toBe(1);
    unmount();
  });
});
