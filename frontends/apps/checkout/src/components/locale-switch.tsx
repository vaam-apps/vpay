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
'use client';

import { LOCALES, type Locale, type Translate } from '../i18n/index';

export function LocaleSwitch({
  t,
  locale,
  onChange,
}: {
  t: Translate;
  locale: Locale;
  onChange: (locale: Locale) => void;
}) {
  const id = 'vpay-locale';
  return (
    <div className="flex items-center gap-2">
      <label className="text-sm opacity-70" htmlFor={id}>
        {t('locale.label')}
      </label>
      <select
        id={id}
        className="select select-bordered select-sm"
        value={locale}
        onChange={(event) => onChange(event.target.value as Locale)}
      >
        {LOCALES.map((candidate) => (
          <option key={candidate} value={candidate}>
            {t(candidate === 'fr' ? 'locale.fr' : 'locale.en')}
          </option>
        ))}
      </select>
    </div>
  );
}
