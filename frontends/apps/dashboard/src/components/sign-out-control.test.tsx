import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { FormState } from "../form-state";
import { SignOutControl } from "./sign-out-control";

describe("the sign-out control", () => {
  it("is a POST form with one button, and no alert before anything is tried", () => {
    const { container } = render(<SignOutControl signOut={vi.fn()} />);
    const button = screen.getByRole("button", { name: "Sign out" });
    expect(button).toHaveAttribute("type", "submit");
    expect(button.closest("form")).not.toBeNull();
    expect(container.querySelector("a")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows the refusal and its request id, and the button works again", async () => {
    // The action kept the cookie because vpay did not confirm the revocation;
    // the person must read that they are still signed in, and be able to try
    // again. THE DECISIVE MUTATION: drop the alert from the control, and the
    // refusal is a button that does nothing.
    const refused: FormState = {
      error: "Could not sign out, so you are still signed in. Try again.",
      requestId: "req_out",
    };
    const signOut = vi.fn(() => Promise.resolve(refused));
    render(<SignOutControl signOut={signOut} />);

    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      /could not sign out.*try again/i,
    );
    expect(screen.getByRole("alert")).toHaveTextContent("req_out");
    const button = screen.getByRole("button", { name: "Sign out" });
    expect(button).not.toBeDisabled();

    fireEvent.click(button);
    await vi.waitFor(() => expect(signOut).toHaveBeenCalledTimes(2));
  });
});
