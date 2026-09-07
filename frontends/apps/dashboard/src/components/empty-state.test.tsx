import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { EmptyState } from './empty-state';

describe('EmptyState', () => {
  it('renders the title as a heading and the description beneath it', () => {
    const { unmount } = render(
      <EmptyState title="No payments match these filters" description="Try widening the date range." />,
    );

    expect(screen.getByRole('heading', { name: 'No payments match these filters' })).toBeInTheDocument();
    expect(screen.getByText('Try widening the date range.')).toBeInTheDocument();
    unmount();
  });
});
