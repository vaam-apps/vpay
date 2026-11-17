/**
 * `/login` when vpay never answers the discovery read.
 *
 * `page.test.tsx` mocks the discovery read; this file does not, so the whole
 * path from the call's deadline to the sentence on screen is real: a timeout
 * must render the outage page, never "This console signs in with a paired
 * key", which would send an operator to fix a deployment that is merely
 * wedged.
 */
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    throw new Error(`redirect:${to}`);
  },
}));
vi.mock("../../src/config/runtime", () => ({
  dashboardConfig: () => ({
    config: {
      apiBaseUrl: "http://vpay:8080",
      clientId: "vpay-dashboard",
      redirectUri: "http://localhost:3000/dash/v1/callback",
      scope: "dashboard:read",
    },
    problems: [],
    apiBaseUrl: "http://vpay:8080",
    publicOrigin: null,
    ssoIssuerOrigin: "https://idp.example",
  }),
}));
vi.mock("../../src/server/actions", () => ({ signIn: vi.fn() }));
vi.mock("../../src/server/session", () => ({
  alreadySignedIn: () => Promise.resolve(false),
  HOME_PATH: "/",
}));

import { hangingVpay } from "../../src/testing/hanging-fetch";
import LoginPage from "./page";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("a discovery read that times out", () => {
  it("renders the outage, with no form, and never the paired-key notice", async () => {
    const { timeOut } = hangingVpay();
    const pending = LoginPage({});
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
    timeOut();
    render(await pending);

    expect(
      screen.getByRole("heading", { name: "Sign-in is not reachable" }),
    ).toBeInTheDocument();
    expect(screen.queryByText(/paired key/i)).toBeNull();
    expect(document.body.querySelector("form, input")).toBeNull();
  });
});

describe("an SSO status read that times out after discovery answered", () => {
  it("says SSO could not be checked and keeps the email question — never hides the button silently", async () => {
    const { timeOut } = hangingVpay();
    // Discovery answers; the status read, the second call, never does.
    vi.mocked(fetch).mockImplementationOnce(() =>
      Promise.resolve(
        new Response(JSON.stringify({ methods: ["human_login"] }), {
          status: 200,
        }),
      ),
    );
    const pending = LoginPage({});
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledTimes(2));
    timeOut();
    render(await pending);

    expect(screen.getByRole("status")).toHaveTextContent(
      "Single sign-on could not be checked",
    );
    expect(
      screen.queryByRole("button", { name: "Sign in with single sign-on" }),
    ).toBeNull();
    expect(screen.getByLabelText("Work email")).toBeInTheDocument();
  });
});
