import { describe, expect, it } from 'vitest';

import { normaliseOrigin, originIsAllowed } from './csrf';

const PUBLIC = 'https://dash.example';

describe('the origin check in front of every server action', () => {
  it('admits the configured public origin', () => {
    expect(originIsAllowed(PUBLIC, PUBLIC, 'dash.example')).toBe(true);
  });

  it('refuses another origin however the request was addressed', () => {
    expect(originIsAllowed('https://evil.example', PUBLIC, 'dash.example')).toBe(false);
  });

  /**
   * **The case issue #88 item 4 exists for.**
   *
   * Next's own check (`app-render/action-handler`) compares `Origin` against
   * `x-forwarded-host` when that header is present, falling back to `Host` —
   * two values the caller sets, which agree whenever the caller wants them
   * to. Behind a proxy that does not strip incoming forwarding headers, a
   * caller sending `X-Forwarded-Host: evil.example` and
   * `Origin: https://evil.example` passes it.
   *
   * THE DECISIVE MUTATION: give `originIsAllowed` an `x-forwarded-host`
   * parameter and prefer it over `publicOrigin`. This case reads `true`.
   * There is deliberately no such parameter, which is why the mutation is a
   * signature change and not a one-character edit.
   */
  it('cannot be satisfied by a forwarded host, because it never sees one', () => {
    // Everything an attacker controls, agreeing with itself.
    expect(originIsAllowed('https://evil.example', PUBLIC, 'evil.example')).toBe(false);
  });

  it('refuses a request with no Origin at all', () => {
    // Fail closed: the check must not be removable by removing a header.
    expect(originIsAllowed(null, PUBLIC, 'dash.example')).toBe(false);
    expect(originIsAllowed('', PUBLIC, 'dash.example')).toBe(false);
  });

  it('refuses the literal `null` origin a sandboxed document sends', () => {
    expect(originIsAllowed('null', PUBLIC, 'dash.example')).toBe(false);
  });

  it('ignores case and a trailing slash on the configured value', () => {
    expect(originIsAllowed('https://dash.example', 'https://Dash.Example/', 'dash.example')).toBe(
      true,
    );
  });

  it('treats a different port as a different origin', () => {
    expect(originIsAllowed('https://dash.example:8443', PUBLIC, 'dash.example')).toBe(false);
  });

  it('treats a different scheme as a different origin', () => {
    expect(originIsAllowed('http://dash.example', PUBLIC, 'dash.example')).toBe(false);
  });

  describe('with no configured public origin', () => {
    it('falls back to the Host header — and only to Host', () => {
      expect(originIsAllowed('http://localhost:3000', null, 'localhost:3000')).toBe(true);
      expect(originIsAllowed('http://localhost:3000', null, 'localhost:3001')).toBe(false);
    });

    it('still refuses a request with no Host and no configured origin', () => {
      expect(originIsAllowed('http://localhost:3000', null, null)).toBe(false);
    });

    it('is not weakened by a blank configured value', () => {
      // A `VPAY_DASHBOARD_PUBLIC_ORIGIN=` line in a compose file is a
      // variable somebody meant to set; treating it as "allow anything"
      // would make the mistake invisible.
      expect(originIsAllowed('https://evil.example', '   ', 'dash.example')).toBe(false);
    });

    it('does not treat an unparseable configured value as permission', () => {
      expect(originIsAllowed('https://evil.example', 'dash.example', 'evil.example')).toBe(true);
      expect(originIsAllowed('https://evil.example', 'dash.example', 'dash.example')).toBe(false);
    });
  });
});

describe('origin normalisation', () => {
  it('keeps scheme, host and port and drops everything else', () => {
    expect(normaliseOrigin('https://Dash.Example:443/some/path?q=1')).toBe(
      'https://dash.example',
    );
    expect(normaliseOrigin('http://dash.example:3000/')).toBe('http://dash.example:3000');
  });

  it('answers null for anything that is not an absolute http(s) URL', () => {
    for (const value of [null, undefined, '', '   ', 'null', 'dash.example', 'file:///x', 'javascript:alert(1)']) {
      expect(normaliseOrigin(value)).toBeNull();
    }
  });
});
