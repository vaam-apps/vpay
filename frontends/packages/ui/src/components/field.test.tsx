import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Field, FieldDescription, FieldError, FieldLabel } from './field.js';
import { Input } from './input.js';

describe('Field', () => {
  it('associates its label and description with the control via Base UI, not a manual id', () => {
    render(
      <Field className="gap-1">
        <FieldLabel>MSISDN</FieldLabel>
        <Input name="msisdn" />
        <FieldDescription>Include the country code.</FieldDescription>
      </Field>,
    );
    const input = screen.getByRole('textbox', { name: 'MSISDN' });
    expect(input.getAttribute('aria-describedby')).toBeTruthy();
  });

  it('shows a validation error via Field.Error, not a manually rendered <p>', () => {
    render(
      <Field className="gap-1" invalid>
        <FieldLabel>MSISDN</FieldLabel>
        <Input name="msisdn" required />
        <FieldError match>Enter a valid number</FieldError>
      </Field>,
    );
    expect(screen.getByText('Enter a valid number')).toBeTruthy();
  });
});
