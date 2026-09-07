import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Table } from './table';

describe('Table', () => {
  it('renders table-zebra when zebra is set', () => {
    render(
      <Table zebra data-testid="table">
        <tbody>
          <tr>
            <td>Row</td>
          </tr>
        </tbody>
      </Table>,
    );
    expect(screen.getByTestId('table').className).toContain('table-zebra');
  });
});
