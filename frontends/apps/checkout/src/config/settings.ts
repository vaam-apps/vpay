/**
 * The two YAML files an operator mounts into the checkout container, as
 * types and as pure parsers.
 *
 * **Runtime, not build time.** `next build` produces one image; a
 * deployment's brand and its feature flags arrive as files read after the
 * process starts (`runtime.ts`). Nothing here reads the filesystem or the
 * environment, so every rule below — including every way a file can be
 * wrong — is a unit test rather than a container someone has to run.
 *
 * **A bad file never blanks the page.** Every parser answers a *complete*
 * settings object plus a list of problems: an unreadable file, a malformed
 * document, a key of the wrong type and a value that fails its rule each
 * cost exactly that key and leave the rest standing. A payment page that
 * refused to render because a logo URL had a typo would be a worse failure
 * than a page with no logo.
 *
 * The problems are for the operator, in the container log. They are not
 * shown to a payer and they are not on the JSON endpoint: a payer cannot
 * fix a mounted file, and the paths in them are the operator's business.
 */
import { parse as parseYaml } from 'yaml';

/** The look of the page: who is serving it, and in what colours. */
export interface Branding {
  /**
   * The **operator's** name — whose deployment of vpay this is — shown in
   * the page header.
   *
   * It is deliberately **not** the merchant's. What a payer is told they
   * are paying comes from the session (`merchant: { name }`, set per
   * merchant in the API's own YAML), and substituting the operator's name
   * there would tell a payer they were paying the wrong party. Where the
   * session carries no merchant name the page still says nothing about who
   * is being paid — see `page.pay_to_unnamed`.
   */
  displayName: string | null;
  /** Absolute `http`/`https` only. Anything else is dropped with a problem. */
  logoUrl: string | null;
  /** `#rrggbb`, converted to a daisyUI theme override by `theme.ts`. */
  primaryColor: string | null;
  /** Free text — an address, a phone number, a URL. Rendered as data, never as a link target. */
  supportContact: string | null;
}

/** Feature flags. One today. */
export interface CheckoutFeatures {
  /**
   * Whether the page offers to remember a payer's number and last method on
   * their own device (`memory.ts`).
   *
   * `false` removes the offer entirely — no checkbox, no read, no write —
   * rather than hiding a control that still stores. Default `true`: the
   * store is opt-in per payer on top of this, so the flag being on means
   * only that the offer is made.
   */
  pageMemory: boolean;
}

/** The page's own operational settings. */
export interface CheckoutSettings {
  /**
   * The origin (optionally with a path prefix) payers reach this page on —
   * the same value as the API's `checkout.public_base_url`.
   *
   * **It is a diagnostic here and nothing else.** The API mints every payer
   * link, so this container never builds one; what it does with the value is
   * report it on `/config/v1` and log one line when the origin a browser
   * actually loaded the page from disagrees with it, which is the first
   * symptom of a proxy in front of this container that nobody told the API
   * about. No rendering and no navigation depends on it.
   */
  publicBaseUrl: string | null;
  /**
   * The rail codes this deployment will show, or `null` for "no opinion".
   *
   * It can only ever **narrow** what a payment offers: the intent's own
   * `payment_method_types` is the list the merchant chose and the server
   * validated, and a code here that the intent does not carry adds nothing.
   * A rail an intent offers and this list excludes is shown to the payer as
   * unsupported, exactly like a rail the page has no flow for — never
   * silently dropped.
   *
   * An **empty list is refused** (defaults restored, one problem logged): it
   * would refuse every payment on the deployment, which is a thing to say in
   * a deployment's replica count, not in a colour-and-copy file.
   */
  allowedMethods: readonly string[] | null;
  features: CheckoutFeatures;
}

/** Everything the two files together say. */
export interface RuntimeConfig {
  branding: Branding;
  checkout: CheckoutSettings;
}

/** A parsed value and everything that was wrong with the document it came from. */
export interface Parsed<T> {
  value: T;
  problems: readonly string[];
}

/** What a container with no files mounted serves. */
export const DEFAULT_BRANDING: Branding = Object.freeze({
  displayName: null,
  logoUrl: null,
  primaryColor: null,
  supportContact: null,
});

/** What a container with no files mounted serves. */
export const DEFAULT_CHECKOUT_SETTINGS: CheckoutSettings = Object.freeze({
  publicBaseUrl: null,
  allowedMethods: null,
  features: Object.freeze({ pageMemory: true }),
});

export const DEFAULT_RUNTIME_CONFIG: RuntimeConfig = Object.freeze({
  branding: DEFAULT_BRANDING,
  checkout: DEFAULT_CHECKOUT_SETTINGS,
});

/**
 * The same bound the API puts on `merchant_clients[].display_name`: 80
 * characters, a rendering bound rather than a storage one, because it is
 * painted into a heading on a phone-sized page.
 */
const MAX_DISPLAY_NAME = 80;
/** Long enough for an address plus a phone number, short enough not to be prose. */
const MAX_SUPPORT_CONTACT = 200;
const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;

/** A YAML document as a plain object, or `null` with a problem for anything else. */
function documentOf(source: string | null, file: string): Parsed<Record<string, unknown> | null> {
  if (source === null) {
    return { value: null, problems: [] };
  }
  let parsed: unknown;
  try {
    parsed = parseYaml(source);
  } catch (error) {
    const detail = error instanceof Error ? error.message : 'unparseable';
    return { value: null, problems: [`${file}: not valid YAML (${detail}) — using defaults`] };
  }
  // An empty document parses to `null`, and that is a legitimate file: it
  // says "mounted, nothing overridden". A scalar or a list is not.
  if (parsed === null || parsed === undefined) {
    return { value: null, problems: [] };
  }
  if (typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { value: null, problems: [`${file}: top level is not a mapping — using defaults`] };
  }
  return { value: parsed as Record<string, unknown>, problems: [] };
}

/** A non-blank string of at most `max` characters, or `null` with a problem. */
function stringField(
  raw: unknown,
  key: string,
  file: string,
  max: number,
  problems: string[],
): string | null {
  if (raw === undefined || raw === null) {
    return null;
  }
  if (typeof raw !== 'string') {
    problems.push(`${file}: ${key} is not a string — ignored`);
    return null;
  }
  const trimmed = raw.trim();
  if (trimmed.length === 0) {
    problems.push(`${file}: ${key} is blank — ignored`);
    return null;
  }
  // Characters, not bytes: the bound is about how much of a heading a name
  // takes up, and `[...s]` counts code points rather than UTF-16 units.
  if ([...trimmed].length > max) {
    problems.push(`${file}: ${key} is longer than ${max} characters — ignored`);
    return null;
  }
  return trimmed;
}

/**
 * An absolute `http`/`https` URL, or `null` with a problem.
 *
 * Strict on purpose. `logo_url` becomes an `<img src>` and a `data:` URI
 * there is a document this page did not write; `public_base_url` is compared
 * against a browser's own origin, which only an absolute URL can be.
 */
function urlField(raw: unknown, key: string, file: string, problems: string[]): string | null {
  const text = stringField(raw, key, file, 2048, problems);
  if (text === null) {
    return null;
  }
  let parsed: URL;
  try {
    parsed = new URL(text);
  } catch {
    problems.push(`${file}: ${key} is not an absolute URL — ignored`);
    return null;
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    problems.push(`${file}: ${key} is not http or https — ignored`);
    return null;
  }
  return text;
}

/**
 * `branding.yaml` → {@link Branding}.
 *
 * `null` for `source` means "no file there", which is not a problem: a
 * deployment that mounts nothing gets vpay's own defaults and one line in
 * the log saying so (written by `runtime.ts`, which is the layer that knows
 * whether the file was absent or unreadable).
 */
export function parseBranding(source: string | null, file = 'branding.yaml'): Parsed<Branding> {
  const document = documentOf(source, file);
  const problems = [...document.problems];
  if (document.value === null) {
    return { value: DEFAULT_BRANDING, problems };
  }
  const raw = document.value;

  const primaryRaw = stringField(raw['primary_color'], 'primary_color', file, 32, problems);
  let primaryColor: string | null = null;
  if (primaryRaw !== null) {
    if (HEX_COLOR.test(primaryRaw)) {
      primaryColor = primaryRaw.toLowerCase();
    } else {
      problems.push(`${file}: primary_color is not a #rrggbb hex colour — ignored`);
    }
  }

  return {
    value: {
      displayName: stringField(raw['display_name'], 'display_name', file, MAX_DISPLAY_NAME, problems),
      logoUrl: urlField(raw['logo_url'], 'logo_url', file, problems),
      primaryColor,
      supportContact: stringField(
        raw['support_contact'],
        'support_contact',
        file,
        MAX_SUPPORT_CONTACT,
        problems,
      ),
    },
    problems,
  };
}

/** A rail code as the wire spells one: lowercase letters, digits and underscores. */
const RAIL_CODE = /^[a-z0-9_]+$/;

/** `config.yaml`'s `checkout:` mapping → {@link CheckoutSettings}. */
export function parseCheckoutSettings(
  source: string | null,
  file = 'config.yaml',
): Parsed<CheckoutSettings> {
  const document = documentOf(source, file);
  const problems = [...document.problems];
  if (document.value === null) {
    return { value: DEFAULT_CHECKOUT_SETTINGS, problems };
  }
  const section: unknown = document.value['checkout'];
  if (section === undefined || section === null) {
    problems.push(`${file}: no checkout: section — using defaults`);
    return { value: DEFAULT_CHECKOUT_SETTINGS, problems };
  }
  if (typeof section !== 'object' || Array.isArray(section)) {
    problems.push(`${file}: checkout: is not a mapping — using defaults`);
    return { value: DEFAULT_CHECKOUT_SETTINGS, problems };
  }
  const raw = section as Record<string, unknown>;

  return {
    value: {
      publicBaseUrl: urlField(raw['public_base_url'], 'checkout.public_base_url', file, problems),
      allowedMethods: allowedMethodsField(raw['allowed_methods'], file, problems),
      features: featuresField(raw['features'], file, problems),
    },
    problems,
  };
}

function allowedMethodsField(
  raw: unknown,
  file: string,
  problems: string[],
): readonly string[] | null {
  if (raw === undefined || raw === null) {
    return null;
  }
  if (!Array.isArray(raw)) {
    problems.push(`${file}: checkout.allowed_methods is not a list — ignored`);
    return null;
  }
  const codes: string[] = [];
  for (const entry of raw) {
    if (typeof entry !== 'string' || !RAIL_CODE.test(entry)) {
      problems.push(
        `${file}: checkout.allowed_methods has an entry that is not a rail code — ignored`,
      );
      return null;
    }
    if (!codes.includes(entry)) {
      codes.push(entry);
    }
  }
  if (codes.length === 0) {
    // Refused rather than honoured. An empty allow-list is indistinguishable
    // in YAML from a list someone meant to fill in, and honouring it would
    // refuse every payment on the deployment with no error anywhere.
    problems.push(`${file}: checkout.allowed_methods is empty — ignored, every rail stays on offer`);
    return null;
  }
  return Object.freeze(codes);
}

function featuresField(raw: unknown, file: string, problems: string[]): CheckoutFeatures {
  if (raw === undefined || raw === null) {
    return DEFAULT_CHECKOUT_SETTINGS.features;
  }
  if (typeof raw !== 'object' || Array.isArray(raw)) {
    problems.push(`${file}: checkout.features is not a mapping — using defaults`);
    return DEFAULT_CHECKOUT_SETTINGS.features;
  }
  const value: unknown = (raw as Record<string, unknown>)['page_memory'];
  if (value === undefined || value === null) {
    return DEFAULT_CHECKOUT_SETTINGS.features;
  }
  if (typeof value !== 'boolean') {
    problems.push(`${file}: checkout.features.page_memory is not true or false — using the default`);
    return DEFAULT_CHECKOUT_SETTINGS.features;
  }
  return Object.freeze({ pageMemory: value });
}

/**
 * Both files at once, with every problem in one list.
 *
 * The pure half of `runtime.ts`: give it what the filesystem said and it
 * answers what the process should serve. `null` for either text means the
 * file was not there or could not be read — the caller says which, because
 * only it knows the path.
 */
export function assembleRuntimeConfig(input: {
  brandingText: string | null;
  brandingFile: string;
  configText: string | null;
  configFile: string;
}): Parsed<RuntimeConfig> {
  const branding = parseBranding(input.brandingText, input.brandingFile);
  const checkout = parseCheckoutSettings(input.configText, input.configFile);
  return {
    value: { branding: branding.value, checkout: checkout.value },
    problems: [...branding.problems, ...checkout.problems],
  };
}
