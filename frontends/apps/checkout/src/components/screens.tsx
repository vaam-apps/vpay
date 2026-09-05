/**
 * Every screen this page can show, as pure React.
 *
 * No `next/*` import, no `fetch`, no `window`: these components receive a
 * state and a translator and render. That is what lets the same functions
 * be a Storybook story, a vitest assertion in both locales, and the page
 * itself — rather than three descriptions of one design that drift.
 *
 * Accessibility is structural here, not decorative:
 *
 * - every screen's heading carries `tabIndex={-1}` and is focused when the
 *   screen changes, so a payer on a screen reader or a keyboard is moved to
 *   the new content instead of being left on a button that no longer exists;
 * - the status text sits in an `aria-live="polite"` region that is present
 *   from first render, because a live region added to the DOM at the same
 *   moment as its text is not announced;
 * - every control is a native `button`, `input` or `input[type=radio]` with
 *   a real label, which is what makes the keyboard-only path work without a
 *   single `onKeyDown`.
 */
import { useEffect, useRef } from 'react';
import { statusTone, type PaymentStatus } from '@vpay/tokens';

import type { MessageKey, Translate } from '../i18n/index';
import type { OutcomeKind } from '../lib/machine';
import type { RailChoices, SupportedRail } from '../lib/rails';

/**
 * The heading every screen starts with.
 *
 * Focus moves here whenever `screen` changes. `preventScroll` is not used:
 * a payer whose viewport does not currently show the heading should be
 * scrolled to it.
 */
export function ScreenHeading({
  screen,
  children,
  className,
}: {
  screen: string;
  children: React.ReactNode;
  className?: string;
}) {
  const ref = useRef<HTMLHeadingElement | null>(null);
  useEffect(() => {
    ref.current?.focus();
  }, [screen]);
  return (
    <h2
      ref={ref}
      tabIndex={-1}
      data-screen={screen}
      className={className ?? 'text-xl font-semibold outline-none'}
    >
      {children}
    </h2>
  );
}

/**
 * The one place a merchant name is turned into a sentence.
 *
 * `named` is the string with `{merchant}` in it; `unnamed` is the sentence
 * to show when the read carried no name. Two dictionary keys rather than a
 * stand-in value for `{merchant}`, because a placeholder ("—", "the
 * merchant", the session id) rendered inside a sentence written for a real
 * name reads like data the page has. It does not have it.
 */
export function merchantLine(
  t: Translate,
  merchant: string | null,
  named: MessageKey,
  unnamed: MessageKey,
  values: Record<string, string | number> = {},
): string {
  return merchant === null ? t(unnamed, values) : t(named, { ...values, merchant });
}

/**
 * The amount and the merchant, shown on every screen that has a session.
 *
 * `merchant` is `null` when the browser read carried no usable name; the
 * heading is then the neutral one, and no identifier stands in for it.
 */
export function PaymentSummary({
  t,
  merchant,
  amount,
  reference,
  livemode,
}: {
  t: Translate;
  merchant: string | null;
  amount: string;
  reference: string;
  livemode: boolean;
}) {
  return (
    <div className="rounded-box bg-base-200 p-4">
      {!livemode ? (
        <p className="mb-2 text-xs font-medium uppercase tracking-wide" data-testid="testmode">
          {t('page.testmode')}
        </p>
      ) : null}
      <p className="text-sm opacity-70" data-testid="pay-to">
        {merchantLine(t, merchant, 'page.pay_to', 'page.pay_to_unnamed')}
      </p>
      <p className="mt-1 text-3xl font-semibold tabular-nums" data-testid="amount">
        <span className="sr-only">{t('page.amount_label')}: </span>
        {amount}
      </p>
      <p className="mt-2 break-all text-xs opacity-60" data-testid="reference">
        {t('page.reference_label')}: {reference}
      </p>
    </div>
  );
}

/** D9: rails the intent offers that this page has no flow for. */
export function UnsupportedRails({ t, codes }: { t: Translate; codes: readonly string[] }) {
  if (codes.length === 0) {
    return null;
  }
  return (
    <ul className="mt-3 space-y-1" data-testid="unsupported-rails">
      {codes.map((code) => (
        <li key={code} className="text-sm opacity-70">
          {t('rail.unsupported', { rail: code })}
        </li>
      ))}
    </ul>
  );
}

/** The rail selector. Shown only when the intent offers more than one rail this page can drive. */
export function RailSelector({
  t,
  rails,
  onChoose,
}: {
  t: Translate;
  rails: RailChoices;
  onChoose: (rail: SupportedRail) => void;
}) {
  return (
    <section>
      <ScreenHeading screen="select_rail">{t('rail.legend')}</ScreenHeading>
      <div className="mt-4 flex flex-col gap-2">
        {rails.supported.map((rail) => (
          <button
            key={rail.code}
            type="button"
            className="btn btn-outline justify-start"
            data-rail={rail.code}
            onClick={() => onChoose(rail)}
          >
            {t(rail.label)}
          </button>
        ))}
      </div>
      <UnsupportedRails t={t} codes={rails.unsupported} />
    </section>
  );
}

/** The MTN path: one labelled field, one submit, one error message tied to the field. */
export function MsisdnForm({
  t,
  amount,
  rail,
  problem,
  canGoBack,
  onSubmit,
  onBack,
}: {
  t: Translate;
  amount: string;
  rail: SupportedRail;
  problem: MessageKey | null;
  canGoBack: boolean;
  onSubmit: (msisdn: string) => void;
  onBack: () => void;
}) {
  const inputId = 'vpay-msisdn';
  const hintId = 'vpay-msisdn-hint';
  const errorId = 'vpay-msisdn-error';
  return (
    <section>
      <ScreenHeading screen="collect_msisdn">{t(rail.label)}</ScreenHeading>
      <form
        className="mt-4 flex flex-col gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          // `FormData.get` is `string | File | null`. A non-string entry is
          // not something this form can produce, but stringifying one would
          // hand the MSISDN validator the text "[object File]" rather than
          // an empty field.
          const raw = data.get('msisdn');
          onSubmit(typeof raw === 'string' ? raw : '');
        }}
      >
        <label className="font-medium" htmlFor={inputId}>
          {t('msisdn.label')}
        </label>
        <input
          id={inputId}
          name="msisdn"
          type="tel"
          inputMode="tel"
          autoComplete="tel"
          className="input input-bordered w-full"
          aria-describedby={problem === null ? hintId : `${hintId} ${errorId}`}
          aria-invalid={problem === null ? undefined : true}
        />
        <p id={hintId} className="text-sm opacity-70">
          {t('msisdn.hint')}
        </p>
        {problem === null ? null : (
          <p id={errorId} role="alert" className="text-sm font-medium" data-testid="msisdn-problem">
            {t(problem)}
          </p>
        )}
        <button type="submit" className="btn btn-primary">
          {t('msisdn.submit', { amount })}
        </button>
        {canGoBack ? (
          <button type="button" className="btn btn-ghost btn-sm" onClick={onBack}>
            {t('msisdn.back')}
          </button>
        ) : null}
      </form>
    </section>
  );
}

/** The Orange path before the payer leaves: one button, nothing to fill in. */
export function RedirectPrompt({
  t,
  amount,
  rail,
  problem,
  canGoBack,
  onContinue,
  onBack,
}: {
  t: Translate;
  amount: string;
  rail: SupportedRail;
  problem: MessageKey | null;
  canGoBack: boolean;
  onContinue: () => void;
  onBack: () => void;
}) {
  return (
    <section>
      <ScreenHeading screen="ready_redirect">{t(rail.label)}</ScreenHeading>
      <p className="mt-3 opacity-80">{t('state.redirecting_body')}</p>
      {problem === null ? null : (
        <p role="alert" className="mt-3 text-sm font-medium" data-testid="redirect-problem">
          {t(problem)}
        </p>
      )}
      <button type="button" className="btn btn-primary mt-4 w-full" onClick={onContinue}>
        {t('msisdn.submit', { amount })}
      </button>
      {canGoBack ? (
        <button type="button" className="btn btn-ghost btn-sm mt-2" onClick={onBack}>
          {t('msisdn.back')}
        </button>
      ) : null}
    </section>
  );
}

/** Confirming, waiting for the payer, or on the way to a rail's page. */
export function StatusPanel({
  t,
  screen,
  title,
  body,
  notice,
  onRetry,
}: {
  t: Translate;
  screen: string;
  title: string;
  body: string | null;
  notice?: MessageKey | null;
  onRetry?: (() => void) | undefined;
}) {
  return (
    <section>
      <ScreenHeading screen={screen}>{title}</ScreenHeading>
      {body === null ? null : <p className="mt-3 opacity-80">{body}</p>}
      <span className="loading loading-dots loading-md mt-4" aria-hidden="true" />
      {notice ? (
        <div className="mt-4">
          <p className="text-sm font-medium" data-testid="poll-notice">
            {t(notice)}
          </p>
          {onRetry === undefined ? null : (
            <button type="button" className="btn btn-outline btn-sm mt-2" onClick={onRetry}>
              {t('error.retry')}
            </button>
          )}
        </div>
      ) : null}
    </section>
  );
}

/**
 * Outcome tone comes from `@vpay/tokens`, never from a colour written here
 * (AGENTS.md). The mapping is the honest one: a failed payment leaves the
 * intent at `requires_payment_method`, which is the status that tone
 * belongs to.
 */
const OUTCOME_STATUS: Record<OutcomeKind, PaymentStatus> = {
  succeeded: 'succeeded',
  canceled: 'canceled',
  failed: 'requires_payment_method',
};

const TONE_CLASS: Record<string, string> = {
  success: 'alert-success',
  error: 'alert-error',
  warning: 'alert-warning',
  info: 'alert-info',
  neutral: '',
};

export function OutcomePanel({
  t,
  kind,
  failure,
  merchant,
  amount,
  destination,
  secondsLeft,
  onContinue,
}: {
  t: Translate;
  kind: OutcomeKind;
  /** A `failure.*` key, already resolved from the intent's `last_payment_error`. */
  failure: MessageKey | null;
  /** `null` where the read carried no usable merchant name. */
  merchant: string | null;
  amount: string;
  /** Where Continue sends the payer, or `null` when the session names nowhere. */
  destination: string | null;
  secondsLeft: number | null;
  onContinue: () => void;
}) {
  const title =
    kind === 'succeeded'
      ? t('outcome.succeeded_title')
      : kind === 'canceled'
        ? t('outcome.canceled_title')
        : t('outcome.failed_title');
  const body =
    kind === 'succeeded'
      ? merchantLine(t, merchant, 'outcome.succeeded_body', 'outcome.succeeded_body_unnamed', {
          amount,
        })
      : kind === 'canceled'
        ? t('outcome.canceled_body')
        : t(failure ?? 'failure.unknown');
  const tone = TONE_CLASS[statusTone[OUTCOME_STATUS[kind]]] ?? '';
  return (
    <section data-outcome={kind}>
      <ScreenHeading screen="outcome">{title}</ScreenHeading>
      <div className={`alert mt-4 ${tone}`.trim()} role="status">
        <span data-testid="outcome-body">{body}</span>
      </div>
      {destination === null ? (
        <p className="mt-4 opacity-80" data-testid="no-destination">
          {t('outcome.no_destination')}
        </p>
      ) : (
        <>
          <button type="button" className="btn btn-primary mt-4 w-full" onClick={onContinue}>
            {t('outcome.continue')}
          </button>
          {secondsLeft === null ? null : (
            <p className="mt-2 text-sm opacity-70" data-testid="countdown">
              {merchantLine(t, merchant, 'outcome.auto_forward', 'outcome.auto_forward_unnamed', {
                seconds: secondsLeft,
              })}
            </p>
          )}
        </>
      )}
    </section>
  );
}

/** Expired, refused, or a read this page could not make. One shape for all three. */
export function NoticePanel({
  t,
  screen,
  title,
  body,
  code,
}: {
  t: Translate;
  screen: string;
  title: string;
  body: string;
  /** The server's own error code, for support. Rendered as data, never as prose. */
  code?: string | undefined;
}) {
  return (
    <section data-error-code={code}>
      <ScreenHeading screen={screen}>{title}</ScreenHeading>
      <div className="alert alert-warning mt-4" role="alert">
        <span data-testid="notice-body">{body}</span>
      </div>
      <span className="sr-only">{t('error.title')}</span>
    </section>
  );
}
