import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Heading, List, PageShell, Stack, Text } from './layout.js';

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
