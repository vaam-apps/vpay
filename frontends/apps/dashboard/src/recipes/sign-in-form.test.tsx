import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { SignInForm } from './sign-in-form';

describe('SignInForm', () => {
  it('submits the email and code from a plain FormData read', () => {
    const onSubmit = vi.fn();
    const { unmount } = render(<SignInForm onSubmit={onSubmit} />);

    fireEvent.change(screen.getByLabelText('Work email'), {
      target: { value: 'ops@example.com' },
    });
    fireEvent.change(screen.getByLabelText('One-time code'), {
      target: { value: '123456' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Sign in' }));

    expect(onSubmit).toHaveBeenCalledWith({ email: 'ops@example.com', code: '123456' });
    unmount();
  });

  it('shows the server error as an alert and marks both fields invalid', () => {
    const { unmount } = render(<SignInForm error="That code has expired." onSubmit={vi.fn()} />);

    expect(screen.getByRole('alert')).toHaveTextContent('That code has expired.');
    expect(screen.getByLabelText('Work email')).toHaveAttribute('aria-invalid', 'true');
    unmount();
  });
});
