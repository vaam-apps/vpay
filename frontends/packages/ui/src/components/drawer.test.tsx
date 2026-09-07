import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { Drawer } from './drawer';

/**
 * `PayerSheet` (this component's predecessor) shipped with no test at all —
 * recorded in docs/status.md as "Renders; no test". Plan §5 Lane A step 5
 * calls that out by name as something this rewrite must not repeat.
 */
describe('Drawer', () => {
  it('renders nothing when closed, and its title and content when open', () => {
    const { rerender, unmount } = render(
      <Drawer open={false} onOpenChange={() => {}} title="Charge detail">
        <p>Charge body</p>
      </Drawer>,
    );
    expect(screen.queryByText('Charge detail')).toBeNull();
    expect(screen.queryByText('Charge body')).toBeNull();

    rerender(
      <Drawer open onOpenChange={() => {}} title="Charge detail">
        <p>Charge body</p>
      </Drawer>,
    );
    expect(screen.getByText('Charge detail')).toBeTruthy();
    expect(screen.getByText('Charge body')).toBeTruthy();
    unmount();
  });

  it('calls onOpenChange(false) when the backdrop is dismissed with Escape', () => {
    const onOpenChange = vi.fn();

    function Controlled() {
      const [open, setOpen] = useState(true);
      return (
        <Drawer
          open={open}
          onOpenChange={(next) => {
            setOpen(next);
            onOpenChange(next);
          }}
          title="Charge detail"
        >
          <p>Charge body</p>
        </Drawer>
      );
    }
    const { unmount } = render(<Controlled />);

    expect(screen.getByText('Charge detail')).toBeTruthy();
    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });
    expect(onOpenChange).toHaveBeenCalledWith(false);
    unmount();
  });
});
