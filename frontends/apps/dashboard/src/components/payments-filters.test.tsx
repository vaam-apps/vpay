import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { PaymentsFilters } from './payments-filters';

const EMPTY = { status: '', createdFrom: '', createdTo: '' };

/** The `<form>` the component renders. */
function form(container: HTMLElement): HTMLFormElement {
  const element = container.querySelector('form');
  expect(element).not.toBeNull();
  return element as HTMLFormElement;
}

describe('the payments filters', () => {
  it('is a GET form, so the filters live in the URL', () => {
    // Which is what makes a filtered list bookmarkable, reloadable and
    // pasteable into a ticket — and what gives the server component behind it
    // one source of truth rather than a second copy in client state that can
    // disagree with the address bar.
    const { container } = render(<PaymentsFilters values={EMPTY} />);
    expect(form(container).getAttribute('method')).toBe('get');
  });

  it('submits the status as `status`, which is the parameter vpay reads', () => {
    // THE case this file exists for. `Select` is a Base UI listbox and not a
    // native `<select>`; if it did not put its value into the form, the status
    // filter would render, look right, be clickable, and silently do nothing —
    // every list would come back unfiltered and the screen would give no sign.
    // Asserted on `FormData`, which is what the browser actually submits.
    const { container } = render(
      <PaymentsFilters values={{ ...EMPTY, status: 'succeeded' }} />,
    );
    expect(new FormData(form(container)).get('status')).toBe('succeeded');
  });

  it('submits the two dates under the names the page reads', () => {
    const { container } = render(
      <PaymentsFilters values={{ status: '', createdFrom: '2026-09-01', createdTo: '2026-09-07' }} />,
    );
    const data = new FormData(form(container));
    // `created_from`/`created_to`, this app's own vocabulary — `payments-query.ts`
    // translates them into the API's `created_gte`/`created_lte` instants.
    expect(data.get('created_from')).toBe('2026-09-01');
    expect(data.get('created_to')).toBe('2026-09-07');
  });

  it('carries no cursor, so applying a filter starts from the first page', () => {
    // `starting_after` names a row in the PREVIOUS result set; carrying it
    // across a filter change asks for "the page after a payment that is no
    // longer in this list", and the answer looks like a working page.
    const data = new FormData(form(render(<PaymentsFilters values={EMPTY} />).container));
    expect(data.has('after')).toBe(false);
    expect(data.has('before')).toBe(false);
  });

  it('offers every status the badge can render, and no others', () => {
    // One vocabulary: a status cannot be filterable but unrenderable, or the
    // other way round. vpay answers 400 naming `status` for a value outside
    // its own vocabulary, which is right of vpay and a refusal this form
    // cannot provoke.
    render(<PaymentsFilters values={EMPTY} />);
    expect(screen.getByLabelText('Status')).toBeInTheDocument();
  });
});
