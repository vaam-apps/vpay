import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Heading, List, PageShell, Stack, Text } from './layout';

describe('layout primitives', () => {
  it('Heading renders the requested level with the shared typography classes', () => {
    render(<Heading level={2}>Title</Heading>);
    const el = screen.getByRole('heading', { level: 2, name: 'Title' });
    expect(el.className).toContain('text-xl');
  });

  it('Stack applies its direction, align and gap variants', () => {
    render(
      <Stack direction="column" gap="lg" data-testid="stack">
        <span>a</span>
      </Stack>,
    );
    const el = screen.getByTestId('stack');
    expect(el.className).toContain('flex-col');
    expect(el.className).toContain('gap-6');
  });

  it('Stack renders the element it is asked for, so a landmark survives', () => {
    // A stack that IS the page header is a `banner` landmark. Rendering it
    // as a `<div>` removes that landmark with no test and no lint failing,
    // which is how `checkout-view.tsx` and `return-view.tsx` lost theirs.
    render(
      <Stack as="header" justify="between" data-testid="header">
        <span>a</span>
      </Stack>,
    );
    const el = screen.getByTestId('header');
    expect(el.tagName).toBe('HEADER');
    expect(screen.getByRole('banner')).toBe(el);
    expect(el.className).toContain('justify-between');
  });

  it('Text applies tone and size without ever writing a raw opacity/text-error inline', () => {
    render(
      <Text tone="muted" size="xs">
        Support line
      </Text>,
    );
    const el = screen.getByText('Support line');
    expect(el.className).toContain('opacity-60');
    expect(el.className).toContain('text-xs');
  });

  it('List and PageShell render their fixed layout classes', () => {
    render(
      <PageShell data-testid="shell">
        <List data-testid="list">
          <li>Item</li>
        </List>
      </PageShell>,
    );
    expect(screen.getByTestId('shell').className).toContain('max-w-md');
    expect(screen.getByTestId('list').className).toContain('space-y-1');
  });
});
