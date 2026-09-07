import { describe, expect, it } from 'vitest';

import { PAGE_SIZE, apiQueryString, NO_QUERY, pagerHrefs, queryFrom } from './payments-query';

const ROWS = [{ id: 'pi_example_1' }, { id: 'pi_example_2' }];

describe('the API query', () => {
  it('always asks for a bounded page', () => {
    expect(new URLSearchParams(apiQueryString(NO_QUERY)).get('limit')).toBe(String(PAGE_SIZE));
  });

  it('translates the URL date vocabulary into the API instant one', () => {
    const search = new URLSearchParams(
      apiQueryString({ ...NO_QUERY, createdFrom: '2026-09-01', createdTo: '2026-09-07' }),
    );
    expect(search.get('created_gte')).toBe('2026-09-01T00:00:00Z');
    expect(search.get('created_lte')).toBe('2026-09-07T23:59:59Z');
  });

  it('drops a date it cannot parse rather than forwarding it', () => {
    // vpay would answer 400 naming created_gte, which is correct of vpay and
    // still a worse screen than a filter that did not apply.
    const search = new URLSearchParams(apiQueryString({ ...NO_QUERY, createdFrom: 'yesterday' }));
    expect(search.has('created_gte')).toBe(false);
  });

  it('sends at most one cursor — forward wins over a hand-edited pair', () => {
    // vpay refuses starting_after and ending_before together.
    const search = new URLSearchParams(
      apiQueryString({ ...NO_QUERY, after: 'pi_a', before: 'pi_b' }),
    );
    expect(search.get('starting_after')).toBe('pi_a');
    expect(search.has('ending_before')).toBe(false);
  });
});

describe('reading the page query string', () => {
  it('takes the first value of a repeated parameter, never a join', () => {
    // `status=succeeded,canceled` is a value vpay answers 400 for, naming a
    // parameter the operator never typed.
    expect(queryFrom({ status: ['succeeded', 'canceled'] }).status).toBe('succeeded');
  });

  it('is empty for anything absent', () => {
    expect(queryFrom({})).toEqual(NO_QUERY);
  });
});

describe('the paging links', () => {
  it('offers Next only when has_more said so — never because the page was full', () => {
    // A full page is not evidence of another one; a Next link onto an empty
    // list reads as data having been lost.
    expect(pagerHrefs(NO_QUERY, ROWS, false).nextHref).toBeNull();
    expect(pagerHrefs(NO_QUERY, ROWS, true).nextHref).toBe('/payments?after=pi_example_2');
  });

  it('offers no Previous on the first page', () => {
    expect(pagerHrefs(NO_QUERY, ROWS, true).previousHref).toBeNull();
  });

  it('offers Previous from the first row once a forward cursor is set', () => {
    // We came from there, so the page above this one exists by construction —
    // it is not derived from `has_more`, which on a forward page is about
    // OLDER rows.
    const hrefs = pagerHrefs({ ...NO_QUERY, after: 'pi_zero' }, ROWS, false);
    expect(hrefs.previousHref).toBe('/payments?before=pi_example_1');
  });

  describe('paging backwards, where has_more means the opposite thing', () => {
    // `vpay_db` walks `seq ASC` from `ending_before` and reverses, so
    // `has_more` on a backward page is "more NEWER rows", not "more older
    // ones". Reading it the same way in both directions was wrong twice over,
    // and each of these two cases is one of the halves.
    const BACK = { ...NO_QUERY, before: 'pi_zero' };

    it('offers Next unconditionally — we came from the page below', () => {
      // has_more false here means "no more newer rows", which says nothing at
      // all about the older ones. Gating Next on it made the link vanish
      // although the page it came from certainly still existed.
      expect(pagerHrefs(BACK, ROWS, false).nextHref).toBe('/payments?after=pi_example_2');
      expect(pagerHrefs(BACK, ROWS, true).nextHref).toBe('/payments?after=pi_example_2');
    });

    it('offers Previous only while has_more says newer rows remain', () => {
      // At the newest end this used to render a Previous link pointing at an
      // empty page, which reads as data having been lost.
      expect(pagerHrefs(BACK, ROWS, false).previousHref).toBeNull();
      expect(pagerHrefs(BACK, ROWS, true).previousHref).toBe('/payments?before=pi_example_1');
    });
  });

  it('keeps the filters across a page turn', () => {
    // Otherwise page two of a filtered list is page two of every payment.
    const hrefs = pagerHrefs(
      { ...NO_QUERY, status: 'succeeded', createdFrom: '2026-09-01' },
      ROWS,
      true,
    );
    const next = new URLSearchParams((hrefs.nextHref ?? '').split('?')[1]);
    expect(next.get('status')).toBe('succeeded');
    expect(next.get('created_from')).toBe('2026-09-01');
    expect(next.get('after')).toBe('pi_example_2');
  });

  it('renders no pager at all for a single unpaged page', () => {
    expect(pagerHrefs(NO_QUERY, ROWS, false)).toEqual({ previousHref: null, nextHref: null });
  });

  it('offers nothing for an empty result set', () => {
    expect(pagerHrefs(NO_QUERY, [], false)).toEqual({ previousHref: null, nextHref: null });
  });
});
