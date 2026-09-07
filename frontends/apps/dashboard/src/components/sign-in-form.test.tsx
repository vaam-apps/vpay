import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { FormState } from '../form-state';
import { SignInForm } from './sign-in-form';
import { TotpForm } from './totp-form';
import { PasswordForm } from './password-form';

const CLEAN: FormState = { error: null, requestId: null };

/** An action that never settles, so the form stays in its pending state. */
function neverSettles() {
  return vi.fn(() => new Promise<FormState>(() => {}));
}

/** An action that refuses with a sentence and an id. */
function refuses(error: string, requestId: string | null = null) {
  return vi.fn(() => Promise.resolve({ error, requestId }));
}

describe('the password form', () => {
  it('asks for an email and a password, and for no code', () => {
    // Two legs, not one: the code is the next request, authorised by the
    // session this one creates.
    render(<SignInForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    expect(screen.getByLabelText('Work email')).toBeInTheDocument();
    expect(screen.getByLabelText('Password')).toHaveAttribute('type', 'password');
    expect(screen.queryByLabelText(/code/i)).toBeNull();
  });

  it('shows a refusal exactly once, with the request id beside it', async () => {
    // Exactly once: the first draft of this form rendered the same sentence
    // through FieldError AND through Alert, so every failed sign-in printed
    // it twice (docs/plans/exp26-notes/lane-d-review.md).
    render(<SignInForm action={refuses('We could not sign you in.', 'req_01J8')} />);
    fireEvent.submit(screen.getByRole('button', { name: 'Sign in' }).closest('form') as HTMLFormElement);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('We could not sign you in.');
    expect(screen.getAllByText('We could not sign you in.')).toHaveLength(1);
    expect(alert).toHaveTextContent('req_01J8');
  });

  it('marks both fields invalid through Field own validation state', async () => {
    render(<SignInForm action={refuses('Refused.')} />);
    fireEvent.submit(screen.getByRole('button', { name: 'Sign in' }).closest('form') as HTMLFormElement);
    await screen.findByRole('alert');

    expect(screen.getByLabelText('Work email')).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByLabelText('Password')).toHaveAttribute('aria-invalid', 'true');
  });

  it('disables every control while a submit is in flight', async () => {
    render(<SignInForm action={neverSettles()} />);
    const button = screen.getByRole('button', { name: 'Sign in' });
    fireEvent.submit(button.closest('form') as HTMLFormElement);

    expect(await screen.findByRole('button', { name: 'Signing in…' })).toBeDisabled();
    expect(screen.getByLabelText('Work email')).toBeDisabled();
    expect(screen.getByLabelText('Password')).toBeDisabled();
  });
});

describe('the code form', () => {
  it('constrains the field to six digits and offers the platform one-time code', () => {
    render(<TotpForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    const code = screen.getByLabelText('One-time code');
    expect(code).toHaveAttribute('maxlength', '6');
    expect(code).toHaveAttribute('pattern', '[0-9]{6}');
    expect(code).toHaveAttribute('inputmode', 'numeric');
    expect(code).toHaveAttribute('autocomplete', 'one-time-code');
  });

  it('disables the field while a submit is in flight — THE replay case', async () => {
    // The code is verified behind a compare-and-swap replay guard, so the
    // SECOND submission of one code is refused on purpose. Without this a
    // double click shows "we could not sign you in" to somebody who typed a
    // perfectly good code.
    render(<TotpForm action={neverSettles()} />);
    fireEvent.submit(screen.getByRole('button', { name: 'Continue' }).closest('form') as HTMLFormElement);

    expect(await screen.findByRole('button', { name: 'Checking…' })).toBeDisabled();
    expect(screen.getByLabelText('One-time code')).toBeDisabled();
  });

  it('says it is finishing an enrolment when it is', () => {
    render(<TotpForm action={vi.fn(() => Promise.resolve(CLEAN))} enrolling />);
    expect(screen.getByRole('button', { name: /finish enrolment/i })).toBeInTheDocument();
  });
});

describe('the password-change form', () => {
  it('asks for the new password twice and for no current one', () => {
    // No "current password" field: the session has already presented both
    // factors, and re-checking a password this endpoint then overwrites
    // would be a second copy of that check in the wrong layer.
    render(<PasswordForm action={vi.fn(() => Promise.resolve(CLEAN))} />);
    expect(screen.getByLabelText('New password')).toBeInTheDocument();
    expect(screen.getByLabelText('Repeat it')).toBeInTheDocument();
    expect(screen.queryByLabelText(/current/i)).toBeNull();
  });

  it('hints the server own minimum length', () => {
    expect(
      (render(<PasswordForm action={vi.fn(() => Promise.resolve(CLEAN))} />).container.querySelector(
        '#dashboard-new-password',
      ) as HTMLInputElement).minLength,
    ).toBe(12);
  });
});
