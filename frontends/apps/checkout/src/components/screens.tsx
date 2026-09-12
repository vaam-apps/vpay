/**
 * Every screen this page can show, as pure React.
 *
 * No `next/*` import, no `fetch`, no `window`: these components receive a
 * state and a translator and render. That is what lets the same functions
 * be a Storybook story, a vitest assertion in both locales, and the page
 * itself — rather than three descriptions of one design that drift.
 *
 * **Styling is `@vaam-apps/ui`'s components and daisyUI classes on top of
 * its one dark theme (2026-09-12, replacing `@vpay/ui`'s `bumblebee` +
 * `cva`).** `@vaam-apps/ui` has no counterpart for several of the layout
 * primitives `@vpay/ui` provided purely to hold a class string — there is
 * no `Stack`, `Text`, `Heading`, `List`, `PageShell`, `Logo` or
 * `VisuallyHidden` in the new package (verified against
 * `<SP>/vaam-ui/package/dist/index.d.ts`) — so this file now writes plain
 * HTML for those, with Tailwind layout utilities and daisyUI's own
 * component classes where needed (`w-full` on the payer-facing CTAs,
 * `sr-only` for the visually-hidden labels, `font-mono`/`text-metric` for
 * the amount). Every class here is a layout utility or a semantic theme
 * token — daisyUI classes, `--state-*`/`--color-*`-driven utilities, or
 * plain flex/gap/spacing — never a literal colour, a hex, or a raw
 * Tailwind palette class, and `cva` is not used in this app.
 *
 * What DOES have a component: `Card`/`CardBody`, `Button`, `Input`,
 * `CheckboxField`, `FormField`, `InlineBanner` (replacing `Alert`), `Badge`,
 * `Spinner`. Several of those changed shape, not just name — see each
 * function's own comment for what moved and why:
 *
 * - `InlineBanner` renders no `role` at all (verified: `inline-banner.js` is
 *   a plain `<div>`), where `@vpay/ui`'s `Alert` defaulted to `role="alert"`.
 *   Every site that relied on that default now wraps `InlineBanner` in a
 *   plain element carrying the role explicitly — `RedirectPrompt` and
 *   `OutcomePanel` name it as an explicit prop already; `NoticePanel`
 *   restores the default it used to get for free.
 * - **`aria-labelledby`/`aria-describedby` cannot be passed to
 *   `@vaam-apps/ui`'s `Checkbox`/`CheckboxField` at all — not a typing gap,
 *   a RUNTIME one.** Headless UI's own `checkbox.d.ts` names them
 *   (alongside `role`/`aria-checked`/`aria-disabled`) as
 *   `CheckboxPropsWeControl` — attributes the component computes itself and
 *   will not take from a caller, confirmed live: passing
 *   `aria-describedby="x"` renders a `<button>` with no
 *   `aria-describedby` attribute at all. `MemoryOptIn` below uses
 *   `CheckboxField` instead, whose own `<Field>`/`<Label>` composition
 *   (Headless UI's, not a manually-passed id string) computes
 *   `aria-labelledby` correctly — measured live: it renders a real
 *   `<label for="…">` and the checkbox's `aria-labelledby` resolves to it.
 *   Both the sentence AND the warning are passed as `CheckboxField`'s
 *   `label` (not split into a label + a separately-described warning): its
 *   own `description` slot carries no ARIA wiring at all, so folding both
 *   into one `<label>` is what actually gets the warning announced with the
 *   control, at the cost of the warning being part of the accessible NAME
 *   rather than a separate description. `CheckboxField`'s type is
 *   similarly closed against `as`/`type`/`data-testid`, widened the same
 *   way as `Checkbox` below for the same reason.
 *   Also default to `as="span"` (Headless UI's own default tag), not a
 *   `<button>` — passing `as="button"` restores the D2 guarantee this page
 *   depends on: a labelable, keyboard-reachable control a wrapping `<label>`
 *   can activate. Measured live: rendering it with `as="button"` produces
 *   `<button type="button" role="checkbox" tabindex="0">`, and a click
 *   anywhere in the real `<label for="…">` `CheckboxField` renders DOES
 *   forward to it in jsdom.
 * - `FormField` has no `invalid` prop, and its own `error`/`hint` slots
 *   render no `data-testid`/`id` at all (its `FieldError` takes only
 *   `{children, className}` and hard-codes `role="alert"`). The MSISDN
 *   field therefore composes `FormField` as a label+layout wrapper only (no
 *   `hint`/`error` props), and writes the hint paragraph and the error text
 *   as explicit children with their own ids, because
 *   `checkout-view.test.tsx` queries the error by `data-testid` and then
 *   reads its `role` off the SAME node, and `FieldError` cannot carry a
 *   `data-testid`. **Updated 2026-09-12, reconciling with `verify-ui`'s new
 *   7a-ii:** an earlier draft copied `FieldError`'s own caption class plus
 *   its danger-hue text token by hand onto that node — a status colour
 *   written directly in an app, which check 7a-ii now refuses for exactly
 *   the reason `AGENTS.md` gives ("never inline a status colour in a
 *   component"; the token is check 7a-ii's own `text-state-<hue>-fg`
 *   family, not spelled out here contiguously on purpose — see that
 *   check's own comment in `justfile` for the exact pattern it refuses).
 *   The error text is an `InlineBanner variant="danger"`
 *   instead, nested one level inside the identified node (`id`/`role`/
 *   `data-testid` live on the wrapping element, matching `ReadyRedirect`'s
 *   `role="alert"` + `InlineBanner` pairing above) — `InlineBanner` forwards
 *   neither `id` nor `data-testid` (verified `inline-banner.js`: it
 *   destructures only `{variant, children, className}`), so those identities
 *   cannot live on the banner itself. `aria-describedby` on the
 *   input is wired by hand too — the association `@vpay/ui`'s `Field` used
 *   to do through context. This restores the exact order the old markup had
 *   (label, control, hint, error) rather than accepting `FormField`'s own
 *   `hint`-before-`children` placement. (`Input`'s own `aria-describedby` IS
 *   respected — it is a plain forwarded HTML attribute, not one of Headless
 *   UI's controlled ARIA props, and `Input` extends `InputHTMLAttributes`
 *   directly rather than going through a Headless UI primitive.)
 *
 * What has no equivalent and is now a real, visible regression, named
 * rather than absorbed silently: `Button`'s `size="xs"` (the "forget"
 * button falls back to `"sm"`, slightly larger than before) and `block`
 * (replaced with an explicit `w-full` — the three payer-facing CTAs still
 * span full width, just via a class instead of a prop). `CheckboxLabel`
 * has no counterpart either; `CheckboxField` (below) is the closest
 * available primitive, not a port of it. `Input` itself has no error/invalid
 * variant at all (verified `input.js`: it accepts only `className` and
 * passthrough props, no `tone`), and the only styling an app is allowed to
 * reach for on a bad value — a raw `input-error` class — is itself a
 * daisyUI component class check 7b now refuses in an app. The MSISDN field
 * therefore renders no visual border change on an invalid value; `aria-invalid`
 * is still set for assistive technology, and the `InlineBanner` error text
 * below the field is the payer-visible signal. Named here rather than
 * worked around, because there is no `@vaam-apps/ui` primitive this could
 * legitimately compose into.
 *
 * Colour is never written down — it comes from the theme's own variables,
 * which `src/config/theme.ts` lets an operator retint at runtime, and
 * status tone still comes from `@vpay/tokens` (AGENTS.md) —
 * `checkoutOutcomeVariant` now, `@vaam-apps/ui`'s variant vocabulary rather
 * than daisyUI's semantic tone names; see that export's own doc comment.
 *
 * Accessibility is structural here, not decorative:
 *
 * - every screen's heading carries `tabIndex={-1}` and is focused when the
 *   screen changes, so a payer on a screen reader or a keyboard is moved to
 *   the new content instead of being left on a button that no longer exists;
 * - the status text sits in an `aria-live="polite"` region that is present
 *   from first render, because a live region added to the DOM at the same
 *   moment as its text is not announced;
 * - the MSISDN field's label, hint and error are tied to the input by hand
 *   (`aria-describedby`, `aria-invalid`) — see `FormField`'s note above;
 * - every control has a real accessible name and is in the tab order.
 */
import { useEffect } from "react";
import {
  Badge,
  Button,
  Card,
  CardBody,
  CheckboxField as VaamCheckboxField,
  FormField,
  InlineBanner,
  Input,
  Spinner,
  type CheckboxFieldProps,
} from "@vaam-apps/ui";
import { checkoutOutcomeVariant } from "@vpay/tokens";

import type { MessageKey, Translate } from "../i18n/index";
import type { Branding } from "../config/settings";
import type { OutcomeKind } from "../lib/machine";
import type { RailChoices, SupportedRail } from "../lib/rails";

/**
 * `@vaam-apps/ui`'s `CheckboxFieldProps` declares no `as`, `type` or
 * `data-testid`, even though `checkbox.js` spreads every prop it doesn't
 * name (`...props`, forwarded onto the inner `Checkbox`, itself spreading
 * `...aria` onto Headless UI's own component) — measured live: rendering
 * `<CheckboxField as="button" type="button" data-testid="y" checked={false}
 * label="…">` produces a `<button type="button" role="checkbox"
 * data-testid="y" aria-labelledby="…" tabindex="0">` wired to a real
 * `<label for="…">`.
 *
 * This widens the TYPE to match what the component already does at
 * runtime, in one place, rather than a cast repeated at every call site
 * below. **A variable type ANNOTATION, not an `as unknown as` assertion
 * (2026-09-12):** the assertion form flagged
 * `@typescript-eslint/no-unnecessary-type-assertion` at this exact
 * statement — correctly, for the statement in isolation, since assigning
 * `VaamCheckboxField` to a same-shaped `const` needs no cast — but
 * `--fix`ing it away breaks every call site below that passes `as`/`type`/
 * `data-testid`, which JSX's excess-property check then refuses against the
 * narrow, un-widened type (`tsc` catches it immediately: "Property 'as'
 * does not exist on type '…CheckboxFieldProps'"). An explicit annotation
 * widens the same way without asserting anything, so the rule has nothing
 * to flag.
 */
const CheckboxField: (
  props: CheckboxFieldProps & {
    as?: "button";
    type?: "button";
    "data-testid"?: string;
  },
) => ReturnType<typeof VaamCheckboxField> = VaamCheckboxField;

/**
 * The heading every screen starts with.
 *
 * Focus moves here whenever `screen` changes. `preventScroll` is not used:
 * a payer whose viewport does not currently show the heading should be
 * scrolled to it.
 *
 * A plain `<h2>` rather than a component: `@vaam-apps/ui` has no `Heading`,
 * and its `ScreenHeader` always renders an `<h1>` with no passthrough props
 * at all (`{title, description}` only — verified `screen-layout.d.ts`), so
 * it cannot carry `tabIndex`, `data-screen`, or the `<h2>` level this page
 * needs. Focused via `document.querySelector('[data-screen]')` rather than
 * a ref, for the same reason as before: exactly one screen is ever mounted
 * at a time, so the query is unambiguous, and it is the same property
 * Cypress already relies on when it selects `[data-screen="…"]`.
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
    <h2 className="text-title font-semibold" tabIndex={-1} data-screen={screen}>
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
 *
 * `h-8 w-auto` on the `<img>` is a functional class, not a decorative one:
 * without it Tailwind's own preflight (`img { height: auto }`) leaves the
 * logo at its file's intrinsic size, which `@vpay/ui`'s `Logo` used to
 * override. `@vaam-apps/ui` has no `Logo`.
 */
export function BrandHeader({
  t,
  branding,
}: {
  t: Translate;
  branding: Branding;
}) {
  return (
    <div className="flex flex-col gap-2">
      {branding.logoUrl === null ? null : (
        // eslint-disable-next-line @next/next/no-img-element -- the operator's own logo, not a local asset Next can optimise.
        <img
          src={branding.logoUrl}
          alt={branding.displayName ?? t("page.operator_logo_alt")}
          data-testid="brand-logo"
          className="h-8 w-auto"
        />
      )}
      <h1 className="text-title-sm font-semibold" data-testid="brand-name">
        {branding.displayName ?? t("page.title")}
      </h1>
    </div>
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
    <p
      className="text-caption text-muted-foreground"
      data-testid="support-contact"
    >
      {t("page.support", { contact: branding.supportContact })}
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
          <p className="text-caption font-semibold" data-testid="testmode">
            {t("page.testmode")}
          </p>
        ) : null}
        <p className="text-caption text-muted-foreground" data-testid="pay-to">
          {merchantLine(t, merchant, "page.pay_to", "page.pay_to_unnamed")}
        </p>
        <p className="font-mono text-metric font-semibold" data-testid="amount">
          <span className="sr-only">{t("page.amount_label")}: </span>
          {amount}
        </p>
        <p
          className="text-caption wrap-anywhere text-muted-foreground"
          data-testid="reference"
        >
          {t("page.reference_label")}: {reference}
        </p>
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
    <ul
      className="flex flex-col gap-1 text-caption text-muted-foreground"
      data-testid="unsupported-rails"
    >
      {codes.map((code) => (
        <li key={code}>{t("rail.unsupported", { rail: code })}</li>
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
      <div className="flex flex-col gap-4">
        <ScreenHeading screen="select_rail">{t("rail.legend")}</ScreenHeading>
        <div className="flex flex-col gap-2">
          {rails.supported.map((rail) => (
            <Button
              key={rail.code}
              type="button"
              variant="secondary"
              data-rail={rail.code}
              onClick={() => onChoose(rail)}
            >
              {t(rail.label)}
              {rail.code === lastRail ? (
                <Badge variant="outline" data-testid="last-used">
                  {t("memory.last_used")}
                </Badge>
              ) : null}
            </Button>
          ))}
        </div>
        <UnsupportedRails t={t} codes={rails.unsupported} />
      </div>
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
  if (!controls.offered) {
    return null;
  }
  return (
    <div className="flex flex-col items-start gap-2">
      {/*
        `CheckboxField`, not a hand-built `<label>` + `Checkbox` pair.
        Headless UI's `Checkbox` treats `aria-labelledby`/`aria-describedby`
        as attributes IT controls and silently drops a manually-passed
        value for either — measured live: a `<Checkbox
        aria-describedby="x">` renders no `aria-describedby` at all.
        `CheckboxField` gets a correct `aria-labelledby` instead, because
        its own `<Field>`/`<Label>` composition computes it internally
        (verified live: a real `<label for="…">`, and the checkbox's
        `aria-labelledby` resolves to it).

        Both the sentence and the warning are passed together as `label`,
        not split into a label + a separately-described warning:
        `CheckboxField`'s own `description` slot is a plain, unwired `<span>`
        (verified `checkbox.js`), so folding both into the one real `<label>`
        is what actually gets the warning announced together with the
        control — the property this control has always been built around
        ("the warning is on the control, not a tooltip"). The cost is that
        the warning is now part of the accessible NAME rather than a
        separate description; `screens.axe.test.tsx`'s `aria-*` rules do not
        distinguish the two, and no test asserted a description role
        specifically.

        `as="button"` restores decision D2's guarantee: Headless UI's
        `Checkbox` defaults to `<span role="checkbox">`, and a `<span>` is
        not a labelable element, so the wrapping `<label>` would not forward
        a click to it at all without this.
      */}
      <CheckboxField
        as="button"
        type="button"
        checked={controls.remember}
        onCheckedChange={controls.onRememberChange}
        data-testid="remember"
        label={
          <span className="flex flex-col gap-1">
            <span>{label}</span>
            <span
              className="text-caption text-muted-foreground"
              data-testid="remember-warning"
            >
              {t("memory.warning")}
            </span>
          </span>
        }
      />
      {controls.hasRecord ? (
        <Button
          type="button"
          variant="ghost"
          size="sm"
          data-testid="forget"
          onClick={controls.onForget}
        >
          {t("memory.forget")}
        </Button>
      ) : null}
      {controls.forgotten ? (
        <p
          className="text-caption text-muted-foreground"
          role="status"
          data-testid="forgotten"
        >
          {t("memory.forgotten")}
        </p>
      ) : null}
    </div>
  );
}

/**
 * The MTN path: one labelled field, one submit, one error tied to the field.
 *
 * `FormField` composes only the label and the layout here (no `hint`/`error`
 * props): its own `error` slot exposes no `data-testid`, and its `hint`
 * placement (before the control) would reorder the field from how it always
 * read (label, input, hint, error). Writing the hint and the error as plain
 * children keeps that order and lets `aria-describedby` name both by hand —
 * the wiring `@vpay/ui`'s `Field` used to do through context.
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
  const hintId = "vpay-msisdn-hint";
  const errorId = "vpay-msisdn-problem";
  return (
    <section>
      <div className="flex flex-col gap-4">
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
          <div className="flex flex-col gap-3">
            <FormField label={t("msisdn.label")} htmlFor={inputId}>
              <Input
                id={inputId}
                name="msisdn"
                type="tel"
                inputMode="tel"
                autoComplete="tel"
                defaultValue={defaultMsisdn ?? ""}
                aria-invalid={problem !== null}
                aria-describedby={
                  problem === null ? hintId : `${hintId} ${errorId}`
                }
              />
              <p id={hintId} className="text-caption text-muted-foreground">
                {t("msisdn.hint")}
              </p>
              {problem === null ? null : (
                // Not `<FieldError>`: its own `{children, className}` type
                // has no `data-testid` and no `id`, and
                // `checkout-view.test.tsx` queries this exact element by
                // testid and then reads its `role` off the SAME node — they
                // must be one element, not a wrapper around one. `id`,
                // `role` and `data-testid` therefore live on this plain
                // `<div>`; `InlineBanner` carries the colour instead of a
                // hand-copied danger-hue text token (see the file header),
                // and it is nested one level in because it forwards neither
                // `id` nor `data-testid` of its own.
                <div id={errorId} role="alert" data-testid="msisdn-problem">
                  <InlineBanner variant="danger">{t(problem)}</InlineBanner>
                </div>
              )}
            </FormField>
            <MemoryOptIn
              t={t}
              controls={memory}
              label={t("memory.remember_number")}
            />
            <Button type="submit" className="w-full">
              {t("msisdn.submit", { amount })}
            </Button>
            {canGoBack ? (
              <Button type="button" variant="ghost" size="sm" onClick={onBack}>
                {t("msisdn.back")}
              </Button>
            ) : null}
          </div>
        </form>
      </div>
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
      <div className="flex flex-col gap-4">
        <ScreenHeading screen="ready_redirect">{t(rail.label)}</ScreenHeading>
        <p className="text-muted-foreground">{t("state.redirecting_body")}</p>
        {problem === null ? null : (
          // `data-testid` moved onto this wrapping `<div>`, 2026-09-12:
          // `InlineBanner` forwards neither `id` nor `data-testid` of its
          // own (verified `inline-banner.js` — it destructures only
          // `{variant, children, className}`), so the attribute written
          // directly on it was silently dropped and never reached the DOM.
          // Nothing queries it today, which is how that went unnoticed.
          <div role="alert" data-testid="redirect-problem">
            <InlineBanner variant="danger">{t(problem)}</InlineBanner>
          </div>
        )}
        <MemoryOptIn
          t={t}
          controls={memory}
          label={t("memory.remember_method", { rail: t(rail.label) })}
        />
        <Button
          type="button"
          className="w-full"
          data-testid="continue"
          onClick={onContinue}
        >
          {t("msisdn.submit", { amount })}
        </Button>
        {canGoBack ? (
          <Button type="button" variant="ghost" size="sm" onClick={onBack}>
            {t("msisdn.back")}
          </Button>
        ) : null}
      </div>
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
      <div className="flex flex-col gap-4">
        <ScreenHeading screen={screen}>{title}</ScreenHeading>
        {body === null ? null : <p className="text-muted-foreground">{body}</p>}
        <Spinner />
        {notice ? (
          <div className="flex flex-col items-start gap-2">
            <p className="font-semibold" data-testid="poll-notice">
              {t(notice)}
            </p>
            {onRetry === undefined ? null : (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={onRetry}
              >
                {t("error.retry")}
              </Button>
            )}
          </div>
        ) : null}
      </div>
    </section>
  );
}

/**
 * Outcome tone comes from `@vpay/tokens`, never from a colour written here
 * (AGENTS.md) — and from `checkoutOutcomeVariant` rather than from
 * `statusTone` or (as of 2026-09-12) `checkoutOutcomeTone`.
 *
 * `checkoutOutcomeTone` mapped each outcome onto daisyUI's semantic tone
 * vocabulary (`"error"|"warning"|"success"`), which `@vpay/ui`'s `Alert`
 * consumed directly. `@vaam-apps/ui`'s `InlineBanner` speaks a DIFFERENT
 * vocabulary (`"danger"|"warning"|"success"|…`) with no `"error"` at all,
 * so `checkoutOutcomeVariant` — added alongside `checkoutOutcomeTone`,
 * expressing the same D4 decision in `InlineBanner`'s own variant names,
 * with a test asserting the two tables cannot drift — is what this file
 * passes to it now. The underlying decision (a payer's own cancellation
 * reads less alarming than a payment that failed for a reason outside
 * their control) is unchanged; only the vocabulary moved.
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
      <div className="flex flex-col gap-4">
        <ScreenHeading screen="outcome">{title}</ScreenHeading>
        <div role="status">
          <InlineBanner variant={checkoutOutcomeVariant[kind]}>
            <span data-testid="outcome-body">{body}</span>
          </InlineBanner>
        </div>
        {reason === null ? null : (
          <p
            className="text-caption text-muted-foreground"
            data-testid="provider-reason"
          >
            <span className="font-semibold">
              {t("outcome.provider_said")}:{" "}
            </span>
            {reason}
          </p>
        )}
        {destination === null ? (
          <p className="text-muted-foreground" data-testid="no-destination">
            {t("outcome.no_destination")}
          </p>
        ) : (
          <Button type="button" className="w-full" onClick={onBack}>
            {merchantLine(
              t,
              merchant,
              "outcome.back_to",
              "outcome.back_to_unnamed",
            )}
          </Button>
        )}
      </div>
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
      <div className="flex flex-col gap-4">
        <ScreenHeading screen={screen}>{title}</ScreenHeading>
        {/*
          `role="alert"` restores what `@vpay/ui`'s `Alert` used to default
          to with no `role` prop written here at all. `InlineBanner` sets no
          role of its own (verified `inline-banner.js` — a plain `<div>`),
          so the default has to be written down explicitly now.
        */}
        <div role="alert">
          <InlineBanner variant="warning">
            <span data-testid="notice-body">{body}</span>
          </InlineBanner>
        </div>
        <span className="sr-only">{t("error.title")}</span>
      </div>
    </section>
  );
}
