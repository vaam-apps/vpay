import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AccountPage } from "./account-page";
import { EnrolmentPanel } from "./enrolment-panel";

describe("the account page", () => {
  const props = {
    displayName: "Ada Lovelace",
    email: "ada@acme.test",
    merchantId: "acme",
  };

  it("says who is signed in and which merchant they read", () => {
    render(<AccountPage {...props} signOut={vi.fn()} />);
    expect(
      screen.getByRole("heading", {
        level: 2,
        name: "Signed in as Ada Lovelace",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText("ada@acme.test")).toBeInTheDocument();
    expect(screen.getByText("acme")).toBeInTheDocument();
  });

  it("signs out with a POST form, never a link", () => {
    const signOut = vi.fn();
    const { container } = render(<AccountPage {...props} signOut={signOut} />);
    const button = screen.getByRole("button", { name: "Sign out" });
    expect(button).toHaveAttribute("type", "submit");
    expect(button.closest("form")).not.toBeNull();
    expect(container.querySelector('a[href*="sign" i]')).toBeNull();
  });

  it("links to no page that does not exist — the read screens are a later ticket", () => {
    const { container } = render(<AccountPage {...props} signOut={vi.fn()} />);
    expect(container.querySelectorAll("a")).toHaveLength(0);
    expect(screen.getByRole("status")).toHaveTextContent("not built");
  });
});

describe("the enrolment screen", () => {
  it("shows the QR code and keeps the typed key in a sheet, not on the screen", async () => {
    render(
      <EnrolmentPanel
        qrDataUrl="data:image/png;base64,AAAA"
        secret="JBSWY3DPEHPK3PXP"
      />,
    );
    expect(screen.getByTestId("totp-qr")).toBeInTheDocument();
    expect(screen.queryByTestId("totp-secret")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Cannot scan? Enter the key instead",
      }),
    );
    expect(await screen.findByTestId("totp-secret")).toHaveTextContent(
      "JBSWY3DPEHPK3PXP",
    );
    // A sheet is a dialog, with a name; not a menu and not a listbox.
    expect(screen.getByRole("dialog")).toHaveAccessibleName(
      "Enter this key in your app",
    );
    expect(screen.queryByRole("menu")).toBeNull();
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("offers no typed key when the URI carried none", () => {
    render(
      <EnrolmentPanel qrDataUrl="data:image/png;base64,AAAA" secret={null} />,
    );
    expect(screen.queryByRole("button", { name: /Enter the key/ })).toBeNull();
  });
});
