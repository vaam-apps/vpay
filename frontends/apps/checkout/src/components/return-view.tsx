/**
 * `ReturnState` → a screen.
 *
 * The return page has no form and no rail selector: the payer has already
 * been to the rail. What it can show is "still waiting for the rail's
 * answer", an outcome, an expired session, or a read it could not make.
 */
import { PageShell, Stack } from '@vpay/ui';

import type { Branding } from '../config/settings';
import type { Locale, Translate } from '../i18n/index';
import { failureMessage } from '../lib/failures';
import { formatAmount } from '../lib/money';
import type { ReturnState } from '../lib/return';
import { LocaleSwitch } from './locale-switch';
import {
  BrandHeader,
  NoticePanel,
  OutcomePanel,
  PaymentSummary,
  StatusPanel,
  SupportLine,
  merchantLine,
} from './screens';

export interface ReturnViewProps {
  state: ReturnState;
  t: Translate;
  locale: Locale;
  branding: Branding;
  destination: string | null;
  /** The outcome screen's one control. No timer runs beside it on this page either. */
  onReturnToMerchant: () => void;
  onLocaleChange: (locale: Locale) => void;
}

export function ReturnView(props: ReturnViewProps) {
  const { state, t, locale } = props;
  const context = 'context' in state ? state.context : null;
  const amount =
    context === null ? '' : formatAmount(context.intent.amount, context.intent.currency, locale);
  const merchant = context?.merchant?.name ?? null;

  return (
    <main>
      <PageShell>
        <Stack as="header" justify="between" gap="md">
          <BrandHeader t={t} branding={props.branding} />
          <LocaleSwitch t={t} locale={locale} onChange={props.onLocaleChange} />
        </Stack>

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
                    reason={state.reason}
                    merchant={merchant}
                    amount={amount}
                    destination={props.destination}
                    onBack={props.onReturnToMerchant}
                  />
                );
              case 'forwarding':
                return (
                  <StatusPanel
                    t={t}
                    screen="forwarding"
                    title={t('state.forwarding_title')}
                    body={merchantLine(
                      t,
                      merchant,
                      'state.forwarding_body',
                      'state.forwarding_body_unnamed',
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

        <SupportLine t={t} branding={props.branding} />
      </PageShell>
    </main>
  );
}
