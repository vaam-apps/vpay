/**
 * D6, as two gates that read this package's own source and its own results
 * rather than trusting a review.
 *
 * The technique is `sdks/stripe-js/src/client.test.ts`'s "has no console
 * call anywhere in the shipping source" and the Flutter package's
 * `test/no_logging_test.dart`, applied here for the same reason: a
 * `console.log(url)` left in from debugging would print a session secret
 * into the payer's devtools, and from there into whatever error-reporting
 * SDK the merchant's front end has installed. Cheaper to forbid outright
 * than to review for.
 */
import { readdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { VpayCheckout } from "./checkout.js";
import {
  embeddedSessionNotSupportedError,
  invalidSessionUrlError,
  platformWindowError,
  unexpectedResponseError,
} from "./errors.js";
import {
  BASE_URL,
  PUBLISHABLE_KEY,
  SESSION_SECRET,
  SESSION_URL,
  fakeClock,
  fakeHost,
  scriptedFetch,
} from "./testing/fixtures.js";

/** Comments stripped, so this file's own prose about `console` is not a match. */
function strippedOfComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//gu, "")
    .replace(/(^|[^:])\/\/.*$/gmu, "$1");
}

describe("no logging", () => {
  it("has no console call anywhere in the shipping source", async () => {
    const dir = fileURLToPath(new URL(".", import.meta.url));
    const shipping = (await readdir(dir)).filter(
      (name) => name.endsWith(".ts") && !name.endsWith(".test.ts"),
    );

    // Pinned as a list, not a count: a new module that forgot to be added
    // here is a module this gate silently stopped covering. `testing/` is
    // not in it because `readdir` is not recursive — those files are test
    // doubles, excluded from `dist`, and they may print.
    expect(shipping.sort()).toEqual([
      "checkout.ts",
      "controller.ts",
      "errors.ts",
      "host-tauri.ts",
      "host-web.ts",
      "host.ts",
      "index.ts",
      "redaction.ts",
      "stop-url.ts",
      "types.ts",
    ]);

    for (const name of shipping) {
      const code = strippedOfComments(await readFile(`${dir}${name}`, "utf8"));
      expect(code, `${name} must not log`).not.toMatch(/console\s*\./u);
    }
  });
});

describe("no error message carries a URL, a secret or a thrown value (D6)", () => {
  it("every error this package builds is a fixed string, bar one public session id", async () => {
    const dir = fileURLToPath(new URL(".", import.meta.url));
    const code = strippedOfComments(await readFile(`${dir}errors.ts`, "utf8"));

    // One template literal in a message, and it interpolates `status`;
    // one more interpolates `sessionId`. Anything else — a `url`, an
    // `err`, a `secret`, a `cause` — is what this assertion exists to
    // catch, and it catches it at the source rather than by sampling
    // results.
    const interpolations = [...code.matchAll(/\$\{([^}]*)\}/gu)].map(
      (match) => match[1]?.trim() ?? "",
    );
    expect(interpolations.sort()).toEqual(["sessionId", "status"]);
  });

  it("a host that rejects quoting the session secret produces a result carrying none of it", async () => {
    const http = scriptedFetch({});
    const checkout = new VpayCheckout({
      baseUrl: BASE_URL,
      publishableKey: PUBLISHABLE_KEY,
      fetch: http.fetch,
      clock: fakeClock(),
      host: fakeHost({
        rejectWith: new Error(`could not open ${SESSION_URL}`),
      }),
    });

    const result = await checkout.start(SESSION_URL);

    const rendered = JSON.stringify(result);
    expect(rendered).not.toContain(SESSION_SECRET);
    expect(rendered).not.toContain("_secret_");
    expect(rendered).not.toContain("checkout.example");
  });

  it("none of the four factories renders anything that looks like a credential", () => {
    const messages = [
      platformWindowError().message ?? "",
      unexpectedResponseError(502).message ?? "",
      invalidSessionUrlError().message ?? "",
      embeddedSessionNotSupportedError("cs_123").message ?? "",
    ];

    for (const message of messages) {
      expect(message).not.toContain("_secret_");
      expect(message).not.toContain("pk_");
      expect(message).not.toMatch(/https?:\/\//u);
    }
  });
});
