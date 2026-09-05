/**
 * `CheckoutState` → a screen. One `switch`, no side effects.
 *
 * Kept separate from the client component that owns the controller so that
 * every screen state can be rendered in a test and in Storybook from a
 * literal state value — including the ones that are hard to reach through
 * the network (an expired session, a refused embed, a rail the page cannot
 * drive). A screenshot of a state nobody can produce is how a page ends up
 * with a branch that has never rendered.
 */
import type { Locale, MessageKey, Translate } from '../i18n/index';
import { failureMessage } from '../lib/failures';
import type { CheckoutState } from '../lib/machine';
import { formatAmount } from '../lib/money';
import type { SupportedRail } from '../lib/rails';
import { LocaleSwitch } from './locale-switch';
import {
  MsisdnForm,
  NoticePanel,
  OutcomePanel,
  PaymentSummary,
  RailSelector,
  RedirectPrompt,
  StatusPanel,
  merchantLine,
} from './screens';

export interface CheckoutViewHandlers {
  onChooseRail: (rail: SupportedRail) => void;
  onBack: () => void;
  onSubmitMsisdn: (msisdn: string) => void;
  onStartRedirect: () => void;
  onRetryPoll: () => void;
  onContinue: () => void;
  onLocaleChange: (locale: Locale) => void;
}

export interface CheckoutViewProps extends CheckoutViewHandlers {
  state: CheckoutState;
  t: Translate;
  locale: Locale;
  /** Where Continue would send the payer, or `null`. Computed by the caller from the session. */
  destination: string | null;
  /** Seconds left on the auto-forward, or `null` when there is nothing to count down to. */
  secondsLeft: number | null;
}

/** The session-bearing states, so the summary is rendered once rather than per screen. */
function contextOf(state: CheckoutState) {
  return 'context' in state ? state.context : null;
}

export function CheckoutView(props: CheckoutViewProps) {
  const { state, t, locale } = props;
  const context = contextOf(state);
  const amount =
    context === null ? '' : formatAmount(context.intent.amount, context.intent.currency, locale);
  // `null`, not `''`: the screens choose a neutrally-worded sentence for a
  // session whose read carried no merchant name, rather than rendering one
  // written for a name with the name missing.
  const merchant = context?.merchant?.name ?? null;

  return (
    <main className="mx-auto flex w-full max-w-md flex-col gap-6 p-6">
      <header className="flex items-center justify-between gap-4">
        <h1 className="text-lg font-semibold">{t('page.title')}</h1>
        <LocaleSwitch t={t} locale={locale} onChange={props.onLocaleChange} />
      </header>

      {context === null ? null : (
        <PaymentSummary
          t={t}
          merchant={merchant}
          amount={amount}
          reference={context.session.id}
          livemode={context.session.livemode}
        />
      )}

      {/*
        Present from first render and never unmounted, so a status written
        into it is announced. A live region created together with its own
        text is not.
      */}
      <div aria-live="polite" aria-atomic="true" data-testid="live-region">
        {renderScreen(props, amount, merchant)}
      </div>
    </main>
  );
}

function renderScreen(
  props: CheckoutViewProps,
  amount: string,
  merchant: string | null,
): React.ReactNode {
  const { state, t } = props;
  switch (state.name) {
    case 'loading':
      return <StatusPanel t={t} screen="loading" title={t('state.loading')} body={null} />;

    case 'error':
      return (
        <NoticePanel
          t={t}
          screen="error"
          title={t('error.title')}
          body={t(state.error.code)}
          code={state.error.serverCode}
        />
      );

    case 'refused':
      return state.reason === 'embed_not_allowed' ? (
        <NoticePanel
          t={t}
          screen="refused_embed"
          title={t('refusal.embed_title')}
          body={t('refusal.embed_body')}
        />
      ) : (
        <NoticePanel
          t={t}
          screen="refused_rail"
          title={t('error.title')}
          body={t('rail.none')}
        />
      );

    case 'expired':
      return (
        <NoticePanel
          t={t}
          screen="expired"
          title={t('expired.title')}
          body={merchantLine(t, merchant, 'expired.body', 'expired.body_unnamed')}
        />
      );

    case 'select_rail':
      return <RailSelector t={t} rails={state.rails} onChoose={props.onChooseRail} />;

    case 'collect_msisdn':
      return (
        <MsisdnForm
          t={t}
          amount={amount}
          rail={state.rail}
          problem={state.problem}
          canGoBack={state.rails.supported.length > 1}
          onSubmit={props.onSubmitMsisdn}
          onBack={props.onBack}
        />
      );

    case 'ready_redirect':
      return (
        <RedirectPrompt
          t={t}
          amount={amount}
          rail={state.rail}
          problem={state.problem}
          canGoBack={state.rails.supported.length > 1}
          onContinue={props.onStartRedirect}
          onBack={props.onBack}
        />
      );

    case 'confirming':
      return <StatusPanel t={t} screen="confirming" title={t('state.confirming')} body={null} />;

    case 'waiting':
      return (
        <StatusPanel
          t={t}
          screen="waiting"
          title={t('state.waiting_title')}
          body={t('state.waiting_body', { amount })}
          notice={state.notice}
          onRetry={state.notice === null ? undefined : props.onRetryPoll}
        />
      );

    case 'redirecting':
      return (
        <StatusPanel
          t={t}
          screen="redirecting"
          title={t('state.redirecting_title')}
          body={t('state.redirecting_body')}
        />
      );

    case 'outcome':
      return (
        <OutcomePanel
          t={t}
          kind={state.kind}
          failure={failureMessage(state.failure)}
          merchant={merchant}
          amount={amount}
          destination={props.destination}
          secondsLeft={props.secondsLeft}
          onContinue={props.onContinue}
        />
      );

    case 'forwarding':
      return (
        <StatusPanel
          t={t}
          screen="forwarding"
          title={t('outcome.continue')}
          body={merchantLine(t, merchant, 'outcome.auto_forward', 'outcome.auto_forward_unnamed', {
            seconds: 0,
          })}
        />
      );

    default: {
      const unreachable: never = state;
      return unreachable;
    }
  }
}

/** Re-exported so tests can name a key without importing the dictionary. */
export type { MessageKey };
