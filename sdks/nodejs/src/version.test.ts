import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { SDK_VERSION } from "./version.js";

describe("SDK_VERSION", () => {
  it("matches package.json, so the User-Agent never lies about the version", () => {
    // `version.ts` is a hand-written constant (see its doc comment for why it
    // is not read from package.json at runtime). This is the only thing that
    // catches the two drifting apart.
    const pkg = JSON.parse(
      readFileSync(new URL("../package.json", import.meta.url), "utf8"),
    ) as { version: string };
    expect(SDK_VERSION).toBe(pkg.version);
  });
});
