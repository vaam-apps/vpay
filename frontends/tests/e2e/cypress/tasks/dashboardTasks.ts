/**
 * Node-side helpers for `dashboard-human-login.cy.ts`: the staff member's
 * one-time password, the TOTP codes the spec has to produce to get past the
 * second factor, and the handful of values one of its tests leaves for a later
 * one.
 *
 * Both of the first two run outside the browser, and both have to.
 *
 * **The password** is written by `just demo-staff` to a git-ignored file under
 * `.e2e/` (mode 0600). The spec is given the *path* through an environment
 * variable and never the value, so the credential is not in `Cypress.env`, not
 * in a `cypress run` argument list, and not in the runner's own log — the same
 * rule `checkoutTasks.ts` follows for the merchant signing key.
 *
 * **The codes** are RFC 6238, HMAC-SHA1 over a counter, which needs a keyed
 * hash. `node:crypto` has one; doing it in the page would mean either
 * `SubtleCrypto` gymnastics inside a spec or a dependency in the browser
 * bundle, and neither buys anything — nothing about this computation is a
 * thing the *browser* has to be able to do. What matters is that the code is
 * computed from the secret THE SCREEN SHOWED, so the spec proves an
 * authenticator app enrolled from that QR would work, rather than proving that
 * a secret the test also knows verifies against itself.
 *
 * Nothing here is a stub of vpay. Every value is either read from a file vpay
 * wrote or computed from a secret vpay minted, and the assertion at the far end
 * is vpay's own refusal or its own account page.
 *
 * Restored from `f885170^` (where it served the pre-VP-6a `dashboard.cy.ts`)
 * and cut back to what this spec uses: `staffTokenExpiry` had no caller, and
 * the test it served (a token replaced before it expires) is not one of the
 * cases a dashboard that reads nothing yet can run.
 */
import { createHmac } from "node:crypto";
import { readFileSync } from "node:fs";

/** RFC 6238's defaults, which are also `vpay_api::staff_auth::totp`'s. */
const DIGITS = 6;
const STEP_SECONDS = 30;

/**
 * What one test in a spec leaves for a later one — held in **Node**, and that
 * is the whole point of it.
 *
 * `Cypress.env(key, value)` is the obvious place for these and the wrong one
 * for any spec that moves its primary origin: Cypress treats a differing PORT
 * as a differing origin, a test's primary origin is fixed by its first
 * `cy.visit`, and the `Cypress` object a spec talks to belongs to that
 * origin's spec bridge, so a runtime `env` write does not follow it across.
 * This spec stays on the dashboard's origin throughout, so it would survive
 * here today; it is in Node anyway because the failure it avoids was measured
 * (CI run 34555068739, 2026-09-11, on the spec this one replaces) and a spec
 * that later adds a visit to the shop must not re-learn it.
 *
 * A `Map` here survives because `cypress run` is one Node process for the
 * whole run and there are no origins in it. This is per-run scratch for a spec
 * that is deliberately a sequence (`testIsolation: false`), not a fixture:
 * nothing seeds it, and nothing reads it once the run is over.
 */
const carried = new Map<string, string>();

/** Stores `value` for a later test in the same run. */
export function carryForward({
  key,
  value,
}: {
  key: string;
  value: string;
}): null {
  carried.set(key, value);
  // `cy.task` fails the test if a task resolves `undefined`, so this is
  // `null` rather than nothing.
  return null;
}

/**
 * Reads back what {@link carryForward} stored, or `null`.
 *
 * `null` rather than a throw: the caller knows which test was supposed to have
 * stored it, and `cypress/support/dashboard.ts` turns the absence into a
 * message that names the key.
 */
export function carriedForward(key: string): string | null {
  return carried.get(key) ?? null;
}

/**
 * The one-time password `vpay-server staff add` printed, from the file
 * `just demo-staff` wrote it to.
 *
 * Throws with the recipe to run rather than returning an empty string: a spec
 * that signed in with `""` would fail at the password step and read as "the
 * credential was rejected", which is the one thing that is not wrong.
 */
export function staffPassword(): string {
  const path = process.env["VPAY_STAFF_PASSWORD_FILE"];
  if (typeof path !== "string" || path.length === 0) {
    throw new Error(
      "dashboard-human-login.cy.ts: VPAY_STAFF_PASSWORD_FILE is not set. `just e2e-specs` sets it; " +
        "running `cypress run` by hand needs `just demo-staff` first and the variable " +
        "pointed at the file it writes.",
    );
  }
  let password: string;
  try {
    password = readFileSync(path, "utf8").trim();
  } catch (cause) {
    throw new Error(
      `dashboard-human-login.cy.ts: cannot read the staff one-time password at ${path}. ` +
        "Run `just demo-staff` against the running stack.",
      { cause },
    );
  }
  if (password.length === 0) {
    throw new Error(
      `dashboard-human-login.cy.ts: the staff password file at ${path} is empty. ` +
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
      throw new Error(
        `dashboard-human-login.cy.ts: '${character}' is not RFC 4648 base32`,
      );
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
   * `0` is the current step. vpay's replay guard is a compare-and-swap on
   * `staff_members.last_totp_step` that admits only a strictly greater step, so
   * a second sign-in inside the same 30 seconds cannot present the same code —
   * and would be refused for the right reason with a message that looks exactly
   * like a wrong one. The spec waits out the step instead of skewing.
   */
  skew?: number;
}

/**
 * An RFC 6238 code for `secret`.
 *
 * HMAC-SHA1 over the big-endian counter, dynamic truncation, modulo 10^6 — the
 * algorithm `vpay_api::staff_auth::totp` implements and pins against RFC 6238
 * Appendix B's own vectors on its side.
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
