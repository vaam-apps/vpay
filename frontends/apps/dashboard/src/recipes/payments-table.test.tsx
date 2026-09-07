import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { PaymentsTable } from './payments-table';

describe('PaymentsTable', () => {
  it('renders one status pill per row, from @vpay/tokens tone and copy', () => {
    // Deliberately obvious placeholder ids — this is a rendering test for
    // the recipe, not a fixture anyone could mistake for a real payment.
    const { unmount } = render(
      <PaymentsTable
        rows={[
          { id: 'pi_example_1', amount: '$12.00', status: 'succeeded' },
          { id: 'pi_example_2', amount: '$4.50', status: 'processing' },
        ]}
      />,
    );

    expect(screen.getByRole('table')).toBeInTheDocument();
    expect(screen.getByText('pi_example_1').closest('tr')).toContainElement(
      screen.getByText('Succeeded'),
    );
    const succeeded = document.querySelector('[data-status="succeeded"]');
    const processing = document.querySelector('[data-status="processing"]');
    expect(succeeded).not.toBeNull();
    expect(processing).not.toBeNull();
    expect(succeeded?.className).toContain('badge-success');
    expect(processing?.className).toContain('badge-info');
    unmount();
  });
});
