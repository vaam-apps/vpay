import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Alert } from './alert';

describe('Alert', () => {
  it('renders role=alert and the tone class, defaulting to neutral', () => {
    const { unmount } = render(<Alert>Saved</Alert>);
    const el = screen.getByRole('alert');
    expect(el.className).toContain('alert');
    expect(el.className).not.toMatch(/alert-(info|success|warning|error)/);
    unmount();
  });

  it.each([
    ['success', 'alert-success'],
    ['warning', 'alert-warning'],
    ['error', 'alert-error'],
  ] as const)('tone=%s renders %s', (tone, expected) => {
    const { unmount } = render(<Alert tone={tone}>Message</Alert>);
    expect(screen.getByRole('alert').className).toContain(expected);
    unmount();
  });
});
