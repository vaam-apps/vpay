/**
 * The four server actions, against a stubbed `fetch`, cookie jar and request
 * headers.
 *
 * Each of these is a `POST` endpoint anything on the internet can reach, so the
 * cases below are the properties that make that survivable: the origin check
 * runs first and does nothing when it refuses, a typo is not a sign-out, a
 * password is not trimmed, sign-out revokes before it forgets.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const world = vi.hoisted(() => ({
  jar: new Map<string, string>(),
  events: [] as string[],
  origin: "http://localhost:3000",
  host: "localhost:3000",
  /** `VPAY_DASHBOARD_PUBLIC_ORIGIN`, as `assembleConfig` answers it. */
  publicOrigin: null as string | null,
  /** The three client variables missing: `config` is `null`. */
  unconfigured: false,
  sets: [] as {
    name: string;
    value: string;
    options: Record<string, unknown>;
  }[],
}));

vi.mock("next/headers", () => ({
  cookies: () =>
    Promise.resolve({
      get: (name: string) => {
        const value = world.jar.get(name);
        return value === undefined ? undefined : { name, value };
      },
      set: (name: string, value: string, options: Record<string, unknown>) => {
        world.events.push(
          `cookie:${name}=${value === "" ? "(cleared)" : "set"}`,
        );
        world.sets.push({ name, value, options });
        if (value === "") {
          world.jar.delete(name);
        } else {
          world.jar.set(name, value);
        }
      },
    }),
  headers: () =>
    Promise.resolve({
      get: (name: string) =>
        name === "origin" ? world.origin : name === "host" ? world.host : null,
    }),
}));
vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    throw new Error(`redirect:${to}`);
  },
}));
vi.mock("../config/runtime", () => ({
  dashboardConfig: () => ({
    config: world.unconfigured
      ? null
      : {
          apiBaseUrl: "http://vpay:8080",
          clientId: "vpay-dashboard",
          redirectUri: "http://localhost:3000/dash/v1/callback",
          scope: "dashboard:read",
        },
    problems: world.unconfigured
      ? [{ variable: "VPAY_DASHBOARD_CLIENT_ID", detail: "the client id." }]
      : [],
    apiBaseUrl: "http://vpay:8080",
    publicOrigin: world.publicOrigin,
  }),
}));

import { NO_ERROR, type FormState } from "../form-state";
import { changePassword, signIn, signOut, submitTotp } from "./actions";
import { encodePendingEnrolment } from "./enrolment";

interface Sent {
  path: string;
  headers: Record<string, string>;
  body: URLSearchParams | null;
}

/** A `fetch` input as the URL it names (`String(request)` would not be one). */
function urlOf(input: string | URL | Request): string {
  if (typeof input === "string") {
    return input;
  }
  return input instanceof URL ? input.href : input.url;
}

/** Answers by path and records what was sent, in `world.events` order too. */
function vpay(routes: Record<string, { status: number; body?: unknown }>) {
  const sent: Sent[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: string | URL | Request, init?: RequestInit) => {
      const url = new URL(urlOf(input));
      world.events.push(`vpay:${url.pathname}`);
      sent.push({
        path: url.pathname,
        headers: (init?.headers ?? {}) as Record<string, string>,
        body:
          typeof init?.body === "string"
            ? new URLSearchParams(init.body)
            : null,
      });
      const hit = routes[url.pathname] ?? { status: 500 };
      return Promise.resolve(
        new Response(JSON.stringify(hit.body ?? {}), {
          status: hit.status,
          headers: { "request-id": "req_act" },
        }),
      );
    }),
  );
  return sent;
}

function formOf(fields: Record<string, string>): FormData {
  const form = new FormData();
  for (const [name, value] of Object.entries(fields)) {
    form.set(name, value);
  }
  return form;
}

async function run(
  action: () => Promise<FormState | void>,
): Promise<FormState | string> {
  try {
    return (await action()) ?? NO_ERROR;
  } catch (error) {
    return (error as Error).message;
  }
}

beforeEach(() => {
  world.jar.clear();
  world.events.length = 0;
  world.sets.length = 0;
  world.origin = "http://localhost:3000";
  world.host = "localhost:3000";
  world.publicOrigin = null;
  world.unconfigured = false;
  vi.spyOn(console, "warn").mockImplementation(() => {});
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("every action opens with the origin check", () => {
  it.each([
    [
      "signIn",
      () => signIn(NO_ERROR, formOf({ email: "a@b.c", password: "x" })),
    ],
    ["submitTotp", () => submitTotp(NO_ERROR, formOf({ code: "123456" }))],
    [
      "changePassword",
      () =>
        changePassword(
          NO_ERROR,
          formOf({
            current_password: "a",
            new_password: "b",
            confirm_password: "b",
          }),
        ),
    ],
  ])(
    "%s refuses a cross-origin POST before reading anything or calling vpay",
    async (_name, call) => {
      world.origin = "https://evil.example";
      world.jar.set("__Host-vpay_dash_session", "sess-1");
      const sent = vpay({});
      const result = await run(call);
      expect(result).toMatchObject({
        error:
          "This request did not come from this dashboard. Reload the page and try again.",
      });
      expect(sent).toEqual([]);
      expect(world.sets).toEqual([]);
    },
  );

  it("signOut, refused, does nothing at all and answers the redirect an honest caller gets", async () => {
    // A forged sign-out must be indistinguishable from a completed one to the
    // forger, and must neither revoke the row nor clear the cookie.
    world.origin = "https://evil.example";
    world.jar.set("__Host-vpay_dash_session", "sess-1");
    const sent = vpay({});
    expect(await run(() => signOut(NO_ERROR, new FormData()))).toBe(
      "redirect:/login",
    );
    expect(sent).toEqual([]);
    expect(world.sets).toEqual([]);
    expect(world.jar.get("__Host-vpay_dash_session")).toBe("sess-1");
  });
});

describe("the origin check does not depend on the client variables", () => {
  // `publicOrigin` is answered beside `apiBaseUrl`, outside `config`. THE
  // DECISIVE MUTATION: read it from `config?.publicOrigin` again — `config`
  // is `null` here, the configured origin is ignored, and the check compares
  // `Host`, which a caller who sets `Origin` can set to agree with it.
  const refusal =
    "This request did not come from this dashboard. Reload the page and try again.";

  it("refuses a cross-origin POST whose Host agrees with its Origin, when the configured origin says otherwise", async () => {
    world.unconfigured = true;
    world.publicOrigin = "https://dash.example";
    world.origin = "https://evil.example";
    world.host = "evil.example";
    const sent = vpay({});
    const result = await run(() =>
      signIn(NO_ERROR, formOf({ email: "a@b.c", password: "x" })),
    );
    expect(result).toMatchObject({ error: refusal });
    expect(sent).toEqual([]);
  });

  it("admits the configured origin when Host is an internal name, and then says the client is unregistered", async () => {
    world.unconfigured = true;
    world.publicOrigin = "https://dash.example";
    world.origin = "https://dash.example";
    world.host = "dashboard:3000";
    const result = await run(() =>
      signIn(NO_ERROR, formOf({ email: "a@b.c", password: "x" })),
    );
    expect(result).toMatchObject({
      error: expect.stringContaining(
        "no dashboard client registered",
      ) as string,
    });
  });
});

describe("signIn", () => {
  it("sets the session cookie with the non-negotiable attributes, then asks for the code", async () => {
    vpay({
      "/dash/v1/staff/login": {
        status: 200,
        body: { session: "sess-new", next: "totp" },
      },
    });
    const result = await run(() =>
      signIn(NO_ERROR, formOf({ email: "ada@acme.test", password: "pw" })),
    );
    expect(result).toBe("redirect:/login/totp");
    const set = world.sets.find(
      (entry) => entry.name === "__Host-vpay_dash_session",
    );
    expect(set?.value).toBe("sess-new");
    expect(set?.options).toMatchObject({
      httpOnly: true,
      secure: true,
      sameSite: "lax",
      path: "/",
      maxAge: 12 * 60 * 60,
    });
    // A `__Host-` cookie is refused by the browser if it carries a `Domain`.
    expect(set?.options).not.toHaveProperty("domain");
  });

  it("sends the password exactly as typed — a leading or trailing space is part of it", async () => {
    const sent = vpay({
      "/dash/v1/staff/login": {
        status: 401,
        body: { error: { message: "No." } },
      },
    });
    await run(() =>
      signIn(NO_ERROR, formOf({ email: "  ada@acme.test ", password: " pw " })),
    );
    expect(sent[0]?.body?.get("password")).toBe(" pw ");
    expect(sent[0]?.body?.get("email")).toBe("ada@acme.test");
    // And no credential header on the step that precedes a session.
    expect(Object.keys(sent[0]?.headers ?? {})).not.toContain("authorization");
    expect(Object.keys(sent[0]?.headers ?? {})).not.toContain(
      "x-vpay-staff-session",
    );
  });

  it("holds the sealed enrolment secret in its own httpOnly cookie on a first sign-in", async () => {
    vpay({
      "/dash/v1/staff/login": {
        status: 200,
        body: {
          session: "sess-new",
          next: "totp_enrolment",
          enrolment: "sealed-blob",
          otpauth_uri: "otpauth://totp/x:y?secret=ABC&issuer=x",
        },
      },
    });
    await run(() =>
      signIn(NO_ERROR, formOf({ email: "a@b.c", password: "pw" })),
    );
    const set = world.sets.find(
      (entry) => entry.name === "__Host-vpay_dash_enrolment",
    );
    expect(set?.value).toContain("sealed-blob");
    expect(set?.options).toMatchObject({ httpOnly: true, maxAge: 600 });
  });

  it("leaves no stale enrolment blob behind for an account that is already enrolled", async () => {
    world.jar.set("__Host-vpay_dash_enrolment", "stale");
    vpay({
      "/dash/v1/staff/login": {
        status: 200,
        body: { session: "sess-new", next: "totp" },
      },
    });
    await run(() =>
      signIn(NO_ERROR, formOf({ email: "a@b.c", password: "pw" })),
    );
    expect(world.jar.has("__Host-vpay_dash_enrolment")).toBe(false);
  });

  it("answers vpay's one sentence and its request id, and sets no cookie, on a refusal", async () => {
    vpay({
      "/dash/v1/staff/login": {
        status: 401,
        body: { error: { message: "We could not sign you in." } },
      },
    });
    const result = await run(() =>
      signIn(NO_ERROR, formOf({ email: "a@b.c", password: "pw" })),
    );
    expect(result).toEqual({
      error: "We could not sign you in.",
      requestId: "req_act",
    });
    expect(world.sets).toEqual([]);
  });

  it("does not spend an argon2id verification on an empty answer", async () => {
    const sent = vpay({});
    const result = await run(() =>
      signIn(NO_ERROR, formOf({ email: "a@b.c", password: "" })),
    );
    expect(result).toMatchObject({
      error: "Enter your work email and your password.",
    });
    expect(sent).toEqual([]);
  });
});

describe("submitTotp", () => {
  beforeEach(() => {
    world.jar.set("__Host-vpay_dash_session", "sess-1");
  });

  it("presents the session in X-Vpay-Staff-Session and the sealed secret from the cookie", async () => {
    world.jar.set(
      "__Host-vpay_dash_enrolment",
      encodePendingEnrolment({
        sealed: "sealed-blob",
        otpauth: "otpauth://totp/x:y?secret=ABC",
      }),
    );
    const sent = vpay({
      "/dash/v1/staff/totp": {
        status: 200,
        body: { password_change_required: false },
      },
    });
    expect(
      await run(() => submitTotp(NO_ERROR, formOf({ code: "123456" }))),
    ).toBe("redirect:/");
    expect(sent[0]?.headers["x-vpay-staff-session"]).toBe("sess-1");
    expect(Object.keys(sent[0]?.headers ?? {})).not.toContain("authorization");
    expect(sent[0]?.body?.get("code")).toBe("123456");
    expect(sent[0]?.body?.get("enrolment")).toBe("sealed-blob");
    expect(world.jar.has("__Host-vpay_dash_enrolment")).toBe(false);
  });

  it("goes to the password page when the printed password is still in force", async () => {
    vpay({
      "/dash/v1/staff/totp": {
        status: 200,
        body: { password_change_required: true },
      },
    });
    expect(
      await run(() => submitTotp(NO_ERROR, formOf({ code: "123456" }))),
    ).toBe("redirect:/login/password");
  });

  it("does not mint a token — the gate on the next page does", async () => {
    const sent = vpay({
      "/dash/v1/staff/totp": {
        status: 200,
        body: { password_change_required: false },
      },
    });
    await run(() => submitTotp(NO_ERROR, formOf({ code: "123456" })));
    expect(sent.map((call) => call.path)).toEqual(["/dash/v1/staff/totp"]);
  });

  it("a wrong code is a sentence, not a sign-out — the session and enrolment cookies stay (F6)", async () => {
    world.jar.set(
      "__Host-vpay_dash_enrolment",
      encodePendingEnrolment({
        sealed: "sealed-blob",
        otpauth: "otpauth://totp/x:y?secret=ABC",
      }),
    );
    vpay({
      "/dash/v1/staff/totp": {
        status: 401,
        body: { error: { message: "We could not sign you in." } },
      },
    });
    const result = await run(() =>
      submitTotp(NO_ERROR, formOf({ code: "000000" })),
    );
    expect(result).toMatchObject({ error: "We could not sign you in." });
    expect(world.sets).toEqual([]);
    expect(world.jar.has("__Host-vpay_dash_session")).toBe(true);
    expect(world.jar.has("__Host-vpay_dash_enrolment")).toBe(true);
  });

  it("goes back to /login with no session cookie, without calling vpay", async () => {
    world.jar.delete("__Host-vpay_dash_session");
    const sent = vpay({});
    expect(
      await run(() => submitTotp(NO_ERROR, formOf({ code: "123456" }))),
    ).toBe("redirect:/login");
    expect(sent).toEqual([]);
  });
});

describe("changePassword", () => {
  beforeEach(() => {
    world.jar.set("__Host-vpay_dash_session", "sess-1");
  });

  it("sends the current and the new password and never the repeat", async () => {
    const sent = vpay({
      "/dash/v1/staff/password": {
        status: 200,
        body: { password_change_required: false },
      },
    });
    expect(
      await run(() =>
        changePassword(
          NO_ERROR,
          formOf({
            current_password: "printed-one",
            new_password: "a-much-longer-one",
            confirm_password: "a-much-longer-one",
          }),
        ),
      ),
    ).toBe("redirect:/");
    expect(sent[0]?.body?.get("current_password")).toBe("printed-one");
    expect(sent[0]?.body?.get("new_password")).toBe("a-much-longer-one");
    expect(sent[0]?.body?.has("confirm_password")).toBe(false);
    expect(sent[0]?.headers["x-vpay-staff-session"]).toBe("sess-1");
  });

  it("refuses a pair that does not match without calling vpay", async () => {
    const sent = vpay({});
    const result = await run(() =>
      changePassword(
        NO_ERROR,
        formOf({
          current_password: "printed-one",
          new_password: "a-much-longer-one",
          confirm_password: "a-much-longer-two",
        }),
      ),
    );
    expect(result).toMatchObject({ error: "The two passwords do not match." });
    expect(sent).toEqual([]);
  });

  it("names the current password as the field a wrong one is about — vpay's 401", async () => {
    // THE DECISIVE MUTATION: return `shown(result.failure)` with no field, and
    // the sentence lands under "Repeat it".
    vpay({
      "/dash/v1/staff/password": {
        status: 401,
        body: { error: { message: "We could not sign you in." } },
      },
    });
    const result = await run(() =>
      changePassword(
        NO_ERROR,
        formOf({
          current_password: "wrong",
          new_password: "a-much-longer-one",
          confirm_password: "a-much-longer-one",
        }),
      ),
    );
    expect(result).toMatchObject({ field: "current_password" });
  });

  it("names the new password as the field a policy refusal is about — vpay's 400 with param new_password", async () => {
    vpay({
      "/dash/v1/staff/password": {
        status: 400,
        body: {
          error: {
            message: "A password must be between 12 and 128 characters.",
            param: "new_password",
          },
        },
      },
    });
    const result = await run(() =>
      changePassword(
        NO_ERROR,
        formOf({
          current_password: "printed-one",
          new_password: "x".repeat(200),
          confirm_password: "x".repeat(200),
        }),
      ),
    );
    expect(result).toMatchObject({
      field: "new_password",
      error: "A password must be between 12 and 128 characters.",
    });
  });

  it("names no field for a refusal that is about the whole form — a 429", async () => {
    vpay({
      "/dash/v1/staff/password": {
        status: 429,
        body: { error: { message: "Too many attempts." } },
      },
    });
    const result = await run(() =>
      changePassword(
        NO_ERROR,
        formOf({
          current_password: "printed-one",
          new_password: "a-much-longer-one",
          confirm_password: "a-much-longer-one",
        }),
      ),
    );
    expect(result).toMatchObject({ error: "Too many attempts." });
    expect(result).not.toHaveProperty("field");
  });

  it("names the field for the three checks made here", async () => {
    vpay({});
    expect(
      await run(() =>
        changePassword(
          NO_ERROR,
          formOf({
            current_password: "",
            new_password: "a",
            confirm_password: "a",
          }),
        ),
      ),
    ).toMatchObject({ field: "current_password" });
    expect(
      await run(() =>
        changePassword(
          NO_ERROR,
          formOf({
            current_password: "p",
            new_password: "",
            confirm_password: "",
          }),
        ),
      ),
    ).toMatchObject({ field: "new_password" });
    expect(
      await run(() =>
        changePassword(
          NO_ERROR,
          formOf({
            current_password: "p",
            new_password: "a",
            confirm_password: "b",
          }),
        ),
      ),
    ).toMatchObject({ field: "confirm_password" });
  });

  it("a wrong current password is a sentence, not a sign-out — the cookie stays", async () => {
    vpay({
      "/dash/v1/staff/password": {
        status: 401,
        body: { error: { message: "We could not sign you in." } },
      },
    });
    const result = await run(() =>
      changePassword(
        NO_ERROR,
        formOf({
          current_password: "wrong",
          new_password: "a-much-longer-one",
          confirm_password: "a-much-longer-one",
        }),
      ),
    );
    expect(result).toMatchObject({ error: "We could not sign you in." });
    expect(world.jar.has("__Host-vpay_dash_session")).toBe(true);
    expect(world.sets).toEqual([]);
  });
});

describe("signOut", () => {
  it("revokes the row at vpay BEFORE it forgets the cookie", async () => {
    // The row carries the access token, so deleting it is the revocation.
    // Clearing the cookie first and then failing to reach vpay would leave a
    // live token with nothing pointing at it.
    world.jar.set("__Host-vpay_dash_session", "sess-1");
    const sent = vpay({
      "/dash/v1/staff/logout": { status: 200, body: { signed_out: true } },
    });
    expect(await run(() => signOut(NO_ERROR, new FormData()))).toBe(
      "redirect:/login",
    );
    expect(sent[0]?.headers["x-vpay-staff-session"]).toBe("sess-1");
    expect(world.events).toEqual([
      "vpay:/dash/v1/staff/logout",
      "cookie:__Host-vpay_dash_session=(cleared)",
      "cookie:__Host-vpay_dash_enrolment=(cleared)",
    ]);
  });

  it("forgets the sealed enrolment secret with the session", async () => {
    // THE DECISIVE MUTATION: clear the session cookie only. The sealed secret
    // then stays in a shared browser for the ten minutes of its life.
    world.jar.set("__Host-vpay_dash_session", "sess-1");
    world.jar.set("__Host-vpay_dash_enrolment", "sealed-blob");
    vpay({
      "/dash/v1/staff/logout": { status: 200, body: { signed_out: true } },
    });
    expect(await run(() => signOut(NO_ERROR, new FormData()))).toBe(
      "redirect:/login",
    );
    expect(world.jar.has("__Host-vpay_dash_enrolment")).toBe(false);
    const cleared = world.sets.find(
      (entry) => entry.name === "__Host-vpay_dash_enrolment",
    );
    expect(cleared?.options).toMatchObject({
      httpOnly: true,
      secure: true,
      sameSite: "lax",
      path: "/",
      maxAge: 0,
    });
  });

  it("keeps the enrolment secret too when the revocation was not confirmed", async () => {
    world.jar.set("__Host-vpay_dash_session", "sess-1");
    world.jar.set("__Host-vpay_dash_enrolment", "sealed-blob");
    vpay({ "/dash/v1/staff/logout": { status: 503 } });
    await run(() => signOut(NO_ERROR, new FormData()));
    expect(world.jar.has("__Host-vpay_dash_enrolment")).toBe(true);
    expect(world.sets).toEqual([]);
  });

  it("KEEPS the cookie and says so when vpay answers 503 — a live row nobody could sign out of is the failure", async () => {
    // THE DECISIVE MUTATION: clear the cookie whatever vpay answered, which is
    // what this action did until the VPA-5a review while its own comment said
    // the opposite.
    world.jar.set("__Host-vpay_dash_session", "sess-1");
    vpay({ "/dash/v1/staff/logout": { status: 503 } });
    const result = await run(() => signOut(NO_ERROR, new FormData()));
    expect(result).toMatchObject({ requestId: "req_act" });
    expect((result as FormState).error).toMatch(
      /could not sign out.*try again/i,
    );
    expect(world.jar.get("__Host-vpay_dash_session")).toBe("sess-1");
    expect(world.sets).toEqual([]);
  });

  it("keeps the cookie when vpay cannot be reached at all", async () => {
    world.jar.set("__Host-vpay_dash_session", "sess-1");
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new Error("connection reset"))),
    );
    const result = await run(() => signOut(NO_ERROR, new FormData()));
    expect(result).toMatchObject({ requestId: null });
    expect((result as FormState).error).toMatch(/could not sign out/i);
    expect(world.jar.has("__Host-vpay_dash_session")).toBe(true);
  });

  it("is a sign-out without a call when there is no cookie to revoke", async () => {
    const sent = vpay({});
    expect(await run(() => signOut(NO_ERROR, new FormData()))).toBe(
      "redirect:/login",
    );
    expect(sent).toEqual([]);
  });
});
