import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Checkbox } from './checkbox';

describe('Checkbox', () => {
  it('renders a native button (decision D2), not a span', () => {
    const { unmount } = render(<Checkbox aria-label="Remember this number" />);
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.tagName).toBe('BUTTON');
    expect(el.className).toContain('checkbox');
    unmount();
  });

  it('toggles aria-checked on click, which daisyUI 5 styles directly', () => {
    const { unmount } = render(<Checkbox aria-label="Remember this number" />);
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(el);
    expect(el.getAttribute('aria-checked')).toBe('true');
    unmount();
  });

  /**
   * The two behaviours decision D2 put at risk, and which the checkbox this
   * one replaces had explicit coverage for (docs/status.md, exp21: "the label
   * click and space key both *measured*"). A `<button role="checkbox">` is
   * not a form control, so neither is free: a label does not associate with
   * it the way it does with an `<input>`, and nothing about a `<span>` would
   * have given the browser's own Space/Enter activation back.
   */
  it('toggles when its wrapping label is clicked', () => {
    const { unmount } = render(
      <label>
        <Checkbox aria-label="Remember this number" /> Remember this number
      </label>,
    );
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(screen.getByText('Remember this number'));
    expect(el.getAttribute('aria-checked')).toBe('true');
    unmount();
  });

  it('toggles when a label associated by htmlFor is clicked', () => {
    const { unmount } = render(
      <>
        <Checkbox id="vpay-remember" />
        <label htmlFor="vpay-remember">Remember this number</label>
      </>,
    );
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.id).toBe('vpay-remember');
    fireEvent.click(screen.getByText('Remember this number'));
    expect(el.getAttribute('aria-checked')).toBe('true');
    unmount();
  });

  /**
   * The Space key itself cannot be asserted here and this test does not
   * pretend to: in a real browser Space and Enter on a `<button>` are turned
   * into a `click` by the user agent, and **jsdom does not implement that** —
   * measured, `fireEvent.keyDown/keyUp` with `key: ' '` leaves `aria-checked`
   * at `false`. Asserting it would be asserting jsdom.
   *
   * What can be asserted is the two halves the browser's behaviour is made
   * of, and together they are the whole of D2's claim: the control really is
   * a native `<button type="button">` (so the user agent supplies the
   * keystroke-to-click translation), and a click toggles it. If either half
   * regresses — a `<span>` comes back, or `type` becomes `submit` inside a
   * form — this fails. The keystroke itself belongs to the real-browser
   * check plan §7 row 6 still owes.
   */
  it('is a native button, which is what makes Space and Enter work in a browser', () => {
    const { unmount } = render(<Checkbox aria-label="Remember this number" />);
    const el = screen.getByRole('checkbox', { name: 'Remember this number' });
    expect(el.tagName).toBe('BUTTON');
    expect(el.getAttribute('type')).toBe('button');
    expect(el.getAttribute('tabindex')).toBe('0');
    fireEvent.click(el);
    expect(el.getAttribute('aria-checked')).toBe('true');
    unmount();
  });
});
