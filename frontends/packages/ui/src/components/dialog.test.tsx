import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Dialog } from './dialog';

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

  /**
   * The two things a modal owes a keyboard user, neither of which this
   * wrapper implements — they are `@base-ui/react/dialog`'s, which is the
   * reason for taking a Base UI primitive rather than styling a `<div>`.
   * Untested, "Base UI does it" is a claim; here it is a measurement, and it
   * is the one that fails if someone later swaps the primitive out for
   * daisyUI's CSS-only `modal` markup, which has neither.
   */
  it('moves focus into the popup when it opens', async () => {
    const { unmount } = render(
      <Dialog.Root defaultOpen>
        <Dialog.Portal>
          <Dialog.Popup>
            <Dialog.Title>Sign-in failed</Dialog.Title>
            <button type="button">Retry</button>
            <Dialog.Close>Dismiss</Dialog.Close>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>,
    );
    const popup = screen.getByRole('dialog', { name: 'Sign-in failed' });
    await waitFor(() => {
      expect(popup.contains(document.activeElement)).toBe(true);
    });
    unmount();
  });

  it('closes on Escape', () => {
    const onOpenChange = vi.fn();
    const { unmount } = render(
      <Dialog.Root defaultOpen onOpenChange={onOpenChange}>
        <Dialog.Portal>
          <Dialog.Popup>
            <Dialog.Title>Sign-in failed</Dialog.Title>
            <Dialog.Close>Dismiss</Dialog.Close>
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>,
    );
    expect(screen.getByRole('dialog', { name: 'Sign-in failed' })).toBeTruthy();
    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(onOpenChange).toHaveBeenCalledWith(false, expect.anything());
    unmount();
  });
});
