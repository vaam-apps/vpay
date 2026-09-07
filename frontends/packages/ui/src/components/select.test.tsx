import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Select } from './select';

const LOCALES = [
  { value: 'en', label: 'English' },
  { value: 'fr', label: 'Français' },
];

describe('Select', () => {
  it('opens the popup and reports the chosen item', () => {
    const onValueChange = vi.fn();
    const { unmount } = render(
      <Select items={LOCALES} defaultValue="en" onValueChange={onValueChange} aria-label="Locale" />,
    );
    const trigger = screen.getByRole('combobox', { name: 'Locale' });
    expect(trigger.textContent).toBe('English');

    fireEvent.click(trigger);
    const option = screen.getByRole('option', { name: 'Français' });
    // Base UI's Select.Item only commits a click that was preceded by a
    // pointerdown on the same item — it is how a real click is
    // distinguished from a click event fired by whatever opened the popup.
    fireEvent.pointerDown(option, { pointerType: 'mouse' });
    fireEvent.click(option, { detail: 1 });
    expect(onValueChange).toHaveBeenCalledWith('fr');
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
  it('opens on ArrowDown from the trigger and moves the highlight with the arrows', async () => {
    const { unmount } = render(
      <Select items={LOCALES} defaultValue="en" aria-label="Locale" />,
    );
    const trigger = screen.getByRole('combobox', { name: 'Locale' });
    expect(screen.queryByRole('listbox')).toBeNull();

    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'ArrowDown', code: 'ArrowDown' });

    const list = await screen.findByRole('listbox');
    const english = screen.getByRole('option', { name: 'English' });
    const french = screen.getByRole('option', { name: 'Français' });

    // The selected item is the one the keyboard lands on, not the first.
    expect(english.getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(english);

    fireEvent.keyDown(list, { key: 'ArrowDown', code: 'ArrowDown' });
    expect(document.activeElement).toBe(french);
    unmount();
  });
});
