import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Spinner } from './spinner.js';

describe('Spinner', () => {
  it('is aria-hidden and renders a visually-hidden label for screen readers', () => {
    const { container } = render(<Spinner label="Waiting for the payer" />);
    const visual = container.querySelector('[aria-hidden]');
    expect(visual).not.toBeNull();
    expect(visual?.className).toContain('loading');
    expect(screen.getByText('Waiting for the payer').className).toContain('sr-only');
  });
});
