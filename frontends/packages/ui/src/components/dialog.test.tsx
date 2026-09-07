import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Dialog } from './dialog.js';

describe('Dialog', () => {
  it('opens from its trigger and closes from Dialog.Close', () => {
    const onOpenChange = vi.fn();
    const { unmount } = render(
      <Dialog.Root onOpenChange={onOpenChange}>
        <Dialog.Trigger>Open</Dialog.Trigger>
        <Dialog.Portal>
          <Dialog.Popup>
            <Dialog.Title>Sign-in failed</Dialog.Title>
            <Dialog.Description>Check the code and try again.</Dialog.Description>
            <Dialog.Close>Dismiss</Dialog.Close>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>,
    );

    expect(screen.queryByText('Sign-in failed')).toBeNull();
    fireEvent.click(screen.getByText('Open'));
    expect(screen.getByRole('dialog', { name: 'Sign-in failed' })).toBeTruthy();

    fireEvent.click(screen.getByText('Dismiss'));
    expect(onOpenChange).toHaveBeenCalledWith(false, expect.anything());
    unmount();
  });
});
