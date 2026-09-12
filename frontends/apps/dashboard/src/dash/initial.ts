/**
 * The read the **server** already made, handed to the Refine hook that would
 * otherwise make it again from the browser.
 *
 * # Why this module exists, stated as the defect it repairs
 *
 * Lane 3 moved `/payments` and `/payments/{id}` onto `useList` / `useOne`,
 * which fetch from `/api/dash` **in the browser, after hydration**. Two
 * things follow, and both were measured in `dashboard.cy.ts` rather than
 * reasoned about:
 *
 * 1. **The first paint carries no data.** At the `load` event the detail
 *    page is a skeleton, so anything reading the document at that moment —
 *    a person on a slow link, a crawler, or
 *    `renders the masked payer as a dash`, which reads `body` once and does
 *    not retry — sees an empty screen. That leg walked every row in the
 *    list, found `[data-testid="detail-payer"]` on none of them, and
 *    reported "no payment in this merchant's list has a charge" about a
 *    merchant whose list holds four.
 * 2. **The page issues a `/api/dash` request of its own**, where until Lane 3
 *    "no page and no component" called that surface (`server/bff.ts` says so
 *    in as many words). `dashboard.cy.ts`'s BFF block aliases that URL
 *    *before* it visits the page, so `cy.wait` consumed the page's own
 *    cookie-bearing request instead of the deliberate cookie-less one the
 *    leg had just made — the leg failed between `cy.clearCookie` and the
 *    `cy.setCookie` that restores it, and the six legs after it ran in a
 *    signed-out browser.
 *
 * So the reads go back where they were: in the Server Component, on the
 * request, with the token that never leaves this process. Refine keeps the
 * data layer — the hooks, the provider, the cache, the paging — and is
 * handed the answer rather than sent to fetch one. This is the SSR shape
 * §2.6 of `docs/plans/exp55-refine-seam-bff-notes/refine-plan.md` names as
 * the alternative to a browser-side first read.
 *
 * # A refusal is *not* retried from the browser, and that is the point
 *
 * {@link initialQueryOptions} answers `enabled: false` when the server's read
 * was refused, and the screen renders that refusal directly. Retrying it
 * through `/api/dash` would be the same refusal a round trip later — except
 * for a `401`, which reaches `authProvider.onError`, whose answer to a `401`
 * is `{ logout: true }` and therefore the real `signOut()` Server Action.
 * `requireStaff` deliberately **keeps** the cookie for a read it could not
 * complete (issue #88 item 2); a browser-side retry of that same read would
 * end the session the server had just decided to keep.
 */
import type { GetListResponse, GetOneResponse } from "@refinedev/core";

import type { PageCursors } from "../payments-query";
import type {
  ApiFailure,
  PaymentDetail,
  PaymentIntentObject,
} from "../server/api";

/**
 * Which way a page can be walked, in Refine's vocabulary rather than vpay's.
 *
 * `next` is *older* rows and `prev` is *newer* ones — `/dash/v1` orders
 * newest first — and that inversion is written **once**, in
 * {@link refineCursor}, because the data provider and the server-rendered
 * first page both have to say it and two transcriptions of it could disagree
 * about which end of the list a link walks to.
 */
export interface RefineCursor {
  /** The cursor for the page after this one: older rows. */
  readonly next: string | null;
  /** The cursor for the page before this one: newer rows. */
  readonly prev: string | null;
}

/** `PageCursors` as Refine names the same two ends. */
export function refineCursor(cursors: PageCursors): RefineCursor {
  return { next: cursors.older, prev: cursors.newer };
}

/**
 * Exactly what `dashDataProvider.getList` resolves to.
 *
 * **Refine's own `GetListResponse`** rather than a shape of this app's that
 * happens to look like it: this value is written straight into the hook's
 * cache as `initialData`, so anything it and the provider's answer disagreed
 * about would be a first render that contradicted every render after it.
 *
 * `total` is `0` for the reason that provider states: `/dash/v1` reports no
 * count, and `0` is Refine's documented "unknown". A number derived from
 * `data.length` here would be one a pager could divide by.
 */
export type InitialListPage = GetListResponse<PaymentIntentObject> & {
  readonly cursor: RefineCursor;
};

/** Exactly what `dashDataProvider.getOne` resolves to. */
export type InitialOneRecord = GetOneResponse<PaymentDetail>;

/**
 * A read the server made on this request: the document, or the refusal.
 *
 * `ApiFailure` and not an `Error`, because the failure crosses the
 * server/client boundary as a plain object and `ReadFailure` renders a status
 * and a request id from exactly this shape.
 */
export type Initial<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly failure: ApiFailure };

/** The payments list, as the server read it. */
export type InitialList = Initial<InitialListPage>;

/** One payment, as the server read it. */
export type InitialOne = Initial<InitialOneRecord>;

/** The refusal the server met, or `null` — including when it was given none. */
export function initialFailure<T>(
  initial: Initial<T> | undefined,
): ApiFailure | null {
  return initial !== undefined && !initial.ok ? initial.failure : null;
}

/**
 * The three `queryOptions` a server-rendered read needs.
 *
 * - `initialData` — the document, so the very first render (which happens on
 *   the server) already has the rows.
 * - `refetchOnMount: false` — **the whole of the second defect above.**
 *   TanStack treats `initialData` as fetched-just-now but stale, and would
 *   re-read it on mount; that re-read is the browser request this app is not
 *   supposed to be making for data it was handed. A query with no data still
 *   fetches — this option only ever suppresses a *re*-fetch — so the unit
 *   suite's hookless mounts are untouched.
 * - `enabled` — `false` only when the server's read was refused. See this
 *   module's header for why a refusal must not be retried from a browser.
 *
 * With no `initial` at all (the component tests, which mount the screens
 * against a stub provider) every option is inert and the hook fetches exactly
 * as it did before.
 */
export function initialQueryOptions<T>(initial: Initial<T> | undefined): {
  readonly initialData?: T;
  readonly refetchOnMount: false;
  readonly enabled: boolean;
} {
  return {
    // Spread rather than `initialData: … : undefined`. Under
    // `exactOptionalPropertyTypes` an explicit `undefined` is not the same
    // as an absent key, and TanStack's `initialData` is one of the options
    // that means something by being absent.
    ...(initial !== undefined && initial.ok
      ? { initialData: initial.value }
      : {}),
    refetchOnMount: false,
    enabled: initialFailure(initial) === null,
  };
}
