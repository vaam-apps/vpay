/**
 * Every screen this page can show, as pure React.
 *
 * No `next/*` import, no `fetch`, no `window`: these components receive a
 * state and a translator and render. That is what lets the same functions
 * be a Storybook story, a vitest assertion in both locales, and the page
 * itself — rather than three descriptions of one design that drift.
 *
 * **Styling is daisyUI's `bumblebee` theme and Base UI's component
 * defaults, and nothing else** (the maintainer's requirement, 2026-09-05).
 * There is no bespoke CSS in this file and none in `globals.css` beyond one
 * `prefers-reduced-motion` block: a class here is either a daisyUI component
 * class (`card`, `btn`, `alert`, `input`, `checkbox`, `badge`) or a Tailwind
 * utility. Colour is never written down — it comes from the theme's own
 * variables, which `src/config/theme.ts` lets an operator retint at runtime,
 * and status tone still comes from `@vpay/tokens` (AGENTS.md).
 *
 * Accessibility is structural here, not decorative:
 *
 * - every screen's heading carries `tabIndex={-1}` and is focused when the
 *   screen changes, so a payer on a screen reader or a keyboard is moved to
 *   the new content instead of being left on a button that no longer exists;
 * - the status text sits in an `aria-live="polite"` region that is present
 *   from first render, because a live region added to the DOM at the same
 *   moment as its text is not announced;
 * - the MSISDN field is a Base UI `Field`, which is what now ties the label,
 *   the hint and the error to the control — the wiring that used to be three
 *   hand-maintained `aria-describedby` ids;
 * - every control has a real accessible name and is in the tab order.
 *
 * That last point used to read "every control is a **native** `button`,
 * `input` or `input[type=radio]`", and it is no longer true: Base UI's
 * checkbox renders a `<span role="checkbox" tabindex="0">` with a visually
 * hidden native input beside it for the form value. The property the old
 * sentence was defending — a keyboard-only payer can reach and operate
 * everything — is still held and still tested, but it is now held by an ARIA
 * role and a `tabindex` rather than by the platform, so the test asserts the
 * role, the tab index, the accessible name and `aria-checked` explicitly.
 * Both the label click and the space key were **measured**, not assumed.
 */
import { Checkbox } from '@base-ui-components/react/checkbox';
import { Field } from '@base-ui-components/react/field';
import { useEffect, useRef } from 'react';
import { statusTone, type PaymentStatus } from '@vpay/tokens';

import type { MessageKey, Translate } from '../i18n/index';
import type { Branding } from '../config/settings';
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
 * The operator's mark, from `branding.yaml`.
 *
 * **Whose name this is matters.** It is the operator's — whoever runs this
 * deployment of vpay — and never the merchant's, which comes from the
 * session and is painted into "Pay {merchant}" by {@link PaymentSummary}. A
 * deployment that configured neither gets the plain page title, which is
 * what shipped before this file knew about branding at all.
 *
 * The logo's `alt` is the operator's name where there is one, because the
 * image is then the only place that name appears; where there is none the
 * image is captioned by nothing else, so it still needs a word.
 */
export function BrandHeader({ t, branding }: { t: Translate; branding: Branding }) {
  return (
    <div className="flex items-center gap-3">
      {branding.logoUrl === null ? null : (
        // eslint-disable-next-line @next/next/no-img-element -- `next/image` optimises through a route this app does not serve (no image optimisation in `output: 'standalone'` without a loader), and the URL is an operator's own absolute one.
        <img
          src={branding.logoUrl}
          alt={branding.displayName ?? t('page.operator_logo_alt')}
          className="h-8 w-auto"
          data-testid="brand-logo"
        />
      )}
      <h1 className="text-lg font-semibold" data-testid="brand-name">
        {branding.displayName ?? t('page.title')}
      </h1>
    </div>
  );
}

/** `branding.yaml`'s `support_contact`, as text. Never a `mailto:` or a `tel:` this page composed. */
export function SupportLine({ t, branding }: { t: Translate; branding: Branding }) {
  if (branding.supportContact === null) {
    return null;
  }
  return (
    <p className="text-xs opacity-60" data-testid="support-contact">
      {t('page.support', { contact: branding.supportContact })}
    </p>
  );
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
    <div className="card bg-base-200">
      <div className="card-body gap-1 p-4">
        {/*
          A paragraph, not a daisyUI `badge`. It was a badge for one
          revision and the screenshot showed why that was wrong: `.badge` is
          `whitespace-nowrap` by design, and this string is a sentence —
          "Test mode — no money moves on this deployment." spilled straight
          out of the card. A badge is for a word.
        */}
        {!livemode ? (
          <p className="text-xs font-semibold uppercase tracking-wide" data-testid="testmode">
            {t('page.testmode')}
          </p>
        ) : null}
        <p className="text-sm opacity-70" data-testid="pay-to">
          {merchantLine(t, merchant, 'page.pay_to', 'page.pay_to_unnamed')}
        </p>
        <p className="text-3xl font-semibold tabular-nums" data-testid="amount">
          <span className="sr-only">{t('page.amount_label')}: </span>
          {amount}
        </p>
        <p className="break-all text-xs opacity-60" data-testid="reference">
          {t('page.reference_label')}: {reference}
        </p>
      </div>
    </div>
  );
}

/** D9: rails the intent offers that this page has no flow for, or the operator's `config.yaml` excludes. */
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

/**
 * The rail selector. Shown only when the intent offers more than one rail
 * this page can drive.
 *
 * Buttons rather than a radio group, unchanged: choosing a rail *is* the
 * navigation, so a control that needed a second press to act on would put a
 * step between a payer and their payment. `lastRail` marks the one this
 * device last paid with — a hint, never a preselection: auto-advancing past
 * a screen a payer has not read is the same mistake as the countdown this
 * page just removed.
 */
export function RailSelector({
  t,
  rails,
  lastRail,
  onChoose,
}: {
  t: Translate;
  rails: RailChoices;
  lastRail: string | null;
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
            {rail.code === lastRail ? (
              <span className="badge badge-ghost badge-sm" data-testid="last-used">
                {t('memory.last_used')}
              </span>
            ) : null}
          </button>
        ))}
      </div>
      <UnsupportedRails t={t} codes={rails.unsupported} />
    </section>
  );
}

/** What the entry screens need to offer, read and clear this device's memory. */
export interface MemoryControls {
  /** `false` when `config.yaml` says `page_memory: false`. Nothing below renders. */
  offered: boolean;
  /** Whether the box is ticked. Owned by the client component. */
  remember: boolean;
  onRememberChange: (remember: boolean) => void;
  /** Whether this device is holding anything, so "forget" is offered only when there is something to forget. */
  hasRecord: boolean;
  onForget: () => void;
  /** Set once a forget has happened, so the payer is told rather than left guessing. */
  forgotten: boolean;
}

/**
 * The opt-in, its warning, and the way out.
 *
 * The warning is **on the control**, not in a tooltip or a help link. What
 * ticking this box costs is paid by whoever picks the handset up next, and
 * a payer in a phone shop deciding in four seconds is exactly the payer a
 * disclosure behind an icon fails.
 *
 * `label` differs between the two entry screens because what is remembered
 * does: a number on the MTN form, a choice of rail on the Orange one.
 */
export function MemoryOptIn({
  t,
  controls,
  label,
}: {
  t: Translate;
  controls: MemoryControls;
  label: string;
}) {
  const labelId = 'vpay-remember-label';
  const warningId = 'vpay-remember-warning';
  if (!controls.offered) {
    return null;
  }
  return (
    <div className="mt-1 flex flex-col gap-2">
      {/*
        `aria-labelledby`/`aria-describedby` rather than a wrapping `<label>`
        alone. Base UI's checkbox is a `<button role="checkbox">` with a
        hidden native input beside it for form value; a `<label>` names a
        native checkbox reliably and a button only by argument, so the
        association is written down. The `<label>` stays for the pointer
        behaviour — tapping the sentence toggles the box, which on a
        phone-sized page is most of the target.
      */}
      <label className="flex cursor-pointer items-start gap-3">
        <Checkbox.Root
          className="checkbox checkbox-sm mt-1 shrink-0"
          checked={controls.remember}
          onCheckedChange={controls.onRememberChange}
          aria-labelledby={labelId}
          aria-describedby={warningId}
          data-testid="remember"
        >
          <Checkbox.Indicator />
        </Checkbox.Root>
        <span className="text-sm">
          <span id={labelId}>{label}</span>
          <span
            id={warningId}
            className="mt-1 block text-xs opacity-70"
            data-testid="remember-warning"
          >
            {t('memory.warning')}
          </span>
        </span>
      </label>
      {controls.hasRecord ? (
        <button
          type="button"
          className="btn btn-ghost btn-xs self-start"
          data-testid="forget"
          onClick={controls.onForget}
        >
          {t('memory.forget')}
        </button>
      ) : null}
      {controls.forgotten ? (
        <p className="text-xs opacity-70" role="status" data-testid="forgotten">
          {t('memory.forgotten')}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The MTN path: one labelled field, one submit, one error tied to the field.
 *
 * The label/hint/error wiring is Base UI's `Field` rather than three
 * hand-kept `aria-describedby` ids. The `id` is still written down, because
 * `#vpay-msisdn` is what the Cypress specs type into and what
 * `checkout-view.test.tsx` asserts on.
 *
 * `defaultMsisdn` is what this device remembered. Uncontrolled on purpose —
 * a payer who edits a prefilled number must not have it put back — and the
 * value is resolved *before* the form first renders (`checkout-client.tsx`
 * reads memory ahead of the session), so there is no moment where a payer
 * could be typing into a field about to be replaced.
 */
export function MsisdnForm({
  t,
  amount,
  rail,
  problem,
  canGoBack,
  defaultMsisdn,
  memory,
  onSubmit,
  onBack,
}: {
  t: Translate;
  amount: string;
  rail: SupportedRail;
  problem: MessageKey | null;
  canGoBack: boolean;
  defaultMsisdn: string | null;
  memory: MemoryControls;
  onSubmit: (msisdn: string) => void;
  onBack: () => void;
}) {
  const inputId = 'vpay-msisdn';
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
        <Field.Root className="form-control gap-1" invalid={problem !== null}>
          <Field.Label className="label-text font-medium" htmlFor={inputId}>
            {t('msisdn.label')}
          </Field.Label>
          <Field.Control
            id={inputId}
            name="msisdn"
            type="tel"
            inputMode="tel"
            autoComplete="tel"
            className="input input-bordered w-full"
            defaultValue={defaultMsisdn ?? ''}
          />
          <Field.Description className="text-sm opacity-70">{t('msisdn.hint')}</Field.Description>
          {problem === null ? null : (
            <Field.Error
              match
              role="alert"
              className="text-sm font-medium text-error"
              data-testid="msisdn-problem"
            >
              {t(problem)}
            </Field.Error>
          )}
        </Field.Root>
        <MemoryOptIn t={t} controls={memory} label={t('memory.remember_number')} />
        <button type="submit" className="btn btn-primary btn-block">
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
  memory,
  onContinue,
  onBack,
}: {
  t: Translate;
  amount: string;
  rail: SupportedRail;
  problem: MessageKey | null;
  canGoBack: boolean;
  memory: MemoryControls;
  onContinue: () => void;
  onBack: () => void;
}) {
  return (
    <section>
      <ScreenHeading screen="ready_redirect">{t(rail.label)}</ScreenHeading>
      <p className="mt-3 opacity-80">{t('state.redirecting_body')}</p>
      {problem === null ? null : (
        <p role="alert" className="mt-3 text-sm font-medium text-error" data-testid="redirect-problem">
          {t(problem)}
        </p>
      )}
      <MemoryOptIn t={t} controls={memory} label={t('memory.remember_method', { rail: t(rail.label) })} />
      <button type="button" className="btn btn-primary btn-block mt-4" onClick={onContinue}>
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

/**
 * The end of a payment: what happened, and one button back.
 *
 * **There is no countdown and no timer** (the maintainer's requirement,
 * 2026-09-05; it was five seconds and not configurable until then). A page
 * that navigates on its own takes the outcome away from a payer who is
 * reading it, and on the failure screen it takes away the only text that
 * says *why* — on a handset, in a shop, with someone waiting. The button is
 * the whole mechanism: `destination === null` means the session named
 * nowhere to go, and the payer is told the payment is finished instead.
 *
 * `reason` is the rail's own sentence where the API gave one, rendered as
 * data under the translated message. See `failures.ts`.
 */
export function OutcomePanel({
  t,
  kind,
  failure,
  reason,
  merchant,
  amount,
  destination,
  onBack,
}: {
  t: Translate;
  kind: OutcomeKind;
  /** A `failure.*` key, already resolved from the intent's `last_payment_error`. */
  failure: MessageKey | null;
  /** The provider's own words, cleaned and bounded, or `null`. */
  reason: string | null;
  /** `null` where the read carried no usable merchant name. */
  merchant: string | null;
  amount: string;
  /** Where the button sends the payer, or `null` when the session names nowhere. */
  destination: string | null;
  onBack: () => void;
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
      {reason === null ? null : (
        <p className="mt-3 text-sm opacity-70" data-testid="provider-reason">
          <span className="font-medium">{t('outcome.provider_said')}: </span>
          {reason}
        </p>
      )}
      {destination === null ? (
        <p className="mt-4 opacity-80" data-testid="no-destination">
          {t('outcome.no_destination')}
        </p>
      ) : (
        <button type="button" className="btn btn-primary btn-block mt-4" onClick={onBack}>
          {merchantLine(t, merchant, 'outcome.back_to', 'outcome.back_to_unnamed')}
        </button>
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
