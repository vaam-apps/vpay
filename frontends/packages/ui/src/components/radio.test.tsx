import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Radio, RadioGroup } from './radio';

describe('RadioGroup / Radio', () => {
  it('lets exactly one radio in the group become checked', () => {
    const onValueChange = vi.fn();
    const { unmount } = render(
      <RadioGroup aria-label="Surface" onValueChange={onValueChange}>
        <Radio value="hosted" aria-label="Hosted" />
        <Radio value="embedded" aria-label="Embedded" />
      </RadioGroup>,
    );
    const hosted = screen.getByRole('radio', { name: 'Hosted' });
    const embedded = screen.getByRole('radio', { name: 'Embedded' });
    expect(hosted.getAttribute('aria-checked')).toBe('false');

    fireEvent.click(embedded);
    expect(onValueChange).toHaveBeenCalledWith('embedded', expect.anything());
    expect(embedded.getAttribute('aria-checked')).toBe('true');
    expect(hosted.getAttribute('aria-checked')).toBe('false');
    unmount();
  });
});
