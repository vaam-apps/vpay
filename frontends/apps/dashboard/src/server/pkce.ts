/**
 * PKCE (RFC 7636) for the one authorization-code exchange this app performs.
 *
 * # Where the verifier lives, and why that is nowhere
 *
 * In a browser-driven flow the verifier has to survive a redirect, so it goes
 * into a cookie or a server-side store and every one of those is a place it
 * can leak from. Here it does not survive anything: ADR-0017 decision 4 has
 * **this app's own server** request the code, follow the `302` and exchange
 * the code, all inside one function call (`server/oauth.ts`). The verifier is
 * a local binding for the length of two `fetch` calls and is never written
 * anywhere, never sent to a browser, and never reachable from a second
 * request.
 *
 * That is the strongest form the design admits, and it is worth stating
 * plainly because the *absence* of a store reads, to someone scanning for
 * one, like the verifier having been forgotten.
 *
 * # What PKCE is doing here at all
 *
 * The dashboard is a public client — no client secret (ADR-0009). The
 * verifier is therefore the only thing that authenticates the exchange:
 * `vpay_api::staff::oauth::token` compares `S256(verifier)` against the
 * stored challenge in constant time and refuses on a mismatch, and a code
 * presented with a wrong verifier is spent anyway so a captured code cannot
 * be probed.
 */
import { createHash, randomBytes } from 'node:crypto';

/** A verifier and the challenge derived from it. */
export interface PkcePair {
  /** The high-entropy secret. Sent only in the token exchange. */
  readonly verifier: string;
  /** `BASE64URL(SHA256(verifier))`. Sent only in the authorization request. */
  readonly challenge: string;
}

/**
 * The one PKCE method this deployment serves.
 *
 * `plain` is refused at both ends — `handle_authorize` rejects it and
 * migration `0035`'s `method_is_s256` refuses the row — so naming it here
 * would be a value nothing accepts.
 */
export const CODE_CHALLENGE_METHOD = 'S256';

/**
 * How many random bytes back a verifier.
 *
 * Thirty-two, which base64url-encodes to 43 characters — RFC 7636 §4.1's
 * minimum length, and its recommendation for a verifier built this way. The
 * spec's ceiling is 128.
 */
const VERIFIER_BYTES = 32;

/** `BASE64URL-ENCODE` (RFC 7636 §A): base64 without padding, `+/` → `-_`. */
function base64url(bytes: Buffer): string {
  return bytes.toString('base64url');
}

/**
 * `BASE64URL(SHA256(ASCII(verifier)))` — RFC 7636 §4.2.
 *
 * Separate from {@link createPkce} so the RFC's own Appendix B vector can be
 * run against it: a generator that returns a fresh random value cannot be
 * pinned to a published test vector, and a challenge derivation that is
 * wrong in a way both ends share would still "work" end to end.
 */
export function challengeFor(verifier: string): string {
  return base64url(createHash('sha256').update(verifier, 'ascii').digest());
}

/**
 * A fresh verifier and its challenge.
 *
 * `randomBytes` is the OS CSPRNG. `Math.random` is not a CSPRNG and a
 * verifier drawn from one is guessable by anyone who can observe a second
 * one — which is the whole attack PKCE exists to stop.
 */
export function createPkce(): PkcePair {
  const verifier = base64url(randomBytes(VERIFIER_BYTES));
  return { verifier, challenge: challengeFor(verifier) };
}

/**
 * An opaque `state`, echoed by the authorization response and checked on the
 * way back.
 *
 * The RFC's reason for `state` is CSRF on a *browser* callback, which this
 * flow does not have — the server that generated it is the server that reads
 * it back, one `await` later. It is generated and checked anyway, because it
 * is the only thing that would catch a `Location` header belonging to some
 * other request, and the check costs one comparison.
 */
export function createState(): string {
  return base64url(randomBytes(VERIFIER_BYTES));
}
