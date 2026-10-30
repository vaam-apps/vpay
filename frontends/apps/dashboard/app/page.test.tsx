/**
 * `/` — the account page, and what it renders instead when vpay will not mint
 * the token a signed-in person's reads need.
 *
 * The real gate (`server/session.ts`) runs against a stubbed `fetch` and a
 * stubbed cookie jar; only the configuration, the cookie store, `redirect` and
 * the `'use server'` module are mocked. The page is an async server component,
 * so it is called as a function and its result rendered.
 */
import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const world = vi.hoisted(() => ({ redirects: [] as string[] }));

vi.mock("next/headers", () => ({
  cookies: () =>
    Promise.resolve({
      get: (name: string) =>
        name === "__Host-vpay_dash_session"
          ? { name, value: "sess-1" }
          : undefined,
    }),
}));
vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    world.redirects.push(to);
    throw new Error(`redirect:${to}`);
  },
}));
vi.mock("../src/config/runtime", () => ({
  dashboardConfig: () => ({
    config: {
      apiBaseUrl: "http://vpay:8080",
      clientId: "vpay-dashboard",
      redirectUri: "http://localhost:3000/dash/v1/callback",
      scope: "dashboard:read",
      publicOrigin: null,
    },
    problems: [],
    apiBaseUrl: "http://vpay:8080",
  }),
}));
vi.mock("../src/server/actions", () => ({ signOut: vi.fn() }));

import Home from "./page";

const SESSION = {
  staff_id: "stf_1",
  display_name: "Ada Lovelace",
  email: "ada@acme.test",
  merchant_id: "acme",
  password_change_required: false,
  access_token_ttl_seconds: 900,
};

function vpay(routes: Record<string, { status: number; body?: unknown }>) {
  vi.stubGlobal(
    "fetch",
    vi.fn((input: string | URL | Request) => {
      const raw =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const hit = routes[new URL(raw).pathname];
      if (hit === undefined) {
        return Promise.reject(new Error(`unrouted ${raw}`));
      }
      return Promise.resolve(
        new Response(JSON.stringify(hit.body ?? {}), {
          status: hit.status,
          headers: { "request-id": "req_home" },
        }),
      );
    }),
  );
}

beforeEach(() => {
  world.redirects.length = 0;
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe("a signed-in person vpay will not mint a token for", () => {
  it("is told so, with the request id and a way out — and the browser is not redirected", async () => {
    // The session read is 200 and the mint is 401: the case that looped the
    // browser `/` -> `/signed-out` -> `/login` -> `/` with no message.
    // THE DECISIVE MUTATION: restore `redirect(SIGNED_OUT_PATH)` in the gate.
    vpay({
      "/dash/v1/staff/session": { status: 200, body: SESSION },
      "/dash/v1/oauth/authorize": {
        status: 401,
        body: { error: { message: "We could not sign you in." } },
      },
    });

    render(await Home());

    expect(world.redirects).toEqual([]);
    expect(
      screen.getByRole("heading", {
        level: 2,
        name: "Signed in, but not authorised",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "We could not sign you in.",
    );
    expect(screen.getByRole("alert")).toHaveTextContent("req_home");
    expect(screen.getByRole("button", { name: "Sign out" })).toBeEnabled();
    expect(screen.queryByText(/Signed in as/)).toBeNull();
  });

  it("is not what a person holding a good token sees — they get the account page", async () => {
    vpay({
      "/dash/v1/staff/session": {
        status: 200,
        body: {
          ...SESSION,
          access_token: "header.payload.sig",
          access_token_expires_at: new Date(Date.now() + 600_000).toISOString(),
        },
      },
    });
    render(await Home());
    expect(world.redirects).toEqual([]);
    expect(
      screen.getByRole("heading", {
        level: 2,
        name: "Signed in as Ada Lovelace",
      }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Signed in, but not authorised")).toBeNull();
  });
});
