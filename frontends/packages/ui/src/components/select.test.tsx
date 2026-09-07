import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Select } from './select.js';

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
});
