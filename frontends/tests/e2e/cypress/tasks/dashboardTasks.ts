/**
 * Node-side helpers for `dashboard.cy.ts`: the staff member's one-time
 * password, and the TOTP codes the spec has to produce to get past the second
 * factor.
 *
 * Both run outside the browser, and both have to.
 *
 * **The password** is written by `just demo-staff` to a git-ignored file
 * under `.e2e/`. The spec is given the *path* through an environment variable
 * and never the value, so the credential is not in `Cypress.env`, not in a
 * `cypress run` argument list, and not in the runner's own log — the same
 * rule `checkoutTasks.ts` follows for the merchant private key.
 *
 * **The codes** are RFC 6238, HMAC-SHA1 over a counter, which needs a keyed
 * hash. `node:crypto` has one; doing it in the page would mean either
 * `SubtleCrypto` gymnastics inside a spec or a dependency in the browser
 * bundle, and neither buys anything — nothing about this computation is a
 * thing the *browser* has to be able to do. What matters is that the code is
 * computed from the secret THE SCREEN SHOWED, so the spec proves an
 * authenticator app enrolled from that QR would work, rather than proving
 * that a secret the test also knows verifies against itself.
 *
 * Nothing here is a stub of vpay. Every value is either read from a file vpay
 * wrote or computed from a secret vpay minted, and the assertion at the far
 * end is vpay's own `401` or its own payments list.
 */
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";

/** RFC 6238's defaults, which are also `vpay_api::staff_auth::totp`'s. */
const DIGITS = 6;
const STEP_SECONDS = 30;

/**
 * The one-time password `vpay-server staff add` printed, from the file
 * `just demo-staff` wrote it to.
 *
 * Throws with the recipe to run rather than returning an empty string: a
 * spec that signed in with `""` would fail at the password step and read as
 * "the credential was rejected", which is the one thing that is not wrong.
 */
export function staffPassword(): string {
  const path = process.env["VPAY_STAFF_PASSWORD_FILE"];
  if (typeof path !== "string" || path.length === 0) {
    throw new Error(
      "dashboard.cy.ts: VPAY_STAFF_PASSWORD_FILE is not set. `just test-e2e` sets it; " +
        "running `cypress run` by hand needs `just demo-staff` first and the variable " +
        "pointed at the file it writes.",
    );
  }
  let password: string;
  try {
    password = readFileSync(path, "utf8").trim();
  } catch (cause) {
    throw new Error(
      `dashboard.cy.ts: cannot read the staff one-time password at ${path}. ` +
        "Run `just demo-staff` against the running stack.",
      { cause },
    );
  }
  if (password.length === 0) {
    throw new Error(
      `dashboard.cy.ts: the staff password file at ${path} is empty. ` +
        "`staff add` prints the password once; tear the stack down (`just demo-down`) " +
        "and start again.",
    );
  }
  return password;
}

/** RFC 4648 base32 decoding, which is the alphabet `otpauth://` secrets use. */
const BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

function base32Decode(encoded: string): Buffer {
  const clean = encoded.replace(/=+$/, "").toUpperCase();
  let bits = 0;
  let value = 0;
  const out: number[] = [];
  for (const character of clean) {
    const index = BASE32_ALPHABET.indexOf(character);
    if (index < 0) {
      throw new Error(`dashboard.cy.ts: '${character}' is not RFC 4648 base32`);
    }
    value = (value << 5) | index;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out.push((value >>> bits) & 0xff);
    }
  }
  return Buffer.from(out);
}

/** What {@link totpCode} takes. */
export interface TotpRequest {
  /** The base32 secret, exactly as the enrolment screen displayed it. */
  secret: string;
  /**
   * How many 30-second steps away from now to compute.
   *
   * `0` is the current step. The spec uses `-1` for the one case that needs a
   * *different* valid code: vpay's replay guard is a compare-and-swap on
   * `staff_members.last_totp_step` that admits only a strictly greater step,
   * so a second sign-in inside the same 30 seconds cannot present the same
   * code — and would be refused for the right reason with a message that
   * looks exactly like a wrong one.
   */
  skew?: number;
}

/**
 * An RFC 6238 code for `secret`.
 *
 * HMAC-SHA1 over the big-endian counter, dynamic truncation, modulo 10^6 —
 * the algorithm `vpay_api::staff_auth::totp` implements and pins against RFC
 * 6238 Appendix B's own vectors on its side.
 */
export function totpCode({ secret, skew = 0 }: TotpRequest): string {
  const key = base32Decode(secret);
  const step = Math.floor(Date.now() / 1000 / STEP_SECONDS) + skew;

  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(step));

  const digest = createHmac("sha1", key).update(counter).digest();
  // Dynamic truncation, RFC 4226 §5.4: the low nibble of the last byte is an
  // offset into the digest.
  const offset = (digest[digest.length - 1] ?? 0) & 0x0f;
  const binary =
    (((digest[offset] ?? 0) & 0x7f) << 24) |
    (((digest[offset + 1] ?? 0) & 0xff) << 16) |
    (((digest[offset + 2] ?? 0) & 0xff) << 8) |
    ((digest[offset + 3] ?? 0) & 0xff);

  return String(binary % 10 ** DIGITS).padStart(DIGITS, "0");
}

/**
 * How long is left in the current 30-second step.
 *
 * The spec waits this out before presenting a *second* code, so that the step
 * it computes is strictly greater than the one the replay guard recorded.
 */
export function secondsLeftInStep(): number {
  return STEP_SECONDS - (Math.floor(Date.now() / 1000) % STEP_SECONDS);
}

/**
 * When the `/dash/v1` access token on a staff session expires, as vpay
 * reports it.
 *
 * `GET /dash/v1/staff/session`, called from Node with the session token the
 * browser is holding. Three reasons it is a task and not something the spec
 * does itself:
 *
 * * `access_token_expires_at` is the only observable that separates "the token
 *   was replaced **before** it expired" from "a read failed and the reactive
 *   retry in `dash-read.ts` covered it". Both render the same page, so a
 *   browser assertion cannot tell them apart, and the margin is the thing
 *   under test;
 * * the session cookie is `httpOnly`, so a page cannot read it — `cy.getCookie`
 *   can (Cypress reads them over CDP) and hands the value here;
 * * the route is on vpay's origin, not the dashboard's, so a `cy.request` from
 *   the spec would be cross-origin.
 *
 * Nothing is stubbed: this is the same endpoint, the same credential and the
 * same row the app itself reads on every render.
 */
export async function staffTokenExpiry(
  sessionToken: string,
): Promise<string | null> {
  const base = process.env["VPAY_BASE_URL"];
  if (typeof base !== "string" || base.length === 0) {
    throw new Error(
      "dashboard.cy.ts: VPAY_BASE_URL is not set. `just test-e2e` sets it; running " +
        "`cypress run` by hand needs it pointed at the running vpay-server.",
    );
  }
  const response = await fetch(`${base}/dash/v1/staff/session`, {
    headers: {
      accept: "application/json",
      // `vpay_api::staff::SESSION_HEADER`.
      "x-vpay-staff-session": sessionToken,
    },
  });
  if (!response.ok) {
    throw new Error(
      `dashboard.cy.ts: GET /dash/v1/staff/session answered ${response.status}. ` +
        "The spec asked about a session it believes is signed in, so this is a real failure " +
        "rather than a missing fixture.",
    );
  }
  const body = (await response.json()) as { access_token_expires_at?: string };
  return body.access_token_expires_at ?? null;
}
