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
 */
"use client";

import { Select, Stack, Text } from "@vpay/ui";

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
  const labelId = "vpay-locale-label";
  return (
    <Stack gap="sm">
      {/*
        A VISIBLE label, `aria-labelledby`, not `aria-label`. The exp26
        migration replaced this element with an `aria-label` on the trigger,
        which keeps the accessible name and takes the word off the screen:
        a sighted payer was left with a bare "English"/"Français" combobox
        beside the page title, with nothing saying it chooses a language.
        Plan §4.1's row for this file deletes the label's CLASSES
        (`text-sm opacity-70`), not the label. It is on the committed
        screenshots either way — `docs/plans/exp21-checkout-page-notes/
        entry-screens.png` has the word, `docs/plans/exp26-notes/lane-b/
        entry-screens.png` does not.
      */}
      <Text as="span" id={labelId} size="sm" tone="muted">
        {t("locale.label")}
      </Text>
      <Select
        id={id}
        size="sm"
        aria-labelledby={labelId}
        value={locale}
        onValueChange={(next) => onChange(next as Locale)}
        items={LOCALES.map((candidate) => ({
          value: candidate,
          label: t(candidate === "fr" ? "locale.fr" : "locale.en"),
        }))}
      />
    </Stack>
  );
}
