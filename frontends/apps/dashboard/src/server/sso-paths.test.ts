/**
 * The callback path is a contract with vpay's configuration: the identity
 * provider redirects to `staff_auth.sso.redirect_uri`, and the route that
 * answers is the file under `app/`. These two must be the same string.
 */
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { SSO_CALLBACK_PATH, SSO_START_PATH } from "./sso-paths";

const APP = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "app");

function routeFile(path: string): string {
  return join(APP, ...path.split("/").filter(Boolean), "route.ts");
}

describe("the SSO paths", () => {
  it.each([
    ["start", SSO_START_PATH],
    ["callback", SSO_CALLBACK_PATH],
  ])("%s names a route file that exists", (_name, path) => {
    expect(existsSync(routeFile(path))).toBe(true);
  });

  it("are the ones the docs and the Helm values spell", () => {
    expect(SSO_START_PATH).toBe("/api/auth/sso/start");
    expect(SSO_CALLBACK_PATH).toBe("/api/auth/sso/callback");
  });

  it("start answers POST only — a GET that sets a cookie and redirects is a link a chat can fire", () => {
    const source = readFileSync(routeFile(SSO_START_PATH), "utf8");
    expect(source).toMatch(/export function POST\b/);
    expect(source).not.toMatch(
      /export (async )?function (GET|PUT|PATCH|DELETE)\b/,
    );
  });

  it("callback answers GET only — it is the identity provider's redirect", () => {
    const source = readFileSync(routeFile(SSO_CALLBACK_PATH), "utf8");
    expect(source).toMatch(/export function GET\b/);
    expect(source).not.toMatch(
      /export (async )?function (POST|PUT|PATCH|DELETE)\b/,
    );
  });

  it("both routes are dynamic: they set cookies and read the request", () => {
    for (const path of [SSO_START_PATH, SSO_CALLBACK_PATH]) {
      expect(readFileSync(routeFile(path), "utf8")).toContain(
        'export const dynamic = "force-dynamic"',
      );
    }
  });
});
