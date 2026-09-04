/**
 * Locale selection and message formatting.
 *
 * Two locales, `fr` and `en`, both complete: `fr.ts` is typed as
 * `Record<MessageKey, string>`, so a key added to `en` and forgotten in `fr`
 * does not compile. There is no runtime fallback to English for a missing
 * key, deliberately — a fallback would let a half-translated dictionary ship
 * and read as finished.
 */
import { en, type MessageKey } from './en';
import { fr } from './fr';

export type { MessageKey };

/** The locales this page serves. Order is meaningful: the first is the default. */
export const LOCALES = ['fr', 'en'] as const;

export type Locale = (typeof LOCALES)[number];

/**
 * Cameroon first. When a browser expresses no preference between the two
 * locales this page has, it gets French — the language of the deployment
 * this repository is written for, and the one Orange's own hosted page uses.
 */
export const DEFAULT_LOCALE: Locale = 'fr';

export const DICTIONARIES: Record<Locale, Record<MessageKey, string>> = { fr, en };

export function isLocale(value: string): value is Locale {
  return (LOCALES as readonly string[]).includes(value);
}

interface LanguageRange {
  tag: string;
  quality: number;
}

/**
 * Parses an `Accept-Language` header into ranges, best first.
 *
 * Hand-rolled rather than reached for from a library because the whole job
 * is three `split`s and a sort, and because the failure mode of a
 * mis-parsed header here is a French page for an English speaker — worth
 * being able to read in one screen.
 */
function parseAcceptLanguage(header: string): LanguageRange[] {
  const ranges: LanguageRange[] = [];
  for (const part of header.split(',')) {
    const [rawTag, ...parameters] = part.trim().split(';');
    const tag = (rawTag ?? '').trim().toLowerCase();
    if (tag.length === 0) {
      continue;
    }
    let quality = 1;
    for (const parameter of parameters) {
      const [name, value] = parameter.trim().split('=');
      if (name?.trim().toLowerCase() === 'q') {
        const parsed = Number.parseFloat(value ?? '');
        // A malformed q is treated as "unstated", i.e. 1 — RFC 9110's own
        // reading. Treating it as 0 would silently drop the payer's first
        // choice because of a stray character.
        quality = Number.isFinite(parsed) ? parsed : 1;
      }
    }
    ranges.push({ tag, quality });
  }
  // Stable sort by quality descending: `Array.prototype.sort` is stable in
  // every engine this runs on, so equal-quality tags keep header order.
  return ranges.sort((a, b) => b.quality - a.quality);
}

/**
 * Picks the locale for an `Accept-Language` header.
 *
 * Matches on the primary subtag only (`fr-CM`, `fr-FR` and `fr` are all
 * French here) because the dictionaries carry no regional variants and
 * pretending otherwise would be a promise this page cannot keep. A `q=0`
 * range is an explicit refusal and is skipped.
 */
export function pickLocale(header: string | null | undefined): Locale {
  if (typeof header !== 'string' || header.trim().length === 0) {
    return DEFAULT_LOCALE;
  }
  for (const range of parseAcceptLanguage(header)) {
    if (range.quality <= 0) {
      continue;
    }
    if (range.tag === '*') {
      return DEFAULT_LOCALE;
    }
    const primary = range.tag.split('-')[0] ?? '';
    if (isLocale(primary)) {
      return primary;
    }
  }
  return DEFAULT_LOCALE;
}

/**
 * Substitutes `{name}` placeholders.
 *
 * A placeholder with no value is left verbatim rather than replaced with an
 * empty string: `Pay {merchant}` on screen is a visible bug report, whereas
 * `Pay ` reads like finished copy with a missing name.
 */
export function format(template: string, values: Record<string, string | number> = {}): string {
  let out = '';
  let index = 0;
  for (;;) {
    const open = template.indexOf('{', index);
    if (open === -1) {
      out += template.slice(index);
      return out;
    }
    const close = template.indexOf('}', open);
    if (close === -1) {
      out += template.slice(index);
      return out;
    }
    const name = template.slice(open + 1, close);
    const value = values[name];
    out += template.slice(index, open);
    out += value === undefined ? template.slice(open, close + 1) : String(value);
    index = close + 1;
  }
}

/** The message lookup a component receives. Bound to one locale. */
export type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

export function translator(locale: Locale): Translate {
  const dictionary = DICTIONARIES[locale];
  return (key, values) => format(dictionary[key], values);
}

/** Placeholder names used by a template, in order of first appearance. */
export function placeholdersOf(template: string): string[] {
  const names: string[] = [];
  let index = 0;
  for (;;) {
    const open = template.indexOf('{', index);
    if (open === -1) {
      return names;
    }
    const close = template.indexOf('}', open);
    if (close === -1) {
      return names;
    }
    const name = template.slice(open + 1, close);
    if (!names.includes(name)) {
      names.push(name);
    }
    index = close + 1;
  }
}

export { en, fr };
