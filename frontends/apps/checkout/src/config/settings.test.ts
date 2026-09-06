/**
 * The two YAML files, every way they can be wrong.
 *
 * The property every case here is about is the one the feature stands on:
 * **a bad file costs exactly the key that is bad, and never the page.**
 */
import { describe, expect, it } from 'vitest';

import {
  DEFAULT_BRANDING,
  DEFAULT_CHECKOUT_SETTINGS,
  assembleRuntimeConfig,
  parseBranding,
  parseCheckoutSettings,
} from './settings';

describe('branding.yaml', () => {
  it('reads a complete file', () => {
    const { value, problems } = parseBranding(`
display_name: Vaam Payments
logo_url: https://cdn.example/logo.svg
primary_color: "#F3C623"
support_contact: support@vaam.example / +237 6 71 23 45 67
`);
    expect(problems).toEqual([]);
    expect(value).toEqual({
      displayName: 'Vaam Payments',
      logoUrl: 'https://cdn.example/logo.svg',
      // Lower-cased on the way in, so two spellings of one colour are one value.
      primaryColor: '#f3c623',
      supportContact: 'support@vaam.example / +237 6 71 23 45 67',
    });
  });

  it('takes a missing file as defaults and says nothing — that is the caller’s line', () => {
    expect(parseBranding(null)).toEqual({ value: DEFAULT_BRANDING, problems: [] });
  });

  it('takes an empty document as "mounted, nothing overridden"', () => {
    expect(parseBranding('')).toEqual({ value: DEFAULT_BRANDING, problems: [] });
    expect(parseBranding('# only a comment\n')).toEqual({
      value: DEFAULT_BRANDING,
      problems: [],
    });
  });

  it('falls back to defaults, loudly, on a document that is not YAML', () => {
    const { value, problems } = parseBranding('display_name: [unclosed\n');
    expect(value).toEqual(DEFAULT_BRANDING);
    expect(problems).toHaveLength(1);
    expect(problems[0]).toContain('not valid YAML');
  });

  it('falls back to defaults on a top level that is not a mapping', () => {
    for (const source of ['- one\n- two\n', 'just a string\n', '42\n']) {
      const { value, problems } = parseBranding(source);
      expect(value).toEqual(DEFAULT_BRANDING);
      expect(problems.join(' ')).toContain('top level is not a mapping');
    }
  });

  it('drops one bad key and keeps every good one', () => {
    const { value, problems } = parseBranding(`
display_name: Vaam Payments
logo_url: javascript:alert(1)
primary_color: bumblebee
support_contact: "   "
`);
    // The whole point of the feature: a typo in a colour does not cost the
    // operator's name, and it certainly does not cost the payment page.
    expect(value.displayName).toBe('Vaam Payments');
    expect(value.logoUrl).toBeNull();
    expect(value.primaryColor).toBeNull();
    expect(value.supportContact).toBeNull();
    expect(problems).toHaveLength(3);
    expect(problems.join(' ')).toContain('logo_url');
    expect(problems.join(' ')).toContain('primary_color');
    expect(problems.join(' ')).toContain('support_contact');
  });

  it('refuses a logo URL that is not absolute http or https', () => {
    for (const url of ['/logo.svg', 'data:image/svg+xml;base64,AAA', 'ftp://x/y.png', '//cdn/x.png']) {
      const { value, problems } = parseBranding(`logo_url: ${JSON.stringify(url)}\n`);
      expect(value.logoUrl, url).toBeNull();
      expect(problems.length, url).toBe(1);
    }
  });

  it('refuses a display name past the same 80-character bound the API puts on a merchant’s', () => {
    const eighty = 'a'.repeat(80);
    expect(parseBranding(`display_name: ${eighty}\n`).value.displayName).toBe(eighty);
    const { value, problems } = parseBranding(`display_name: ${'a'.repeat(81)}\n`);
    expect(value.displayName).toBeNull();
    expect(problems.join(' ')).toContain('80 characters');
  });

  it('counts characters, not UTF-16 units, so an emoji is one', () => {
    // 80 astral characters is 160 UTF-16 units. A byte or unit bound would
    // refuse a name that fits on the screen exactly as well as any other.
    expect(parseBranding(`display_name: "${'\u{1f41d}'.repeat(80)}"\n`).value.displayName).not.toBeNull();
    expect(parseBranding(`display_name: "${'\u{1f41d}'.repeat(81)}"\n`).value.displayName).toBeNull();
  });

  it('ignores a value of the wrong YAML type rather than stringifying it', () => {
    const { value, problems } = parseBranding('display_name: 42\nlogo_url: true\n');
    expect(value.displayName).toBeNull();
    expect(value.logoUrl).toBeNull();
    expect(problems).toHaveLength(2);
  });

  it('names the file it was reading, so a log line says which mount is wrong', () => {
    const { problems } = parseBranding('display_name: ""\n', '/etc/vpay/checkout/branding.yaml');
    expect(problems[0]).toContain('/etc/vpay/checkout/branding.yaml');
  });
});

describe('config.yaml', () => {
  it('reads a complete checkout section', () => {
    const { value, problems } = parseCheckoutSettings(`
checkout:
  public_base_url: https://pay.example
  allowed_methods:
    - mtn_momo
    - orange_money
  features:
    page_memory: false
`);
    expect(problems).toEqual([]);
    expect(value).toEqual({
      publicBaseUrl: 'https://pay.example',
      allowedMethods: ['mtn_momo', 'orange_money'],
      features: { pageMemory: false },
    });
  });

  it('defaults page memory ON, because the payer’s own opt-in is the second lock', () => {
    expect(parseCheckoutSettings(null).value.features.pageMemory).toBe(true);
    expect(DEFAULT_CHECKOUT_SETTINGS.features.pageMemory).toBe(true);
  });

  it('offers every rail when the file names none', () => {
    // `null`, not `[]`: "no opinion" and "nothing is allowed" are opposite
    // answers and the page acts on the difference.
    expect(parseCheckoutSettings(null).value.allowedMethods).toBeNull();
  });

  it('says so when the document has no checkout section at all', () => {
    const { value, problems } = parseCheckoutSettings('dashboard:\n  enabled: true\n');
    expect(value).toEqual(DEFAULT_CHECKOUT_SETTINGS);
    expect(problems.join(' ')).toContain('no checkout: section');
  });

  it('refuses an empty allow-list rather than refusing every payment', () => {
    const { value, problems } = parseCheckoutSettings('checkout:\n  allowed_methods: []\n');
    expect(value.allowedMethods).toBeNull();
    expect(problems.join(' ')).toContain('empty');
  });

  it('refuses the whole list when one entry is not a rail code', () => {
    // Not "drop the bad one and keep the rest": a list with a typo in it is a
    // list whose author meant something this file cannot work out, and
    // silently narrowing the rails a payer is offered is the wrong way to
    // guess.
    for (const source of [
      'checkout:\n  allowed_methods: [mtn_momo, 7]\n',
      'checkout:\n  allowed_methods: [mtn_momo, "MTN MoMo"]\n',
      'checkout:\n  allowed_methods: mtn_momo\n',
    ]) {
      const { value, problems } = parseCheckoutSettings(source);
      expect(value.allowedMethods, source).toBeNull();
      expect(problems.length, source).toBe(1);
    }
  });

  it('drops a duplicate rather than offering one rail twice', () => {
    expect(
      parseCheckoutSettings('checkout:\n  allowed_methods: [mtn_momo, mtn_momo]\n').value
        .allowedMethods,
    ).toEqual(['mtn_momo']);
  });

  it('keeps the default when page_memory is not a boolean', () => {
    const { value, problems } = parseCheckoutSettings(
      'checkout:\n  features:\n    page_memory: "no"\n',
    );
    expect(value.features.pageMemory).toBe(true);
    expect(problems.join(' ')).toContain('page_memory');
  });

  it('refuses `off`, because YAML 1.2 makes that the STRING "off"', () => {
    // Worth a case rather than a comment: `off`/`on`/`yes`/`no` are booleans
    // in YAML 1.1 and plain strings in the 1.2 core schema the `yaml`
    // package parses. An operator who writes `page_memory: off` and is
    // silently given `true` would have turned a privacy feature ON while
    // believing they turned it off, so the value is refused loudly and the
    // documented spelling is `false`.
    const { value, problems } = parseCheckoutSettings(
      'checkout:\n  features:\n    page_memory: off\n',
    );
    expect(value.features.pageMemory).toBe(true);
    expect(problems.join(' ')).toContain('page_memory is not true or false');
  });

  it('reads `false`, the documented spelling', () => {
    expect(
      parseCheckoutSettings('checkout:\n  features:\n    page_memory: false\n').value.features
        .pageMemory,
    ).toBe(false);
  });

  it('refuses a public_base_url that is not an absolute http(s) URL', () => {
    const { value, problems } = parseCheckoutSettings('checkout:\n  public_base_url: pay.example\n');
    expect(value.publicBaseUrl).toBeNull();
    expect(problems.join(' ')).toContain('public_base_url');
  });
});

describe('assembleRuntimeConfig', () => {
  it('carries both files’ problems in one list, in file order', () => {
    const { value, problems } = assembleRuntimeConfig({
      brandingText: 'primary_color: nope\n',
      brandingFile: 'branding.yaml',
      configText: 'checkout:\n  allowed_methods: []\n',
      configFile: 'config.yaml',
    });
    expect(value.branding.primaryColor).toBeNull();
    expect(value.checkout.allowedMethods).toBeNull();
    expect(problems).toHaveLength(2);
    expect(problems[0]).toContain('branding.yaml');
    expect(problems[1]).toContain('config.yaml');
  });

  it('serves the documented defaults when neither file is mounted', () => {
    const { value, problems } = assembleRuntimeConfig({
      brandingText: null,
      brandingFile: 'branding.yaml',
      configText: null,
      configFile: 'config.yaml',
    });
    expect(problems).toEqual([]);
    expect(value).toEqual({
      branding: DEFAULT_BRANDING,
      checkout: DEFAULT_CHECKOUT_SETTINGS,
    });
  });
});
