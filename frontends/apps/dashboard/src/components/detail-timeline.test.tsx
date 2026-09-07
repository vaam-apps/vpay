import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { DetailTimeline } from './detail-timeline';

describe('DetailTimeline', () => {
  it('renders one list item per event, label and timestamp both', () => {
    const { unmount } = render(
      <DetailTimeline
        events={[
          { id: 'evt_1', at: '2026-09-07T09:00:00Z', label: 'PaymentIntent created' },
          { id: 'evt_2', at: '2026-09-07T09:00:04Z', label: 'Charge succeeded' },
        ]}
      />,
    );

    const items = screen.getAllByRole('listitem');
    expect(items).toHaveLength(2);
    expect(screen.getByText('PaymentIntent created')).toBeInTheDocument();
    expect(screen.getByText('Charge succeeded')).toBeInTheDocument();
    expect(screen.getByText('2026-09-07T09:00:04Z')).toBeInTheDocument();
    unmount();
  });

  it('states plainly that there are no events, rather than rendering an empty list', () => {
    const { unmount } = render(<DetailTimeline events={[]} />);

    expect(screen.queryByRole('list')).not.toBeInTheDocument();
    expect(screen.getByText('No events yet.')).toBeInTheDocument();
    unmount();
  });
});
