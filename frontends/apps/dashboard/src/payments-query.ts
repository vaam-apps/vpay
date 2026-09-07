/**
 * Turning `/payments`' own query string into `/dash/v1/payment_intents`', and
 * back into the two paging links.
 *
 * Pure, and separate from the page, because paging is where a list view goes
 * quietly wrong: a cursor carried across a filter change asks for "the page
 * after a row that is no longer in this result set", and the answer looks
 * like a working page.
 *
 * # Two vocabularies, deliberately not one
 *
 * The **URL** this app serves uses `created_from` / `created_to` and plain
 * `YYYY-MM-DD` dates, because that is what an `<input type="date">` produces
 * and what an operator can read in an address bar. The **API** takes
 * `created_gte` / `created_lte` as RFC 3339 instants. Translating between
 * them is this file's whole job, and doing it here rather than in the page
 * means the boundary conversion is a unit test.
 */
import { endOfDayUtc, startOfDayUtc } from './format';

/** How many rows a page asks for. */
export const PAGE_SIZE = 25;

/** The filters and cursor, as this app's own URL spells them. */
export interface PaymentsQuery {
  /** One `IntentStatus` wire value, or `''`. */
  readonly status: string;
  /** `YYYY-MM-DD` or `''`. */
  readonly createdFrom: string;
  /** `YYYY-MM-DD` or `''`. */
  readonly createdTo: string;
  /** The id to page forward from, or `''`. */
  readonly after: string;
  /** The id to page back from, or `''`. */
  readonly before: string;
}

/** An empty query — every payment, newest first, first page. */
export const NO_QUERY: PaymentsQuery = {
  status: '',
  createdFrom: '',
  createdTo: '',
  after: '',
  before: '',
};

/**
 * One search parameter as a plain string.
 *
 * Next hands a repeated parameter as an array. The **first** value wins
 * rather than the last, and rather than joining them: `?status=succeeded&status=canceled`
 * is one of the two, and a joined `succeeded,canceled` would be a value vpay
 * answers `400` for, naming a parameter the operator did not type.
 */
export function one(value: string | string[] | undefined): string {
  if (Array.isArray(value)) {
    return typeof value[0] === 'string' ? value[0].trim() : '';
  }
  return typeof value === 'string' ? value.trim() : '';
}

/** The query this request is asking for. */
export function queryFrom(
  params: Readonly<Record<string, string | string[] | undefined>>,
): PaymentsQuery {
  return {
    status: one(params['status']),
    createdFrom: one(params['created_from']),
    createdTo: one(params['created_to']),
    after: one(params['after']),
    before: one(params['before']),
  };
}

/**
 * The `/dash/v1/payment_intents` query string.
 *
 * A filter this app cannot turn into an RFC 3339 instant — a hand-edited
 * `created_from=yesterday` — is **dropped** rather than forwarded. vpay would
 * answer `400` naming `created_gte`, which is correct of vpay and is still a
 * worse screen than a list that ignored an unparseable date; `format.ts`'s
 * `startOfDayUtc` is what decides, and it accepts only a real calendar date.
 *
 * `starting_after` and `ending_before` are mutually exclusive — vpay refuses
 * both together — so forward wins if a hand-edited URL carries both.
 */
export function apiQueryString(query: PaymentsQuery): string {
  const search = new URLSearchParams();
  search.set('limit', String(PAGE_SIZE));
  if (query.status.length > 0) {
    search.set('status', query.status);
  }
  const gte = query.createdFrom.length > 0 ? startOfDayUtc(query.createdFrom) : null;
  if (gte !== null) {
    search.set('created_gte', gte);
  }
  const lte = query.createdTo.length > 0 ? endOfDayUtc(query.createdTo) : null;
  if (lte !== null) {
    search.set('created_lte', lte);
  }
  if (query.after.length > 0) {
    search.set('starting_after', query.after);
  } else if (query.before.length > 0) {
    search.set('ending_before', query.before);
  }
  return search.toString();
}

/** The filters alone, for a link that changes only the cursor. */
function filterParams(query: PaymentsQuery): URLSearchParams {
  const search = new URLSearchParams();
  if (query.status.length > 0) {
    search.set('status', query.status);
  }
  if (query.createdFrom.length > 0) {
    search.set('created_from', query.createdFrom);
  }
  if (query.createdTo.length > 0) {
    search.set('created_to', query.createdTo);
  }
  return search;
}

/** Where the two paging links point, or `null` where there is nowhere to go. */
export interface PagerHrefs {
  readonly previousHref: string | null;
  readonly nextHref: string | null;
}

/**
 * The paging links for a page of `rows`.
 *
 * **`hasMore` is the only thing that decides whether there is a next page.**
 * A full page is not evidence of one: a result set of exactly `PAGE_SIZE`
 * rows would produce a "Next" link onto an empty list, which reads as data
 * having been lost.
 *
 * **"Previous" exists as soon as this is not the first page**, which is what
 * `after`/`before` being set means. It is not derived from `hasMore`, because
 * paging backwards is a different question from whether more rows lie ahead.
 */
export function pagerHrefs(
  query: PaymentsQuery,
  rows: readonly { readonly id: string }[],
  hasMore: boolean,
): PagerHrefs {
  const first = rows[0];
  const last = rows[rows.length - 1];
  const onFirstPage = query.after.length === 0 && query.before.length === 0;

  const next =
    hasMore && last !== undefined
      ? withCursor(query, 'after', last.id)
      : null;
  const previous =
    !onFirstPage && first !== undefined ? withCursor(query, 'before', first.id) : null;

  return { previousHref: previous, nextHref: next };
}

/** `/payments` with the filters kept and one cursor set. */
function withCursor(query: PaymentsQuery, key: 'after' | 'before', id: string): string {
  const search = filterParams(query);
  search.set(key, id);
  return `/payments?${search.toString()}`;
}
