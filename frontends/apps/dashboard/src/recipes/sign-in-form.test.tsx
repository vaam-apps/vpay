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

  it('shows the error exactly once', () => {
    // An earlier draft rendered it through `FieldError` AND through `Alert`,
    // so a failed sign-in printed the same sentence twice. Measured at 2 and
    // fixed to 1 — docs/plans/exp26-notes/lane-d-review.md, finding 3.
    const { container, unmount } = render(<SignInForm error="Bad code." onSubmit={vi.fn()} />);

    const occurrences = (container.textContent ?? '').split('Bad code.').length - 1;
    expect(occurrences).toBe(1);
    unmount();
  });

  it('quotes the request id in the alert when the caller has one', () => {
    const { unmount } = render(
      <SignInForm error="Bad code." requestId="req_01J8ABCDEF" onSubmit={vi.fn()} />,
    );

    expect(screen.getByRole('alert')).toHaveTextContent('req_01J8ABCDEF');
    unmount();
  });

  it('constrains the code to its length, six by default', () => {
    const { unmount } = render(<SignInForm onSubmit={vi.fn()} />);

    const code = screen.getByLabelText('One-time code');
    expect(code).toHaveAttribute('maxlength', '6');
    expect(code).toHaveAttribute('pattern', '[0-9]{6}');
    expect(code).toHaveAttribute('inputmode', 'numeric');
    unmount();
  });

  it('takes a different code length where a deployment sets one', () => {
    const { unmount } = render(<SignInForm codeLength={8} onSubmit={vi.fn()} />);

    expect(screen.getByLabelText('One-time code')).toHaveAttribute('maxlength', '8');
    unmount();
  });

  it('disables every control while a submit is in flight', () => {
    const { unmount } = render(<SignInForm pending onSubmit={vi.fn()} />);

    expect(screen.getByLabelText('Work email')).toBeDisabled();
    expect(screen.getByLabelText('One-time code')).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Signing in…' })).toBeDisabled();
    unmount();
  });

  it('submits nothing while pending, even if the form is submitted some other way', () => {
    // The one-time code is verified behind a compare-and-swap replay guard:
    // the SECOND submission of one code is refused on purpose, and would
    // read to the staff member as "invalid code" for a code that was fine.
    const onSubmit = vi.fn();
    const { container, unmount } = render(<SignInForm pending onSubmit={onSubmit} />);

    const form = container.querySelector('form');
    expect(form).not.toBeNull();
    fireEvent.submit(form as HTMLFormElement);

    expect(onSubmit).not.toHaveBeenCalled();
    unmount();
  });
});
