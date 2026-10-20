/**
 * `/login` with and without single sign-on.
 *
 * The page is an async server component, so it is called as a function and
 * its result rendered. Mocked: the config, the server actions (a `'use
 * server'` module cannot be imported into jsdom), the session read, and the
 * one vpay call this page makes. The password form and the components around
 * it are real.
 */
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  getSsoStatus: vi.fn(),
  signedIn: { value: false },
}));

vi.mock("next/navigation", () => ({
  redirect: (to: string) => {
    throw new Error(`redirect:${to}`);
  },
}));
vi.mock("../../src/config/runtime", () => ({
  dashboardConfig: () => ({
    config: { apiBaseUrl: "http://vpay-server:8080" },
    problems: [],
  }),
}));
vi.mock("../../src/server/actions", () => ({ signIn: vi.fn() }));
vi.mock("../../src/server/session", () => ({
  alreadySignedIn: () => Promise.resolve(mocks.signedIn.value),
  HOME_PATH: "/payments",
}));
vi.mock("../../src/server/api", () => ({
  getSsoStatus: mocks.getSsoStatus,
}));

import { SSO_FAILED_MESSAGE } from "../../src/components/sso-sign-in";
import LoginPage from "./page";

async function renderLogin(search: Record<string, string | string[]> = {}) {
  render(await LoginPage({ searchParams: Promise.resolve(search) }));
}

beforeEach(() => {
  mocks.signedIn.value = false;
  mocks.getSsoStatus.mockReset();
});

describe("the login page", () => {
  it("shows the Keycloak button when single sign-on is enabled, as a plain form post", async () => {
    mocks.getSsoStatus.mockResolvedValue({ enabled: true });
    await renderLogin();

    const button = screen.getByRole("button", {
      name: "Sign in with Keycloak",
    });
    const form = button.closest("form");
    expect(form).not.toBeNull();
    // A plain HTML post: it must work without JavaScript.
    expect(form?.getAttribute("method")?.toLowerCase()).toBe("post");
    expect(form?.getAttribute("action")).toBe("/api/auth/sso/start");
    expect(button).toHaveAttribute("type", "submit");
    // No hidden field: no token, code or state is rendered into the page.
    expect(form?.querySelectorAll("input")).toHaveLength(0);
  });

  it("keeps the password form whole when single sign-on is enabled", async () => {
    mocks.getSsoStatus.mockResolvedValue({ enabled: true });
    await renderLogin();

    expect(screen.getByLabelText("Work email")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
  });

  it("shows no Keycloak button when single sign-on is not enabled, and the password form is untouched", async () => {
    mocks.getSsoStatus.mockResolvedValue({ enabled: false });
    await renderLogin();

    expect(screen.queryByRole("button", { name: /keycloak/i })).toBeNull();
    expect(screen.queryByText("or")).toBeNull();
    expect(screen.getByLabelText("Work email")).toBeInTheDocument();
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
  });

  it("shows one generic alert on ?sso=failed, above the password form", async () => {
    mocks.getSsoStatus.mockResolvedValue({ enabled: true });
    await renderLogin({ sso: "failed" });

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(SSO_FAILED_MESSAGE);
    expect(
      screen.getByLabelText("Work email").compareDocumentPosition(alert) &
        Node.DOCUMENT_POSITION_PRECEDING,
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Sign in" })).toBeInTheDocument();
  });

  it("shows no alert without ?sso=failed, and ignores any other value", async () => {
    mocks.getSsoStatus.mockResolvedValue({ enabled: true });
    await renderLogin();
    expect(screen.queryByRole("alert")).toBeNull();

    document.body.innerHTML = "";
    await renderLogin({ sso: "access_denied: some IdP text" });
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(/access_denied/)).toBeNull();
  });

  it("never renders anything else from the query string", async () => {
    mocks.getSsoStatus.mockResolvedValue({ enabled: true });
    await renderLogin({
      sso: "failed",
      code: "IDP-CODE-123",
      state: "IDP-STATE-456",
      error: "access_denied",
      error_description: "The IdP says no",
    });
    const html = document.body.innerHTML;
    for (const leaked of [
      "IDP-CODE-123",
      "IDP-STATE-456",
      "access_denied",
      "The IdP says no",
    ]) {
      expect(html).not.toContain(leaked);
    }
  });

  it("redirects an already signed-in browser before asking vpay about single sign-on", async () => {
    mocks.signedIn.value = true;
    await expect(
      LoginPage({ searchParams: Promise.resolve({}) }),
    ).rejects.toThrow("redirect:/payments");
    expect(mocks.getSsoStatus).not.toHaveBeenCalled();
  });
});
