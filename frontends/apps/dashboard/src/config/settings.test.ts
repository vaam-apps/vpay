import { describe, expect, it } from "vitest";

import {
  API_BASE_URL_VAR,
  CLIENT_ID_VAR,
  PUBLIC_ORIGIN_VAR,
  REDIRECT_URI_VAR,
  SCOPE_VAR,
  SSO_ISSUER_VAR,
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
    const { config, problems, apiBaseUrl, publicOrigin } =
      assembleConfig(COMPLETE);
    expect(problems).toEqual([]);
    expect(apiBaseUrl).toBe("http://vpay-server:8080");
    expect(config).toEqual({
      apiBaseUrl: "http://vpay-server:8080",
      clientId: "vpay-dashboard",
      redirectUri: "http://localhost:3000/dash/v1/callback",
      scope: "dashboard:read",
    });
    // Absent from COMPLETE on purpose: it is the one OPTIONAL setting, and
    // `null` here is what makes `server/csrf.ts` fall back to comparing
    // `Host` rather than to allowing anything.
    expect(publicOrigin).toBeNull();
  });

  it("carries the public origin when the deployment set one", () => {
    // The only consumer is the origin check in front of every server action
    // (issue #88 item 4). It is optional, and a deployment behind a proxy
    // that rewrites `Host` has to set it — see `AssembledConfig.publicOrigin`.
    const { problems, publicOrigin } = assembleConfig({
      ...COMPLETE,
      [PUBLIC_ORIGIN_VAR]: "https://dash.example",
    });
    expect(problems).toEqual([]);
    expect(publicOrigin).toBe("https://dash.example");
  });

  it("answers the public origin when the client variables are missing too — the check must not fall back to Host there", () => {
    // THE DECISIVE MUTATION: put `publicOrigin` back inside `config`. It is
    // `null` whenever any client variable is missing, so the origin check
    // then compared `Host` on exactly the deployment that had set the
    // variable to say otherwise.
    const { config, problems, apiBaseUrl, publicOrigin } = assembleConfig({
      [API_BASE_URL_VAR]: "http://vpay-server:8080",
      [PUBLIC_ORIGIN_VAR]: "https://dash.example",
    });
    expect(config).toBeNull();
    expect(problems.length).toBeGreaterThan(0);
    expect(apiBaseUrl).toBe("http://vpay-server:8080");
    expect(publicOrigin).toBe("https://dash.example");
  });

  it("answers the public origin even when vpay's address is missing", () => {
    expect(
      assembleConfig({ [PUBLIC_ORIGIN_VAR]: "https://dash.example" })
        .publicOrigin,
    ).toBe("https://dash.example");
  });

  it("does not refuse to boot without one", () => {
    // Deliberate, and recorded rather than assumed: making it required would
    // refuse every server action on every deployment that has not been
    // reconfigured. The consequence is the `Host` fallback, never an
    // allow-everything.
    const { problems, publicOrigin } = assembleConfig({
      ...COMPLETE,
      [PUBLIC_ORIGIN_VAR]: "   ",
    });
    expect(problems).toEqual([]);
    expect(publicOrigin).toBeNull();
  });

  it("reduces the SSO issuer to its origin, and reads an unset or unusable one as unknown", () => {
    // Optional like the public origin, and for the same reason: unset must
    // never become a default. `null` makes `server/sso.ts` refuse to redirect
    // anywhere, because a redirect that cannot be checked is not made.
    expect(assembleConfig(COMPLETE).ssoIssuerOrigin).toBeNull();
    expect(
      assembleConfig({
        ...COMPLETE,
        [SSO_ISSUER_VAR]: "https://IDP.example:8443/realms/staff",
      }).ssoIssuerOrigin,
    ).toBe("https://idp.example:8443");
    for (const unusable of ["   ", "idp.example", "/realms/staff", "ftp://x"]) {
      const assembled = assembleConfig({
        ...COMPLETE,
        [SSO_ISSUER_VAR]: unusable,
      });
      expect(assembled.ssoIssuerOrigin).toBeNull();
      // Never a boot refusal: SSO is optional, the password sign-in is not.
      expect(assembled.problems).toEqual([]);
    }
  });

  it("reads an issuer on plain http as unknown unless it is on a loopback host", () => {
    // A redirect to a non-loopback `http` provider is always refused
    // (`safeAuthorizationUrl`), so counting such an issuer as configured would
    // draw a button that can only fail. Unknown shows the not-configured
    // notice instead.
    const origin = (issuer: string): string | null =>
      assembleConfig({ ...COMPLETE, [SSO_ISSUER_VAR]: issuer }).ssoIssuerOrigin;
    expect(origin("http://idp.example/realms/staff")).toBeNull();
    expect(origin("http://10.0.0.5:8180/realms/staff")).toBeNull();
    expect(origin("http://localhost:8180/realms/staff")).toBe(
      "http://localhost:8180",
    );
    expect(origin("http://127.0.0.1:8180/realms/staff")).toBe(
      "http://127.0.0.1:8180",
    );
    expect(origin("http://[::1]:8180/realms/staff")).toBe("http://[::1]:8180");
    expect(origin("https://idp.example/realms/staff")).toBe(
      "https://idp.example",
    );
    // Never a boot refusal: SSO is optional, the password sign-in is not.
    expect(
      assembleConfig({
        ...COMPLETE,
        [SSO_ISSUER_VAR]: "http://idp.example/realms/staff",
      }).problems,
    ).toEqual([]);
  });

  it("answers the SSO issuer origin when the client variables are missing too", () => {
    expect(
      assembleConfig({ [SSO_ISSUER_VAR]: "https://idp.example/realms/staff" })
        .ssoIssuerOrigin,
    ).toBe("https://idp.example");
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

describe("the base URL on its own", () => {
  // Whether vpay serves a sign-in is asked of vpay (`server/discovery.ts`), and
  // asking needs the address and nothing else. A deployment with no
  // `dashboard_client` has no client id, redirect URI or scope, and must be
  // told "paired key only", not that three variables are missing.
  it("is answered when only the three client values are missing", () => {
    const assembled = assembleConfig({
      [API_BASE_URL_VAR]: "http://vpay:8080/",
    });
    expect(assembled.config).toBeNull();
    expect(assembled.apiBaseUrl).toBe("http://vpay:8080");
    expect(
      assembled.problems.map((problem) => problem.variable).sort(),
    ).toEqual([CLIENT_ID_VAR, REDIRECT_URI_VAR, SCOPE_VAR].sort());
  });

  it("is null when it is absent, blank or not an http(s) URL", () => {
    for (const value of [undefined, "  ", "vpay:8080", "file:///etc/passwd"]) {
      const env = { ...COMPLETE, [API_BASE_URL_VAR]: value };
      expect(assembleConfig(env).apiBaseUrl).toBeNull();
    }
  });
});
