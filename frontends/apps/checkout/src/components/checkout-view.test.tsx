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
import { SESSION_ID, makeContext, makePublicIntent, makeSession } from '../testing/fixtures';
import { CheckoutView, type CheckoutViewProps } from './checkout-view';
import { ReturnView } from './return-view';
import { CHECKOUT_SCREENS, RETURN_SCREENS } from '../testing/screen-states';

const NOOP = () => undefined;

function renderState(state: CheckoutState, locale: Locale, overrides: Partial<CheckoutViewProps> = {}) {
  const props: CheckoutViewProps = {
    state,
    t: translator(locale),
    locale,
    destination: 'https://shop.example/ok?sid=cs_test_fixture000000000001',
    secondsLeft: 5,
    onChooseRail: NOOP,
    onBack: NOOP,
    onSubmitMsisdn: NOOP,
    onStartRedirect: NOOP,
    onRetryPoll: NOOP,
    onContinue: NOOP,
    onLocaleChange: NOOP,
    ...overrides,
  };
  return render(<CheckoutView {...props} />);
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
    for (const testId of ['pay-to', 'outcome-body', 'countdown']) {
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
    expect(text(outcome.container, 'countdown')).toBe(
      format(DICTIONARIES.en['outcome.auto_forward_unnamed'], { seconds: 5 }),
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
    expect(text(container, 'countdown')).toContain('Boutique Test');
    unmount();
  });

  it('does the same on the return page', () => {
    const { container, unmount } = render(
      <ReturnView
        state={{
          name: 'outcome',
          context: {
            session: makeSession({ status: 'complete', payment_status: 'paid' }),
            intent: makePublicIntent({ status: 'succeeded' }),
            merchant: null,
          },
          kind: 'succeeded',
          failure: null,
        }}
        t={translator('fr')}
        locale="fr"
        destination="https://shop.example/done"
        secondsLeft={3}
        onContinue={NOOP}
        onLocaleChange={NOOP}
      />,
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

  it('gives every control an accessible name and a native, focusable element', () => {
    for (const [name, state] of Object.entries(CHECKOUT_SCREENS)) {
      const { container, unmount } = renderState(state, 'fr');
      const controls = container.querySelectorAll('button, input, select, a[href]');
      for (const control of controls) {
        expect(['BUTTON', 'INPUT', 'SELECT', 'A'], name).toContain(control.tagName);
        const labelled =
          control.getAttribute('aria-label') ??
          (control.id.length > 0
            ? container.querySelector(`label[for="${control.id}"]`)?.textContent
            : null) ??
          control.textContent;
        expect(labelled?.trim(), `${name}: ${control.outerHTML}`).not.toBe('');
      }
      // Nothing pretending to be a control.
      expect(container.querySelectorAll('div[onclick], span[role="button"]').length).toBe(0);
      unmount();
    }
  });

  it('ties the MSISDN error to the field with aria-describedby and aria-invalid', () => {
    const { container, unmount } = renderState(
      CHECKOUT_SCREENS['collect_msisdn_invalid'] as CheckoutState,
      'en',
    );
    const input = container.querySelector('input#vpay-msisdn');
    expect(input?.getAttribute('aria-invalid')).toBe('true');
    const described = input?.getAttribute('aria-describedby') ?? '';
    expect(described).toContain('vpay-msisdn-error');
    expect(container.querySelector('#vpay-msisdn-error')?.getAttribute('role')).toBe('alert');
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

  it('offers Continue plus a visible countdown when there is somewhere to go', () => {
    const onContinue = vi.fn();
    const { unmount } = renderState(
      CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState,
      'en',
      { onContinue },
    );
    expect(screen.getByTestId('countdown').textContent).toContain('5');
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(onContinue).toHaveBeenCalledTimes(1);
    unmount();
  });

  it('says so plainly when the session names nowhere to return to', () => {
    const { unmount } = renderState(
      CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState,
      'en',
      { destination: null, secondsLeft: null },
    );
    expect(screen.getByTestId('no-destination')).toBeTruthy();
    expect(screen.queryByTestId('countdown')).toBeNull();
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
        const { container, unmount } = render(
          <ReturnView
            state={state}
            t={translator(locale)}
            locale={locale}
            destination="https://shop.example/done"
            secondsLeft={3}
            onContinue={NOOP}
            onLocaleChange={NOOP}
          />,
        );
        expect(container.querySelector('[data-screen]')?.textContent?.trim()).not.toBe('');
        unmount();
      });
    }
  }

  it('shows the failure the intent reported, in the payer’s language', () => {
    const { container, unmount } = render(
      <ReturnView
        state={RETURN_SCREENS['outcome_failed']!}
        t={translator('fr')}
        locale="fr"
        destination={null}
        secondsLeft={null}
        onContinue={NOOP}
        onLocaleChange={NOOP}
      />,
    );
    expect(container.textContent).toContain(DICTIONARIES.fr['failure.payer_timeout']);
    unmount();
  });
});
