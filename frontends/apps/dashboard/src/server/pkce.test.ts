/**
 * PKCE, against RFC 7636's own vector.
 *
 * The derivation is pinned to a published pair rather than to itself: a
 * `challengeFor` that hashed the wrong encoding would still "work" end to
 * end, because vpay only ever compares this app's challenge with this app's
 * verifier — the mistake would be invisible until a second client existed.
 */
import { describe, expect, it } from 'vitest';

import { CODE_CHALLENGE_METHOD, challengeFor, createPkce, createState } from './pkce';

/** RFC 7636 Appendix B. */
const RFC_VERIFIER = 'dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk';
const RFC_CHALLENGE = 'E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM';

describe('the code challenge', () => {
  it('is BASE64URL(SHA256(ASCII(verifier))) — RFC 7636 Appendix B', () => {
    expect(challengeFor(RFC_VERIFIER)).toBe(RFC_CHALLENGE);
  });

  it('carries no base64 padding and no + or /', () => {
    // base64url, not base64. A `+` or a `/` in a query parameter is a
    // different string by the time it reaches the server.
    expect(challengeFor(RFC_VERIFIER)).not.toMatch(/[+/=]/);
  });

  it('is S256 and nothing else', () => {
    // `plain` is refused at both ends — handle_authorize rejects it and
    // migration 0035's method_is_s256 refuses the row.
    expect(CODE_CHALLENGE_METHOD).toBe('S256');
  });
});

describe('a generated verifier', () => {
  it('is 43 characters — the RFC minimum for 32 random bytes', () => {
    expect(createPkce().verifier).toHaveLength(43);
  });

  it('is inside the RFC 7636 §4.1 character set', () => {
    expect(createPkce().verifier).toMatch(/^[A-Za-z0-9\-._~]+$/);
  });

  it('carries the challenge derived from that exact verifier', () => {
    const pair = createPkce();
    expect(pair.challenge).toBe(challengeFor(pair.verifier));
  });

  it('is different every time', () => {
    // Not a strength test — it cannot be — but a generator that returned a
    // constant would pass every other assertion in this file.
    const values = new Set(Array.from({ length: 32 }, () => createPkce().verifier));
    expect(values.size).toBe(32);
  });
});

describe('state', () => {
  it('is different every time', () => {
    const values = new Set(Array.from({ length: 32 }, () => createState()));
    expect(values.size).toBe(32);
  });
});
