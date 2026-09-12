/**
 * `CheckoutState` → a screen. One `switch`, no side effects.
 *
 * Kept separate from the client component that owns the controller so that
 * every screen state can be rendered in a test and in Storybook from a
 * literal state value — including the ones that are hard to reach through
 * the network (an expired session, a refused embed, a rail the page cannot
 * drive). A screenshot of a state nobody can produce is how a page ends up
 * with a branch that has never rendered.
 *
 * **`@vaam-apps/ui` cutover (2026-09-12).** `PageShell` and `Stack` have no
 * counterpart in the new package (verified against `dist/index.d.ts`):
 * `ScreenStack` (`@vaam-apps/ui`) replaces `PageShell` for the vertical
 * rhythm only — no centring, no `max-w`, no padding of its own, so a bare
 * `<ScreenStack>` briefly shipped full-bleed at any viewport width, a
 * regression on the payer's own page that the dashboard's equivalent
 * (`app/layout.tsx`) had already been given a fix for. Closed the same way,
 * here at the call site rather than inside the component, because
 * `ScreenStack` forwards `className` for exactly this: `className="mx-auto
 * w-full max-w-md p-6"` restores `PageShell`'s old centred column.
 * `max-w-md` — not the dashboard's `max-w-5xl` — because this is a
 * single-column payment form (an amount, a rail, a phone number), not a
 * table; a wide measure would just stretch that form's line length and its
 * button width without showing more information. `return-view.tsx` renders
 * the same shell for the same reason and carries the identical class. A
 * plain `<header>` replaces `Stack as="header"`: `checkout-view.test.tsx`'s
 * own "keeps the brand-and-language row a banner landmark" test is exactly
 * the guard against losing that element a second time. `LiveRegion` has no
 * counterpart either — `@vaam-apps/ui` exports no live region at all — so
 * this is a plain `<div aria-live="polite" aria-atomic="true">`, faithful
 * but without the test `@vpay/ui`'s own `LiveRegion` carried guaranteeing a
 * hostile caller cannot weaken those two attributes.
 */
import { ScreenStack } from "@vaam-apps/ui";

import type { Branding } from "../config/settings";
import type { Locale, MessageKey, Translate } from "../i18n/index";
import { failureMessage } from "../lib/failures";
import type { CheckoutState } from "../lib/machine";
import { formatAmount } from "../lib/money";
import type { SupportedRail } from "../lib/rails";
import { LocaleSwitch } from "./locale-switch";
import {
  BrandHeader,
  MsisdnForm,
  NoticePanel,
  OutcomePanel,
  PaymentSummary,
  RailSelector,
  RedirectPrompt,
  StatusPanel,
  SupportLine,
  merchantLine,
  type MemoryControls,
} from "./screens";

export interface CheckoutViewHandlers {
  onChooseRail: (rail: SupportedRail) => void;
  onBack: () => void;
  onSubmitMsisdn: (msisdn: string) => void;
  onStartRedirect: () => void;
  onRetryPoll: () => void;
  /** The outcome screen's one control. Named for what it does: there is no timer behind it. */
  onReturnToMerchant: () => void;
  onLocaleChange: (locale: Locale) => void;
}

export interface CheckoutViewProps extends CheckoutViewHandlers {
  state: CheckoutState;
  t: Translate;
  locale: Locale;
  /** The deployment's own `branding.yaml`, read at container start. */
  branding: Branding;
  /** Where the outcome screen's button sends the payer, or `null`. Computed by the caller from the session. */
  destination: string | null;
  /** What this device remembered, already resolved. `null` for nothing. */
  defaultMsisdn: string | null;
  /** The rail this device last paid with, marked in the selector. Never preselected. */
  lastRail: string | null;
  /** The opt-in, its state, and the way to clear it. */
  memory: MemoryControls;
}

/** The session-bearing states, so the summary is rendered once rather than per screen. */
function contextOf(state: CheckoutState) {
  return "context" in state ? state.context : null;
}

export function CheckoutView(props: CheckoutViewProps) {
  const { state, t, locale } = props;
  const context = contextOf(state);
  const amount =
    context === null
      ? ""
      : formatAmount(context.intent.amount, context.intent.currency, locale);
  // `null`, not `''`: the screens choose a neutrally-worded sentence for a
  // session whose read carried no merchant name, rather than rendering one
  // written for a name with the name missing.
  const merchant = context?.merchant?.name ?? null;

  return (
    <main>
      <ScreenStack className="mx-auto w-full max-w-md p-6">
        <header className="flex items-center justify-between gap-4">
          <BrandHeader t={t} branding={props.branding} />
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

        <SupportLine t={t} branding={props.branding} />
      </ScreenStack>
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
    case "loading":
      return (
        <StatusPanel
          t={t}
          screen="loading"
          title={t("state.loading")}
          body={null}
        />
      );

    case "error":
      return (
        <NoticePanel
          t={t}
          screen="error"
          title={t("error.title")}
          body={t(state.error.code)}
          code={state.error.serverCode}
        />
      );

    case "refused":
      return state.reason === "embed_not_allowed" ? (
        <NoticePanel
          t={t}
          screen="refused_embed"
          title={t("refusal.embed_title")}
          body={t("refusal.embed_body")}
        />
      ) : (
        <NoticePanel
          t={t}
          screen="refused_rail"
          title={t("error.title")}
          body={t("rail.none")}
        />
      );

    case "expired":
      return (
        <NoticePanel
          t={t}
          screen="expired"
          title={t("expired.title")}
          body={merchantLine(
            t,
            merchant,
            "expired.body",
            "expired.body_unnamed",
          )}
        />
      );

    case "select_rail":
      return (
        <RailSelector
          t={t}
          rails={state.rails}
          lastRail={props.lastRail}
          onChoose={props.onChooseRail}
        />
      );

    case "collect_msisdn":
      return (
        <MsisdnForm
          t={t}
          amount={amount}
          rail={state.rail}
          problem={state.problem}
          canGoBack={state.rails.supported.length > 1}
          defaultMsisdn={props.defaultMsisdn}
          memory={props.memory}
          onSubmit={props.onSubmitMsisdn}
          onBack={props.onBack}
        />
      );

    case "ready_redirect":
      return (
        <RedirectPrompt
          t={t}
          amount={amount}
          rail={state.rail}
          problem={state.problem}
          canGoBack={state.rails.supported.length > 1}
          memory={props.memory}
          onContinue={props.onStartRedirect}
          onBack={props.onBack}
        />
      );

    case "confirming":
      return (
        <StatusPanel
          t={t}
          screen="confirming"
          title={t("state.confirming")}
          body={null}
        />
      );

    case "waiting":
      return (
        <StatusPanel
          t={t}
          screen="waiting"
          title={t("state.waiting_title")}
          body={t("state.waiting_body", { amount })}
          notice={state.notice}
          onRetry={state.notice === null ? undefined : props.onRetryPoll}
        />
      );

    case "redirecting":
      return (
        <StatusPanel
          t={t}
          screen="redirecting"
          title={t("state.redirecting_title")}
          body={t("state.redirecting_body")}
        />
      );

    case "outcome":
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

    case "forwarding":
      return (
        <StatusPanel
          t={t}
          screen="forwarding"
          title={t("state.forwarding_title")}
          body={merchantLine(
            t,
            merchant,
            "state.forwarding_body",
            "state.forwarding_body_unnamed",
          )}
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
