import { describe, expect, it } from "vitest";

import {
  API_BASE_URL_VAR,
  CLIENT_ID_VAR,
  PUBLIC_ORIGIN_VAR,
  REDIRECT_URI_VAR,
  SCOPE_VAR,
  assembleConfig,
} from "./settings";

const COMPLETE: Record<string, string> = {
  [API_BASE_URL_VAR]: "http://vpay-server:8080",
  [CLIENT_ID_VAR]: "vpay-dashboard",
  [REDIRECT_URI_VAR]: "http://localhost:3000/dash/v1/callback",
  [SCOPE_VAR]: "dashboard:read",
};

describe("a complete configuration", () => {
  it("is assembled as written", () => {
    const { config, problems } = assembleConfig(COMPLETE);
    expect(problems).toEqual([]);
    expect(config).toEqual({
      apiBaseUrl: "http://vpay-server:8080",
      clientId: "vpay-dashboard",
      redirectUri: "http://localhost:3000/dash/v1/callback",
      scope: "dashboard:read",
      // Absent from COMPLETE on purpose: it is the one OPTIONAL setting, and
      // `null` here is what makes `server/csrf.ts` fall back to comparing
      // `Host` rather than to allowing anything.
      publicOrigin: null,
    });
  });

  it("carries the public origin when the deployment set one", () => {
    // The only consumer is the origin check in front of every server action
    // (issue #88 item 4). It is optional, and a deployment behind a proxy
    // that rewrites `Host` has to set it — see `DashboardConfig.publicOrigin`.
    const { config, problems } = assembleConfig({
      ...COMPLETE,
      [PUBLIC_ORIGIN_VAR]: "https://dash.example",
    });
    expect(problems).toEqual([]);
    expect(config?.publicOrigin).toBe("https://dash.example");
  });

  it("does not refuse to boot without one", () => {
    // Deliberate, and recorded rather than assumed: making it required would
    // refuse every server action on every deployment that has not been
    // reconfigured. The consequence is the `Host` fallback, never an
    // allow-everything.
    const { config, problems } = assembleConfig({
      ...COMPLETE,
      [PUBLIC_ORIGIN_VAR]: "   ",
    });
    expect(problems).toEqual([]);
    expect(config?.publicOrigin).toBeNull();
  });

  it("drops a trailing slash from the base URL only", () => {
    // `//dash/v1/...` is a different path to axum and answers the 404
    // envelope. The redirect URI is NOT normalised: it is compared byte for
    // byte against a registration this app cannot see.
    const { config } = assembleConfig({
      ...COMPLETE,
      [API_BASE_URL_VAR]: "http://vpay-server:8080/",
      [REDIRECT_URI_VAR]: "http://localhost:3000/dash/v1/callback/",
    });
    expect(config?.apiBaseUrl).toBe("http://vpay-server:8080");
    expect(config?.redirectUri).toBe("http://localhost:3000/dash/v1/callback/");
  });
});

describe("an incomplete configuration", () => {
  it.each([API_BASE_URL_VAR, CLIENT_ID_VAR, REDIRECT_URI_VAR, SCOPE_VAR])(
    "fails closed when %s is absent, and names it",
    (variable) => {
      const { [variable]: _dropped, ...rest } = COMPLETE;
      const { config, problems } = assembleConfig(rest);
      // Null, not a plausible default: a login form that could not have
      // signed anybody in is worse than a page saying so.
      expect(config).toBeNull();
      expect(problems.map((problem) => problem.variable)).toContain(variable);
    },
  );

  it("treats a blank value as absent", () => {
    // `VPAY_DASHBOARD_CLIENT_ID=` in a compose file is a variable somebody
    // meant to set; honouring it would send `client_id=` to /authorize.
    const { config } = assembleConfig({ ...COMPLETE, [CLIENT_ID_VAR]: "   " });
    expect(config).toBeNull();
  });

  it("refuses a base URL that is not an absolute http(s) URL", () => {
    for (const value of ["vpay-server:8080", "/dash", "file:///etc/passwd"]) {
      expect(
        assembleConfig({ ...COMPLETE, [API_BASE_URL_VAR]: value }).config,
      ).toBeNull();
    }
  });

  it("reports every missing setting at once", () => {
    // An operator fixing them one restart at a time is the failure mode this
    // avoids.
    const { problems } = assembleConfig({});
    expect(problems.map((problem) => problem.variable).sort()).toEqual(
      [API_BASE_URL_VAR, CLIENT_ID_VAR, REDIRECT_URI_VAR, SCOPE_VAR].sort(),
    );
  });
});
