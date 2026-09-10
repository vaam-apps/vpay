/**
 * Every screen this page can show, as pure React.
 *
 * No `next/*` import, no `fetch`, no `window`: these components receive a
 * state and a translator and render. That is what lets the same functions
 * be a Storybook story, a vitest assertion in both locales, and the page
 * itself — rather than three descriptions of one design that drift.
 *
 * **Styling is `@vpay/ui`'s components — daisyUI's `bumblebee` theme and
 * Base UI's behaviour, composed through `cva` — and nothing else** (the
 * maintainer's requirement, 2026-09-05). This file writes **no** class name
 * of its own, which is what plan §3 asks for in as many words: raw
 * utilities live only inside `frontends/packages/ui/src/`. `Card`, `Alert`,
 * `Badge`, `Button`, `Field`, `Input`, `Checkbox`, `Spinner` and the layout
 * primitives `Stack`/`Text`/`Heading`/`List`/`PageShell`/`Logo`/
 * `VisuallyHidden`/`CheckboxLabel` own every daisyUI component class and
 * every layout utility between them
 * (`docs/plans/2026-09-07-ui-revamp.md` §3, §4.1).
 *
 * The five utilities this file did keep for one revision were functional
 * rather than decorative, and each is now a named variant or a component
 * instead: the payment amount is `<Text size="3xl" numeric>`, the session
 * reference `<Text wrap="anywhere">`, the operator's mark `<Logo>` (whose
 * `h-8 w-auto` overrides Tailwind's own `img{height:auto}` preflight
 * reset), the visually hidden labels `<VisuallyHidden>`, and the memory
 * opt-in's clickable sentence `<CheckboxLabel>`. Colour is never written
 * down — it comes from the theme's own variables, which
 * `src/config/theme.ts` lets an operator retint at runtime, and status tone
 * still comes from `@vpay/tokens` (AGENTS.md).
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
 * `input` or `input[type=radio]`", and it is no longer exactly true — but
 * Base UI 1.8.0 gets it closer than the rc it replaces: decision D2
 * (2026-09-07) renders the checkbox as a real `<button role="checkbox">`
 * rather than the `<span role="checkbox">` `@vpay/ui`'s own rc-era default
 * would have produced, so a keyboard-only payer reaches it exactly as they
 * reach every other button on the page. It is still an ARIA role rather
 * than the native `input[type=checkbox]` the sentence originally promised,
 * so the test still asserts the role, the tab index, the accessible name
 * and `aria-checked` explicitly rather than assuming the platform.
 */
import { useEffect } from "react";
import {
  Alert,
  Badge,
  Button,
  Card,
  CardBody,
  Checkbox,
  CheckboxLabel,
  Field,
  FieldDescription,
  FieldError,
  FieldLabel,
  Heading,
  Input,
  List,
  Logo,
  Spinner,
  Stack,
  Text,
  VisuallyHidden,
} from "@vpay/ui";
import { checkoutOutcomeTone } from "@vpay/tokens";

import type { MessageKey, Translate } from "../i18n/index";
import type { Branding } from "../config/settings";
import type { OutcomeKind } from "../lib/machine";
import type { RailChoices, SupportedRail } from "../lib/rails";

/**
 * The heading every screen starts with.
 *
 * Focus moves here whenever `screen` changes. `preventScroll` is not used:
 * a payer whose viewport does not currently show the heading should be
 * scrolled to it.
 *
 * Focused via `document.querySelector('[data-screen]')` rather than a
 * `ref` on `Heading`: `Heading`'s own props type is
 * `React.ComponentPropsWithoutRef<'h1'>`, which — by design, the same way
 * every other `@vpay/ui` component works — does not accept a `ref`. Exactly
 * one screen is ever mounted at a time (`checkout-view.tsx`'s `switch`
 * renders one branch), so the query is unambiguous, and this is the same
 * property Cypress already relies on when it selects `[data-screen="…"]`.
 */
export function ScreenHeading({
  screen,
  children,
}: {
  screen: string;
  children: React.ReactNode;
}) {
  useEffect(() => {
    document.querySelector<HTMLElement>(`[data-screen="${screen}"]`)?.focus();
  }, [screen]);
  return (
    <Heading level={2} tabIndex={-1} data-screen={screen}>
      {children}
    </Heading>
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
  return merchant === null
    ? t(unnamed, values)
    : t(named, { ...values, merchant });
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
export function BrandHeader({
  t,
  branding,
}: {
  t: Translate;
  branding: Branding;
}) {
  return (
    <Stack gap="md">
      {branding.logoUrl === null ? null : (
        <Logo
          src={branding.logoUrl}
          alt={branding.displayName ?? t("page.operator_logo_alt")}
          data-testid="brand-logo"
        />
      )}
      <Heading level={1} data-testid="brand-name">
        {branding.displayName ?? t("page.title")}
      </Heading>
    </Stack>
  );
}

/** `branding.yaml`'s `support_contact`, as text. Never a `mailto:` or a `tel:` this page composed. */
export function SupportLine({
  t,
  branding,
}: {
  t: Translate;
  branding: Branding;
}) {
  if (branding.supportContact === null) {
    return null;
  }
  return (
    <Text tone="muted" size="xs" data-testid="support-contact">
      {t("page.support", { contact: branding.supportContact })}
    </Text>
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
    <Card>
      <CardBody>
        {/*
          A paragraph, not a daisyUI `badge`. It was a badge for one
          revision and the screenshot showed why that was wrong: `.badge` is
          `whitespace-nowrap` by design, and this string is a sentence —
          "Test mode — no money moves on this deployment." spilled straight
          out of the card. A badge is for a word.
        */}
        {!livemode ? (
          <Text size="xs" weight="semibold" data-testid="testmode">
            {t("page.testmode")}
          </Text>
        ) : null}
        <Text size="sm" tone="muted" data-testid="pay-to">
          {merchantLine(t, merchant, "page.pay_to", "page.pay_to_unnamed")}
        </Text>
        <Text size="3xl" weight="semibold" numeric data-testid="amount">
          <VisuallyHidden>{t("page.amount_label")}: </VisuallyHidden>
          {amount}
        </Text>
        <Text size="xs" tone="muted" wrap="anywhere" data-testid="reference">
          {t("page.reference_label")}: {reference}
        </Text>
      </CardBody>
    </Card>
  );
}

/** D9: rails the intent offers that this page has no flow for, or the operator's `config.yaml` excludes. */
export function UnsupportedRails({
  t,
  codes,
}: {
  t: Translate;
  codes: readonly string[];
}) {
  if (codes.length === 0) {
    return null;
  }
  return (
    <List data-testid="unsupported-rails">
      {codes.map((code) => (
        <li key={code}>{t("rail.unsupported", { rail: code })}</li>
      ))}
    </List>
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
      <Stack direction="column" align="stretch" gap="md">
        <ScreenHeading screen="select_rail">{t("rail.legend")}</ScreenHeading>
        <Stack direction="column" align="stretch" gap="sm">
          {rails.supported.map((rail) => (
            <Button
              key={rail.code}
              type="button"
              variant="outline"
              data-rail={rail.code}
              onClick={() => onChoose(rail)}
            >
              {t(rail.label)}
              {rail.code === lastRail ? (
                <Badge tone="ghost" size="sm" data-testid="last-used">
                  {t("memory.last_used")}
                </Badge>
              ) : null}
            </Button>
          ))}
        </Stack>
        <UnsupportedRails t={t} codes={rails.unsupported} />
      </Stack>
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
  const labelId = "vpay-remember-label";
  const warningId = "vpay-remember-warning";
  if (!controls.offered) {
    return null;
  }
  return (
    <Stack direction="column" align="start" gap="sm">
      {/*
        `aria-labelledby`/`aria-describedby` rather than a wrapping `<label>`
        alone. Decision D2's checkbox renders a native `<button
        role="checkbox">` — measured against `@vpay/ui`'s own tests, not
        assumed — and while a `<label>` names it as reliably as it names any
        native control, the association is written down explicitly rather
        than left to a browser to notice a single text child. The
        `<label>` stays for the pointer behaviour: tapping the sentence
        toggles the box, which on a phone-sized page is most of the target,
        and a `<button>` is a labelable element so the browser's own
        label-click forwarding still applies — measured, not assumed.

        The two ids are constants rather than `useId` values because exactly
        one entry screen is on the page at a time — `collect_msisdn` and
        `ready_redirect` are different states of one machine. Two of these
        rendered together would be two elements sharing an id.
      */}
      <CheckboxLabel>
        <Stack align="start" gap="md">
          <Checkbox
            checked={controls.remember}
            onCheckedChange={controls.onRememberChange}
            aria-labelledby={labelId}
            aria-describedby={warningId}
            data-testid="remember"
          />
          <Stack direction="column" gap="xs">
            <Text as="span" id={labelId}>
              {label}
            </Text>
            <Text
              as="span"
              id={warningId}
              tone="muted"
              size="xs"
              data-testid="remember-warning"
            >
              {t("memory.warning")}
            </Text>
          </Stack>
        </Stack>
      </CheckboxLabel>
      {controls.hasRecord ? (
        <Button
          type="button"
          variant="ghost"
          size="xs"
          data-testid="forget"
          onClick={controls.onForget}
        >
          {t("memory.forget")}
        </Button>
      ) : null}
      {controls.forgotten ? (
        <Text size="xs" tone="muted" role="status" data-testid="forgotten">
          {t("memory.forgotten")}
        </Text>
      ) : null}
    </Stack>
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
  const inputId = "vpay-msisdn";
  return (
    <section>
      <Stack direction="column" align="stretch" gap="md">
        <ScreenHeading screen="collect_msisdn">{t(rail.label)}</ScreenHeading>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const data = new FormData(event.currentTarget);
            // `FormData.get` is `string | File | null`. A non-string entry
            // is not something this form can produce, but stringifying one
            // would hand the MSISDN validator the text "[object File]"
            // rather than an empty field.
            const raw = data.get("msisdn");
            onSubmit(typeof raw === "string" ? raw : "");
          }}
        >
          <Stack direction="column" align="stretch" gap="sm">
            <Field invalid={problem !== null}>
              <FieldLabel htmlFor={inputId}>{t("msisdn.label")}</FieldLabel>
              <Input
                id={inputId}
                name="msisdn"
                type="tel"
                inputMode="tel"
                autoComplete="tel"
                defaultValue={defaultMsisdn ?? ""}
              />
              <FieldDescription>{t("msisdn.hint")}</FieldDescription>
              {problem === null ? null : (
                <FieldError match role="alert" data-testid="msisdn-problem">
                  {t(problem)}
                </FieldError>
              )}
            </Field>
            <MemoryOptIn
              t={t}
              controls={memory}
              label={t("memory.remember_number")}
            />
            <Button type="submit" block>
              {t("msisdn.submit", { amount })}
            </Button>
            {canGoBack ? (
              <Button type="button" variant="ghost" size="sm" onClick={onBack}>
                {t("msisdn.back")}
              </Button>
            ) : null}
          </Stack>
        </form>
      </Stack>
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
      <Stack direction="column" align="stretch" gap="md">
        <ScreenHeading screen="ready_redirect">{t(rail.label)}</ScreenHeading>
        <Text tone="muted">{t("state.redirecting_body")}</Text>
        {problem === null ? null : (
          <Alert tone="error" role="alert" data-testid="redirect-problem">
            {t(problem)}
          </Alert>
        )}
        <MemoryOptIn
          t={t}
          controls={memory}
          label={t("memory.remember_method", { rail: t(rail.label) })}
        />
        <Button type="button" block data-testid="continue" onClick={onContinue}>
          {t("msisdn.submit", { amount })}
        </Button>
        {canGoBack ? (
          <Button type="button" variant="ghost" size="sm" onClick={onBack}>
            {t("msisdn.back")}
          </Button>
        ) : null}
      </Stack>
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
      <Stack direction="column" align="stretch" gap="md">
        <ScreenHeading screen={screen}>{title}</ScreenHeading>
        {body === null ? null : <Text tone="muted">{body}</Text>}
        <Spinner />
        {notice ? (
          <Stack direction="column" align="start" gap="sm">
            <Text weight="semibold" data-testid="poll-notice">
              {t(notice)}
            </Text>
            {onRetry === undefined ? null : (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={onRetry}
              >
                {t("error.retry")}
              </Button>
            )}
          </Stack>
        ) : null}
      </Stack>
    </section>
  );
}

/**
 * Outcome tone comes from `@vpay/tokens`, never from a colour written here
 * (AGENTS.md) — and from `checkoutOutcomeTone` rather than from `statusTone`.
 *
 * This read `statusTone[OUTCOME_STATUS[kind]]` until 2026-09-07, where
 * `OUTCOME_STATUS` mapped `failed` onto the intent status a failed attempt
 * leaves behind, `requires_payment_method`. That mapping is *accurate* and it
 * produced the wrong screen: `requires_payment_method` is `neutral`, because
 * on a dashboard it means "awaiting a payment method", so a payer whose
 * payment had failed read a GREY box while a payer who cancelled read a red
 * one. It is on the committed screenshot. The operator's status palette is
 * not the payer's outcome palette, and `@vpay/tokens` now says both.
 *
 * `checkoutOutcomeTone[kind]` is passed straight into `Alert`'s `tone` prop
 * — there is no intermediate class map in this file any more (there was,
 * `TONE_CLASS`, and a template literal building `` `alert mt-4 ${tone}` ``;
 * both are gone). `@vpay/tokens`' own type already lines up with `Alert`'s
 * `tone` variant, so there is nothing left here that could reintroduce the
 * defect the constant existed to fix.
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
    kind === "succeeded"
      ? t("outcome.succeeded_title")
      : kind === "canceled"
        ? t("outcome.canceled_title")
        : t("outcome.failed_title");
  const body =
    kind === "succeeded"
      ? merchantLine(
          t,
          merchant,
          "outcome.succeeded_body",
          "outcome.succeeded_body_unnamed",
          {
            amount,
          },
        )
      : kind === "canceled"
        ? t("outcome.canceled_body")
        : t(failure ?? "failure.unknown");
  return (
    <section data-outcome={kind}>
      <Stack direction="column" align="stretch" gap="md">
        <ScreenHeading screen="outcome">{title}</ScreenHeading>
        <Alert tone={checkoutOutcomeTone[kind]} role="status">
          <span data-testid="outcome-body">{body}</span>
        </Alert>
        {reason === null ? null : (
          <Text size="sm" tone="muted" data-testid="provider-reason">
            <Text as="span" weight="semibold">
              {t("outcome.provider_said")}:{" "}
            </Text>
            {reason}
          </Text>
        )}
        {destination === null ? (
          <Text tone="muted" data-testid="no-destination">
            {t("outcome.no_destination")}
          </Text>
        ) : (
          <Button type="button" block onClick={onBack}>
            {merchantLine(
              t,
              merchant,
              "outcome.back_to",
              "outcome.back_to_unnamed",
            )}
          </Button>
        )}
      </Stack>
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
      <Stack direction="column" align="stretch" gap="md">
        <ScreenHeading screen={screen}>{title}</ScreenHeading>
        <Alert tone="warning">
          <span data-testid="notice-body">{body}</span>
        </Alert>
        <VisuallyHidden>{t("error.title")}</VisuallyHidden>
      </Stack>
    </section>
  );
}
