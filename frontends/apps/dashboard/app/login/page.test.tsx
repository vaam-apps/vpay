/**
 * `/login` — which page it is depends on what vpay says it serves.
 *
 * The page is an async server component, so it is called as a function and its
 * result rendered. Mocked: the configuration, the server actions (a `'use
 * server'` module cannot be imported into jsdom), the session read, and the
 * discovery read. The sign-in form and the notices around it are real.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  readDiscovery: vi.fn(),
  signedIn: { value: false },
  assembled: { value: {} },
}));

vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    throw new Error(`redirect:${to}`);
  },
}));
vi.mock("../../src/config/runtime", () => ({
  dashboardConfig: () => mocks.assembled.value,
}));
vi.mock("../../src/server/actions", () => ({ signIn: vi.fn() }));
vi.mock("../../src/server/session", () => ({
  alreadySignedIn: () => Promise.resolve(mocks.signedIn.value),
  HOME_PATH: "/",
}));
vi.mock("../../src/server/discovery", () => ({
  readDiscovery: mocks.readDiscovery,
}));

import LoginPage from "./page";

const CONFIGURED = {
  config: {
    apiBaseUrl: "http://vpay:8080",
    clientId: "vpay-dashboard",
    redirectUri: "http://localhost:3000/dash/v1/callback",
    scope: "dashboard:read",
    publicOrigin: null,
  },
  problems: [],
  apiBaseUrl: "http://vpay:8080",
};

const UNREGISTERED = {
  config: null,
  apiBaseUrl: "http://vpay:8080",
  problems: [
    { variable: "VPAY_DASHBOARD_CLIENT_ID", detail: "the client id." },
    { variable: "VPAY_DASHBOARD_REDIRECT_URI", detail: "the redirect." },
    { variable: "VPAY_DASHBOARD_SCOPE", detail: "the scope." },
  ],
};

async function renderLogin() {
  render(await LoginPage());
}

beforeEach(() => {
  mocks.signedIn.value = false;
  mocks.assembled.value = CONFIGURED;
  mocks.readDiscovery.mockReset();
});

describe("a deployment that serves human_login", () => {
  beforeEach(() => {
    mocks.readDiscovery.mockResolvedValue({
      kind: "served",
      methods: ["keypair", "human_login"],
    });
  });

  it("asks the email, alone — the password is the next screen", async () => {
    await renderLogin();
    expect(
      screen.getByRole("heading", { level: 2, name: "Sign in" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Work email")).toBeInTheDocument();
    expect(screen.queryByLabelText("Password")).toBeNull();
  });

  it("redirects a browser that already holds a session, before rendering a form", async () => {
    mocks.signedIn.value = true;
    await expect(LoginPage()).rejects.toThrow("redirect:/");
  });

  it("says which variables to set, and renders no form, when the client is not registered", async () => {
    mocks.assembled.value = UNREGISTERED;
    await renderLogin();
    expect(
      screen.getByRole("heading", { name: "No sign-in is configured" }),
    ).toBeInTheDocument();
    for (const variable of [
      "VPAY_DASHBOARD_CLIENT_ID",
      "VPAY_DASHBOARD_REDIRECT_URI",
      "VPAY_DASHBOARD_SCOPE",
    ]) {
      expect(screen.getByText(variable)).toBeInTheDocument();
    }
    expect(document.body.querySelector("form, input")).toBeNull();
  });
});

describe("a deployment that does not serve human_login", () => {
  beforeEach(() => {
    mocks.readDiscovery.mockResolvedValue({
      kind: "paired-key-only",
      methods: ["keypair"],
    });
  });

  it("says the console signs in with a paired key, and offers no field", async () => {
    await renderLogin();
    expect(
      screen.getByRole("heading", {
        level: 2,
        name: "This console signs in with a paired key",
      }),
    ).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("VP-6b");
    expect(document.body.querySelector("form, input, button")).toBeNull();
  });

  it("does not tell the operator to set the client variables — there is no client to register", async () => {
    mocks.assembled.value = UNREGISTERED;
    await renderLogin();
    expect(screen.queryByText("VPAY_DASHBOARD_CLIENT_ID")).toBeNull();
    expect(
      screen.getByRole("heading", { name: /paired key/ }),
    ).toBeInTheDocument();
  });

  it("ignores a signed-in browser — there is no sign-in to be past", async () => {
    mocks.signedIn.value = true;
    await renderLogin();
    expect(
      screen.getByRole("heading", { name: /paired key/ }),
    ).toBeInTheDocument();
  });
});

describe("a vpay that could not be asked", () => {
  it("is an outage with the request id, and never the paired-key notice", async () => {
    mocks.readDiscovery.mockResolvedValue({
      kind: "unreadable",
      failure: { status: 503, message: "Starting up.", requestId: "req_01J8" },
    });
    await renderLogin();
    expect(
      screen.getByRole("heading", { name: "Sign-in is not reachable" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("req_01J8");
    expect(screen.queryByText(/paired key/)).toBeNull();
    expect(document.body.querySelector("form, input")).toBeNull();
  });
});

describe("a container with no address for vpay", () => {
  it("names the variables and asks vpay nothing", async () => {
    mocks.assembled.value = {
      config: null,
      apiBaseUrl: null,
      problems: [{ variable: "VPAY_DASH_API", detail: "vpay's base URL." }],
    };
    await renderLogin();
    expect(screen.getByText("VPAY_DASH_API")).toBeInTheDocument();
    expect(mocks.readDiscovery).not.toHaveBeenCalled();
  });
});
