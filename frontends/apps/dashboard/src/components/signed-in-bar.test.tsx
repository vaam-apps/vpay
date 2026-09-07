import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { SignedInBar } from './signed-in-bar';

describe('the signed-in bar', () => {
  it('names the staff member and the one tenant this dashboard reads', () => {
    // The merchant is on screen because it is not a preference: /dash/v1
    // reads exactly one tenant, fixed by the client registration. An
    // operator looking at an empty list has to be able to tell "no payments"
    // from "wrong merchant", and nothing else on the page answers that.
    render(
      <SignedInBar email="ada@example.test" merchantId="demo-merchant-tenant" signOut={vi.fn()} />,
    );
    expect(screen.getByText('ada@example.test')).toBeInTheDocument();
    expect(screen.getByText('demo-merchant-tenant')).toBeInTheDocument();
  });

  it('signs out with a form POST, never a link', () => {
    // A sign-out link is something a prefetcher, a link scanner or an
    // <img src> in a chat message can fire.
    render(
      <SignedInBar email="ada@example.test" merchantId="demo-merchant-tenant" signOut={vi.fn()} />,
    );
    const button = screen.getByRole('button', { name: 'Sign out' });
    expect(button).toHaveAttribute('type', 'submit');
    expect(screen.queryByRole('link', { name: /sign out/i })).toBeNull();
  });
});
