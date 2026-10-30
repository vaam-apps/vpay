import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { FormState } from "../form-state";
import { PasswordForm } from "./password-form";
import { SignInForm } from "./sign-in-form";
import { TotpForm } from "./totp-form";

const CLEAN: FormState = { error: null, requestId: null };

/** An action that never settles, so the form stays in its pending state. */
function neverSettles() {
  return vi.fn(
    (_previous: FormState, _form: FormData) => new Promise<FormState>(() => {}),
  );
}

/** An action that refuses with a sentence and an id. */
function refuses(error: string, requestId: string | null = null) {
  return vi.fn((_previous: FormState, _form: FormData) =>
    Promise.resolve({ error, requestId }),
  );
}

/** `act`, for a synchronous event whose follow-on (a transition) is async. */
async function inAct(event: () => void) {
  await act(() => {
    event();
    return Promise.resolve();
  });
}

function type(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

function press(name: string) {
  fireEvent.click(screen.getByRole("button", { name }));
}

/** Both sign-in answers given, standing on the password question. */
function toThePasswordStep() {
  type("Work email", "ada@acme.test");
  press("Continue");
}

describe("one question per screen", () => {
  it("asks the email alone first — no password field, and no Back to go to", () => {
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    expect(screen.getByLabelText("Work email")).toBeInTheDocument();
    expect(screen.queryByLabelText("Password")).toBeNull();
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
    expect(screen.getByText("Question 1 of 2")).toBeInTheDocument();
  });

  it("asks the password alone second, with the email gone from view", () => {
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    toThePasswordStep();
    expect(screen.getByLabelText("Password")).toHaveAttribute(
      "type",
      "password",
    );
    expect(screen.queryByLabelText("Work email")).toBeNull();
    expect(screen.getByText("Question 2 of 2")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Back" })).toBeInTheDocument();
  });

  it("never renders two questions at once, on any of the three flows", () => {
    for (const ui of [
      <SignInForm key="s" action={vi.fn(() => Promise.resolve(CLEAN))} />,
      <TotpForm key="t" action={vi.fn(() => Promise.resolve(CLEAN))} />,
      <PasswordForm key="p" action={vi.fn(() => Promise.resolve(CLEAN))} />,
    ]) {
      const { container, unmount } = render(ui);
      expect(container.querySelectorAll("input:not(.sr-only)")).toHaveLength(1);
      unmount();
    }
  });

  it("keeps Continue disabled until the answer is valid", () => {
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    type("Work email", "not-an-address");
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    type("Work email", "ada@acme.test");
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
  });

  it("does not advance on Enter while the answer is invalid", () => {
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    type("Work email", "nope");
    fireEvent.submit(
      screen.getByLabelText("Work email").closest("form") as HTMLFormElement,
    );
    expect(screen.getByLabelText("Work email")).toBeInTheDocument();
    expect(screen.queryByLabelText("Password")).toBeNull();
  });

  it("advances on Enter when the answer is valid", () => {
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    type("Work email", "ada@acme.test");
    fireEvent.submit(
      screen.getByLabelText("Work email").closest("form") as HTMLFormElement,
    );
    expect(screen.getByLabelText("Password")).toBeInTheDocument();
  });

  it("posts nothing until the last question is answered", () => {
    const action = vi.fn(() => Promise.resolve(CLEAN));
    render(<SignInForm action={action} />);
    toThePasswordStep();
    expect(action).not.toHaveBeenCalled();
  });

  it("posts every answer once, as one FormData, when the last is given", async () => {
    const action = vi.fn((_previous: FormState, _form: FormData) =>
      Promise.resolve(CLEAN),
    );
    render(<SignInForm action={action} />);
    toThePasswordStep();
    type("Password", " pw with spaces ");
    await inAct(() => press("Sign in"));

    expect(action).toHaveBeenCalledTimes(1);
    const form = action.mock.calls[0]?.[1] as FormData;
    expect(form.get("email")).toBe("ada@acme.test");
    // Not trimmed: a password's leading or trailing space is part of it.
    expect(form.get("password")).toBe(" pw with spaces ");
  });

  it("keeps both answers when the person goes Back and forward again", () => {
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    toThePasswordStep();
    type("Password", "pw");
    press("Back");
    expect(screen.getByLabelText("Work email")).toHaveValue("ada@acme.test");
    press("Continue");
    expect(screen.getByLabelText("Password")).toHaveValue("pw");
  });

  it("hands a password manager the account name without asking for it again", () => {
    const { container } = render(
      <SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />,
    );
    // On the email step there is nothing to carry yet.
    expect(
      container.querySelector('input[autocomplete="username"].sr-only'),
    ).toBeNull();
    toThePasswordStep();
    const carried = container.querySelector(
      'input.sr-only[autocomplete="username"]',
    ) as HTMLInputElement;
    expect(carried.value).toBe("ada@acme.test");
    expect(carried).toHaveAttribute("aria-hidden", "true");
    expect(carried.tabIndex).toBe(-1);
    // The visible questions on this step: the password only.
    expect(
      screen
        .getAllByLabelText(/./)
        .filter((el) => !el.classList.contains("sr-only")),
    ).toHaveLength(1);
  });
});

describe("a refusal", () => {
  it("is shown exactly once, with the request id beside it", async () => {
    // Exactly once: the first draft of the sign-in rendered the same sentence
    // through FieldError AND through Alert.
    render(
      <SignInForm action={refuses("We could not sign you in.", "req_01J8")} />,
    );
    toThePasswordStep();
    type("Password", "pw");
    await inAct(() => press("Sign in"));

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("We could not sign you in.");
    expect(screen.getAllByText("We could not sign you in.")).toHaveLength(1);
    expect(alert).toHaveTextContent("req_01J8");
  });

  it("marks the question it was given to as invalid, and keeps what was typed", async () => {
    render(<SignInForm action={refuses("Refused.")} />);
    toThePasswordStep();
    type("Password", "pw");
    await inAct(() => press("Sign in"));
    await screen.findByRole("alert");

    expect(screen.getByLabelText("Password")).toHaveAttribute(
      "aria-invalid",
      "true",
    );
    // Controlled inputs: React 19 resets an uncontrolled form after its action
    // settles, which would empty the field a refused password was typed into.
    expect(screen.getByLabelText("Password")).toHaveValue("pw");
  });

  it("disables the question and both buttons while a submit is in flight", async () => {
    render(<SignInForm action={neverSettles()} />);
    toThePasswordStep();
    type("Password", "pw");
    await inAct(() => press("Sign in"));

    expect(
      await screen.findByRole("button", { name: "Signing in…" }),
    ).toBeDisabled();
    expect(screen.getByLabelText("Password")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
  });
});

describe("the code question", () => {
  it("is one screen: six digits, the platform's one-time code, a numeric keypad", () => {
    render(<TotpForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    const code = screen.getByLabelText("One-time code");
    expect(code).toHaveAttribute("maxlength", "6");
    expect(code).toHaveAttribute("inputmode", "numeric");
    expect(code).toHaveAttribute("autocomplete", "one-time-code");
    expect(screen.queryByText(/Question \d of/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
  });

  it("is Continue-disabled until there are six digits", () => {
    render(<TotpForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    for (const partial of ["", "12345", "12345a", "1234567"]) {
      type("One-time code", partial);
      expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    }
    type("One-time code", "123456");
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
  });

  it("disables the field while a submit is in flight — THE replay case", async () => {
    // The code is verified behind a compare-and-swap replay guard, so the
    // SECOND submission of one code is refused on purpose. Without this a
    // double click shows "we could not sign you in" to somebody who typed a
    // perfectly good code.
    render(<TotpForm action={neverSettles()} />);
    type("One-time code", "123456");
    await inAct(() => press("Continue"));

    expect(
      await screen.findByRole("button", { name: "Checking…" }),
    ).toBeDisabled();
    expect(screen.getByLabelText("One-time code")).toBeDisabled();
  });

  it("does not call the action twice for a double submit", async () => {
    const action = neverSettles();
    render(<TotpForm action={action} />);
    type("One-time code", "123456");
    const form = screen
      .getByLabelText("One-time code")
      .closest("form") as HTMLFormElement;
    await inAct(() => {
      fireEvent.submit(form);
      fireEvent.submit(form);
    });
    expect(action).toHaveBeenCalledTimes(1);
  });

  it("says it is finishing an enrolment when it is", () => {
    render(<TotpForm action={vi.fn(() => Promise.resolve(CLEAN))} enrolling />);
    expect(
      screen.getByRole("button", { name: /finish enrolment/i }),
    ).toBeInTheDocument();
  });
});

describe("the password change", () => {
  function toThe(step: "new" | "repeat") {
    type("Current password", "printed-one");
    press("Continue");
    if (step === "repeat") {
      type("New password", "a-much-longer-one");
      press("Continue");
    }
  }

  it("asks the CURRENT password first, then the new one, then the new one again — three screens", () => {
    // This asserted the opposite until 2026-09-10 ("and for no current one"):
    // the session has presented both factors once, up to twelve hours earlier,
    // which made the cookie on its own the credential for an irreversible
    // account takeover (issue #79 item 3).
    render(<PasswordForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    expect(screen.getByLabelText("Current password")).toBeInTheDocument();
    expect(screen.getByText("Question 1 of 3")).toBeInTheDocument();
    toThe("new");
    expect(screen.getByLabelText("New password")).toBeInTheDocument();
    expect(screen.getByText("Question 2 of 3")).toBeInTheDocument();
    type("New password", "a-much-longer-one");
    press("Continue");
    expect(screen.getByLabelText("Repeat it")).toBeInTheDocument();
    expect(screen.getByText("Question 3 of 3")).toBeInTheDocument();
  });

  it("lets a password manager offer the stored password rather than a new one", () => {
    render(<PasswordForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    expect(screen.getByLabelText("Current password")).toHaveAttribute(
      "autocomplete",
      "current-password",
    );
    toThe("new");
    expect(screen.getByLabelText("New password")).toHaveAttribute(
      "autocomplete",
      "new-password",
    );
  });

  it("holds the new password to the server's own minimum length", () => {
    render(<PasswordForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    toThe("new");
    type("New password", "eleven-char");
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    type("New password", "twelve-chars");
    expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
  });

  it("holds the repeat to the new password, because vpay sees only one", () => {
    render(<PasswordForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    toThe("repeat");
    type("Repeat it", "a-much-longer-two");
    expect(screen.getByRole("button", { name: "Set password" })).toBeDisabled();
    type("Repeat it", "a-much-longer-one");
    expect(screen.getByRole("button", { name: "Set password" })).toBeEnabled();
  });

  it("posts all three answers under the names the server action reads", async () => {
    const action = vi.fn((_previous: FormState, _form: FormData) =>
      Promise.resolve(CLEAN),
    );
    render(<PasswordForm action={action} />);
    toThe("repeat");
    type("Repeat it", "a-much-longer-one");
    await inAct(() => press("Set password"));
    const form = action.mock.calls[0]?.[1] as FormData;
    expect(form.get("current_password")).toBe("printed-one");
    expect(form.get("new_password")).toBe("a-much-longer-one");
    expect(form.get("confirm_password")).toBe("a-much-longer-one");
  });
});
