/**
 * The language switch.
 *
 * **Client-side only, and that is a security requirement rather than a
 * preference.** A link to `?lang=fr` has no fragment, and resolving a
 * fragment-less relative URL drops the current one — which on this page is
 * the session's `client_secret` (D6). Switching language would silently
 * destroy the credential the page is holding. So the server picks the
 * initial locale from `Accept-Language`, and this control swaps the
 * dictionary in place, updating `document.documentElement.lang` so assistive
 * technology follows.
 *
 * **Native `<select>`, not `@vaam-apps/ui`'s `Select` (2026-09-12, decision
 * 8) — forced, not preferred.** `SelectTrigger` destructures
 * `{id, className, children}` and spreads nothing else (verified
 * `select.js`), so `aria-labelledby` cannot reach the rendered element, and
 * `Select` itself takes no `name`/`id`/hidden input at all — it does not
 * participate in a form and cannot be named externally, the same finding
 * `dashui-plan-screens.md` §3.1 makes for the dashboard's status filter.
 * `checkout-view.test.tsx`'s "names the language switch on the screen, not
 * only to a screen reader" test exists BECAUSE a regression already
 * happened once (exp26): a caller removed the visible label and named the
 * control with `aria-label` instead, which `getByLabelText` passes against
 * just as happily. A native `<select>` with a real `<label htmlFor>` keeps
 * `getByLabelText` working, keeps the word on screen, and makes
 * `aria-labelledby` unnecessary — `<label for>` is the stronger mechanism —
 * so there is no `role="combobox"` question to answer either: a native,
 * unmultiplied `<select>` already carries that role implicitly.
 *
 * `<Label>` is `@vaam-apps/ui`'s own — it renders a real `<label>` and
 * spreads `htmlFor` straight onto it (verified `label.js`), so this is a
 * genuine component, not a hand-rolled substitute. The `<select>` itself
 * has no `@vaam-apps/ui` counterpart at all; `select` is daisyUI's own class
 * for a native select, kept here so the one control on this page with no
 * component still matches the theme. **The daisyUI 4 border modifier for
 * this control is deliberately not applied (2026-09-12):** daisyUI 5 does
 * not define it — the base class carries a border by default now — and
 * `verify-ui` check 2 refuses that removed class in an app for exactly this
 * reason (spelled out in full in that check's own comment, not repeated
 * here on purpose — this file is itself inside the pathspec that check
 * greps).
 */
"use client";

import { Label } from "@vaam-apps/ui";

import { LOCALES, type Locale, type Translate } from "../i18n/index";

export function LocaleSwitch({
  t,
  locale,
  onChange,
}: {
  t: Translate;
  locale: Locale;
  onChange: (locale: Locale) => void;
}) {
  const id = "vpay-locale";
  return (
    <div className="flex flex-col gap-1">
      <Label htmlFor={id}>{t("locale.label")}</Label>
      <select
        id={id}
        name="locale"
        className="select select-sm"
        value={locale}
        onChange={(event) => onChange(event.target.value as Locale)}
      >
        {LOCALES.map((candidate) => (
          <option key={candidate} value={candidate}>
            {t(candidate === "fr" ? "locale.fr" : "locale.en")}
          </option>
        ))}
      </select>
    </div>
  );
}
