/**
 * Every checkout screen, in both languages, with the a11y addon on.
 *
 * The states come from `../testing/screen-states.ts` — the same literals
 * `checkout-view.test.tsx` asserts against — so a screen a designer reviews
 * here is a screen a test covers, and a screen added to one without the
 * other fails the "covers each state the machine can be in" assertion.
 *
 * Picked up by `frontends/packages/ui/.storybook/main.ts`, which is where
 * the a11y addon and the two daisyUI themes are already configured; CI's
 * `web` job builds that Storybook.
 */
import type { Meta, StoryObj } from '@storybook/react';

import { translator, type Locale } from '../i18n/index';
import type { CheckoutState } from '../lib/machine';
import { CheckoutView } from './checkout-view';
import { ReturnView } from './return-view';
import { CHECKOUT_SCREENS, RETURN_SCREENS } from '../testing/screen-states';

const NOOP = () => undefined;

function Screen({ state, locale }: { state: CheckoutState; locale: Locale }) {
  return (
    <CheckoutView
      state={state}
      t={translator(locale)}
      locale={locale}
      destination="https://shop.example/ok?sid=cs_test_fixture000000000001"
      secondsLeft={5}
      onChooseRail={NOOP}
      onBack={NOOP}
      onSubmitMsisdn={NOOP}
      onStartRedirect={NOOP}
      onRetryPoll={NOOP}
      onContinue={NOOP}
      onLocaleChange={NOOP}
    />
  );
}

const meta = {
  title: 'Checkout/Screens',
  component: Screen,
  parameters: {
    layout: 'centered',
    a11y: { config: { rules: [{ id: 'color-contrast', enabled: true }] } },
  },
  argTypes: {
    locale: { control: 'inline-radio', options: ['fr', 'en'] },
  },
  args: { locale: 'fr' },
  tags: ['autodocs'],
} satisfies Meta<typeof Screen>;

export default meta;
type Story = StoryObj<typeof meta>;

function story(name: keyof typeof CHECKOUT_SCREENS): Story {
  return { args: { state: CHECKOUT_SCREENS[name] as CheckoutState, locale: 'fr' } };
}

export const Loading: Story = story('loading');
export const ChooseRail: Story = story('select_rail');
export const MtnNumber: Story = story('collect_msisdn');
export const MtnNumberRejected: Story = story('collect_msisdn_invalid');
export const OrangeReady: Story = story('ready_redirect');
export const Confirming: Story = story('confirming');
export const WaitingForThePayer: Story = story('waiting');
export const WaitingWithAFailedPoll: Story = story('waiting_notice');
export const RedirectingToTheRail: Story = story('redirecting');
export const Succeeded: Story = story('outcome_succeeded');
export const Failed: Story = story('outcome_failed');
export const Canceled: Story = story('outcome_canceled');
export const Forwarding: Story = story('forwarding');
export const Expired: Story = story('expired');
export const EmbeddingRefused: Story = story('refused_embed');
export const NoRailThisPageCanDrive: Story = story('refused_rail');
export const LinkNotValid: Story = story('error');

/** English, so the a11y addon runs over both dictionaries' string lengths. */
export const MtnNumberInEnglish: Story = {
  args: { state: CHECKOUT_SCREENS['collect_msisdn'] as CheckoutState, locale: 'en' },
};

export const SucceededInEnglish: Story = {
  args: { state: CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState, locale: 'en' },
};

/** The return page, which has no form and cannot confirm anything. */
export const ReturnPolling: Story = {
  args: { state: CHECKOUT_SCREENS['waiting'] as CheckoutState, locale: 'fr' },
  render: () => (
    <ReturnView
      state={RETURN_SCREENS['polling']!}
      t={translator('fr')}
      locale="fr"
      destination={null}
      secondsLeft={null}
      onContinue={NOOP}
      onLocaleChange={NOOP}
    />
  ),
};

export const ReturnSucceeded: Story = {
  args: { state: CHECKOUT_SCREENS['outcome_succeeded'] as CheckoutState, locale: 'fr' },
  render: () => (
    <ReturnView
      state={RETURN_SCREENS['outcome_succeeded']!}
      t={translator('fr')}
      locale="fr"
      destination="https://shop.example/done"
      secondsLeft={5}
      onContinue={NOOP}
      onLocaleChange={NOOP}
    />
  ),
};

export const ReturnFailed: Story = {
  args: { state: CHECKOUT_SCREENS['outcome_failed'] as CheckoutState, locale: 'fr' },
  render: () => (
    <ReturnView
      state={RETURN_SCREENS['outcome_failed']!}
      t={translator('fr')}
      locale="fr"
      destination="https://shop.example/done"
      secondsLeft={5}
      onContinue={NOOP}
      onLocaleChange={NOOP}
    />
  ),
};
