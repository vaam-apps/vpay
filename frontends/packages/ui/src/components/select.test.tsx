import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Select } from "./select";

const LOCALES = [
  { value: "en", label: "English" },
  { value: "fr", label: "Français" },
];

describe("Select", () => {
  it("opens the popup and reports the chosen item", async () => {
    const onValueChange = vi.fn();
    const { unmount } = render(
      <Select
        items={LOCALES}
        defaultValue="en"
        onValueChange={onValueChange}
        aria-label="Locale"
      />,
    );
    const trigger = screen.getByRole("combobox", { name: "Locale" });
    expect(trigger.textContent).toBe("English");

    fireEvent.click(trigger);
    // `findByRole`, not `getByRole`: Base UI mounts the popup into a portal
    // and the items with it, which is not guaranteed to have happened by the
    // time `fireEvent` returns. See the docblock on the case below — every
    // assertion in this file that follows an interaction waits for its
    // condition rather than assuming the schedule that produced it.
    const option = await screen.findByRole("option", { name: "Français" });
    // Base UI's Select.Item only commits a click that was preceded by a
    // pointerdown on the same item — it is how a real click is
    // distinguished from a click event fired by whatever opened the popup.
    fireEvent.pointerDown(option, { pointerType: "mouse" });
    fireEvent.click(option, { detail: 1 });
    await waitFor(() => {
      expect(onValueChange).toHaveBeenCalledWith("fr");
    });
    unmount();
  });

  /**
   * Base UI's `Select` is a button trigger plus a portalled listbox, not a
   * native `<select>` — so every keyboard affordance a native control gets
   * for free is the primitive's to provide, and this is the case that fails
   * if the trigger is ever swapped for a styled `<div>`.
   *
   * Committing the highlighted item with Enter is deliberately NOT asserted:
   * measured in this environment, Base UI does not commit on a synthetic
   * `keydown` — on the list, on the option, on the trigger or on
   * `document.activeElement` — and a case that fires Enter and then asserts
   * nothing changed would be a case asserting the harness. Real-browser
   * commit is Cypress's, plan §7 row 6.
   */
  it("opens on ArrowDown from the trigger and moves the highlight with the arrows", async () => {
    const { unmount } = render(
      <Select items={LOCALES} defaultValue="en" aria-label="Locale" />,
    );
    const trigger = screen.getByRole("combobox", { name: "Locale" });
    expect(screen.queryByRole("listbox")).toBeNull();

    trigger.focus();
    fireEvent.keyDown(trigger, { key: "ArrowDown", code: "ArrowDown" });

    const list = await screen.findByRole("listbox");
    const english = await screen.findByRole("option", { name: "English" });
    const french = await screen.findByRole("option", { name: "Français" });

    // The selected item is the one the keyboard lands on, not the first.
    expect(english.getAttribute("aria-selected")).toBe("true");
    // `waitFor`, not a bare `expect`: Base UI moves focus onto the
    // highlighted item AFTER the listbox is in the document, so
    // `findByRole('listbox')` above can resolve a tick before focus lands.
    // Written as a bare assertion this passed on vitest 3 and failed six
    // runs in ten on vitest 4 (exp41 review) — the schedule changed, the
    // component did not. It had also been flaking under load on branches
    // that touch no frontend code at all, on vitest 3, always green in
    // isolation, which is the same race seen from the other side: this file
    // was assuming a schedule rather than waiting for a condition. What is
    // asserted is unchanged: focus must reach this item, and `waitFor` still
    // fails, naming the element it found instead, if it never does.
    await waitFor(() => {
      expect(document.activeElement).toBe(english);
    });

    fireEvent.keyDown(list, { key: "ArrowDown", code: "ArrowDown" });
    await waitFor(() => {
      expect(document.activeElement).toBe(french);
    });
    unmount();
  });
});
