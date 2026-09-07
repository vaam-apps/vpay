import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Button } from './button.js';

describe('Button', () => {
  it('defaults to btn btn-primary, type=button', () => {
    render(<Button>Pay</Button>);
    const el = screen.getByRole('button', { name: 'Pay' });
    expect(el.className.split(' ')).toEqual(expect.arrayContaining(['btn', 'btn-primary']));
    expect(el.getAttribute('type')).toBe('button');
  });

  it('variant and size compose without dropping either class', () => {
    render(
      <Button variant="outline" size="sm">
        Back
      </Button>,
    );
    const el = screen.getByRole('button', { name: 'Back' });
    expect(el.className).toContain('btn-outline');
    expect(el.className).toContain('btn-sm');
    expect(el.className).not.toContain('btn-primary');
  });

  it('render composes another element (the shop "pay without leaving" link)', () => {
    render(
      <Button render={<a href="/orders/1/embedded" />} nativeButton={false} variant="outline">
        Pay this order without leaving the shop
      </Button>,
    );
    // Base UI's Button always exposes an ARIA `role="button"`, even when
    // `render` swaps in an `<a>` — an anchor styled as a button acts as one.
    const el = screen.getByRole('button', { name: 'Pay this order without leaving the shop' });
    expect(el.tagName).toBe('A');
    expect(el.getAttribute('href')).toBe('/orders/1/embedded');
    expect(el.className).toContain('btn-outline');
  });
});
