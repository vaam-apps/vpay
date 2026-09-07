import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Checkbox } from './checkbox.js';

describe('Checkbox', () => {
  it('renders a native button (decision D2), not a span', () => {
    const { unmount } = render(<Checkbox aria-label="Remember this number" />);
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.tagName).toBe('BUTTON');
    expect(el.className).toContain('checkbox');
    unmount();
  });

  it('toggles aria-checked on click, which daisyUI 5 styles directly', () => {
    render(<Checkbox aria-label="Remember this number" />);
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(el);
    expect(el.getAttribute('aria-checked')).toBe('true');
  });
});
