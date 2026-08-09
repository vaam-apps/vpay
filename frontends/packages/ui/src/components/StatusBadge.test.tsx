import { render, screen } from '@testing-library/react';
import { PAYMENT_STATUS, statusLabel } from '@vpay/tokens';
import { describe, expect, it } from 'vitest';

import { StatusBadge } from './StatusBadge.js';

describe('StatusBadge', () => {
  it('renders the shared label for every status', () => {
    for (const s of PAYMENT_STATUS) {
      const { unmount } = render(<StatusBadge status={s} />);
      expect(screen.getByText(statusLabel[s])).toBeTruthy();
      unmount();
    }
  });

  it('exposes the raw status for tests and e2e selectors', () => {
    render(<StatusBadge status="processing" />);
    expect(screen.getByText(statusLabel.processing).getAttribute('data-status')).toBe(
      'processing',
    );
  });

  it('applies the success tone only to succeeded', () => {
    const { container, unmount } = render(<StatusBadge status="succeeded" />);
    expect(container.querySelector('.badge-success')).not.toBeNull();
    unmount();

    const other = render(<StatusBadge status="processing" />);
    expect(other.container.querySelector('.badge-success')).toBeNull();
  });
});
