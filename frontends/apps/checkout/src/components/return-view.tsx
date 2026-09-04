/**
 * `ReturnState` → a screen.
 *
 * The return page has no form and no rail selector: the payer has already
 * been to the rail. What it can show is "still waiting for the rail's
 * answer", an outcome, an expired session, or a read it could not make.
 */
import type { Locale, Translate } from '../i18n/index';
import { failureMessage } from '../lib/failures';
import { formatAmount } from '../lib/money';
import type { ReturnState } from '../lib/return';
import { LocaleSwitch } from './locale-switch';
import { NoticePanel, OutcomePanel, PaymentSummary, StatusPanel, merchantLine } from './screens';

export interface ReturnViewProps {
  state: ReturnState;
  t: Translate;
  locale: Locale;
  destination: string | null;
  secondsLeft: number | null;
  onContinue: () => void;
  onLocaleChange: (locale: Locale) => void;
}

export function ReturnView(props: ReturnViewProps) {
  const { state, t, locale } = props;
  const context = 'context' in state ? state.context : null;
  const amount =
    context === null ? '' : formatAmount(context.intent.amount, context.intent.currency, locale);
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

      <div aria-live="polite" aria-atomic="true" data-testid="live-region">
        {(() => {
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
            case 'expired':
              return (
                <NoticePanel
                  t={t}
                  screen="expired"
                  title={t('expired.title')}
                  body={merchantLine(t, merchant, 'expired.body', 'expired.body_unnamed')}
                />
              );
            case 'polling':
              return (
                <StatusPanel
                  t={t}
                  screen="polling"
                  title={t('state.waiting_title')}
                  body={t('state.waiting_body', { amount })}
                  notice={state.notice}
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
                  body={merchantLine(
                    t,
                    merchant,
                    'outcome.auto_forward',
                    'outcome.auto_forward_unnamed',
                    { seconds: 0 },
                  )}
                />
              );
            default: {
              const unreachable: never = state;
              return unreachable;
            }
          }
        })()}
      </div>
    </main>
  );
}
