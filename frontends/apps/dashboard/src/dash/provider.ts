/**
 * The one place this app reads `/dash/v1` from, shaped like a data provider.
 *
 * # What this is, and what it deliberately is not
 *
 * Two pages fetch payments today, each by assembling a path, calling
 * `readDash`, and unpacking the envelope itself. That is three decisions —
 * which path, which query parameters, what a page of results means — written
 * twice and reachable only through a rendered Server Component. This module
 * is those decisions extracted into two functions with a name the eventual
 * framework already uses (`getList` / `getOne`), so that wiring a framework
 * to them later is a change of *caller* rather than a rewrite of the read
 * path.
 *
 * **There is no Refine dependency here and there is not meant to be one.**
 * The package is not installed, nothing imports it, and this module's shapes
 * are this app's own: `ApiResult` for the outcome, `PaymentsQuery` for the
 * filters, `PageCursors` for the paging. A Refine `DataProvider` — whose
 * `getList` answers `{ data, total, cursor }` and whose `create`/`update`/
 * `deleteOne` must throw against a surface that refuses every non-`GET` — is
 * a thin adapter *over* this, and it is a later lane's. Declaring `total: 0`
 * here, in a module no framework reads, would be a field that exists to
 * satisfy a contract nothing in this repository has signed.
 *
 * # Still server-side, and that is not a temporary state
 *
 * Every function here takes the `/dash/v1` bearer token as an argument and
 * calls `readDash`, which means it runs where the token is: in this process,
 * on this request (`ADR-0008`, `ADR-0017` decision 4). What keeps it there is
 * what keeps `server/api.ts` there — every importer is a Server Component, a
 * Server Action or a Route Handler, and it is exported to none that is not —
 * rather than a `server-only` import, which throws under anything but
 * `react-server` and would make this module's own vitest suite unrunnable.
 * Nothing it returns carries a credential, and the re-mint `readDash`
 * performs is untouched.
 *
 * # One resource, by type
 *
 * `/dash/v1` serves `payment_intents` and nothing else — slices 2–6 have no
 * routes, no handlers and no stubs (`docs/flows/dashboard.md`). So the
 * resource is a one-member union rather than a `string`, and asking this
 * module for `webhooks` is a compile error rather than a `404` at runtime or,
 * worse, an empty list.
 */
import {
  apiQueryString,
  pageCursors,
  type PageCursors,
  type PaymentsQuery,
} from "../payments-query";
import type { DashboardConfig } from "../config/settings";
import type {
  ApiFailure,
  ApiResult,
  PaymentDetail,
  PaymentIntentList,
  PaymentIntentObject,
} from "../server/api";
import { readDash } from "../server/dash-read";
import { type DashResource } from "./resource-name";

/**
 * The only resource `/dash/v1` serves.
 *
 * A `const` as well as a type, so that a caller naming the resource and this
 * module's own path building are one string rather than two that can drift.
 */
export { PAYMENT_INTENTS } from "./resource-name";

/** Every resource this app may ask for — there is exactly one. */
export type { DashResource } from "./resource-name";

/**
 * What a read needs beyond its own arguments.
 *
 * Structurally a subset of `server/session.ts`'s `StaffContext`, so a page
 * that has been through `requireStaff` passes `gate.staff` straight in. It is
 * spelled out here rather than imported as that type because importing it
 * would pull `next/headers` and `next/navigation` into every consumer of this
 * module, including its own test.
 */
export interface DashSession {
  /** The settings this container booted with. */
  readonly config: DashboardConfig;
  /** The `/dash/v1` bearer token, read back from the `staff_sessions` row. */
  readonly accessToken: string;
  /** The staff session token, which `readDash` mints a replacement with. */
  readonly sessionToken: string;
}

/** What {@link getList} was asked for. */
export interface ListParams {
  readonly resource: DashResource;
  /** The filters and cursor, as this app's own URL spells them. */
  readonly query: PaymentsQuery;
}

/** What {@link getOne} was asked for. */
export interface OneParams {
  readonly resource: DashResource;
  /** The `pi_…` id, unencoded. */
  readonly id: string;
}

/**
 * One page of a list, and the two ends it can be walked from.
 *
 * There is no `total`, because `/dash/v1` returns none: `ListObject` is
 * `{ object, data, has_more, url }` and nothing else. A count invented here —
 * `data.length`, or a `0` standing in for "unknown" — would be a number a
 * pager could divide by, and the control it drew would be decorative.
 */
export interface ListPage<T> {
  /** The rows, newest first, exactly as vpay ordered them. */
  readonly data: readonly T[];
  /**
   * vpay's own flag, verbatim and un-reinterpreted.
   *
   * It means "a row exists past the limit **in the direction just walked**",
   * which is a different sentence on a backward page than on a forward one —
   * see {@link ListPage.cursor}, which is where that is resolved.
   */
  readonly hasMore: boolean;
  /** Which way this page can be paged from, `has_more`'s inversion applied. */
  readonly cursor: PageCursors;
}

/**
 * The reads this app makes, named the way a data provider names them.
 *
 * An interface rather than two loose functions so that the eventual Refine
 * adapter has one thing to be written against, and so that a test can supply
 * a different implementation without stubbing a module.
 */
export interface DashProvider {
  /** One page of `resource`, filtered and paged by `params.query`. */
  getList(
    params: ListParams,
  ): Promise<ApiResult<ListPage<PaymentIntentObject>>>;
  /** One record of `resource`, by id. */
  getOne(params: OneParams): Promise<ApiResult<PaymentDetail>>;
}

/**
 * The provider bound to one staff member's request.
 *
 * Bound per request rather than per process, deliberately: the bearer it
 * closes over was read out of the `staff_sessions` row on *this* render, and
 * a provider that outlived the request would be a token cached across
 * requests — which is the one thing that would stop sign-out being a
 * revocation (`app/payments/page.tsx`).
 *
 * @param session the config and the two tokens `requireStaff` produced
 */
export function dashProvider(session: DashSession): DashProvider {
  return {
    async getList({ resource, query }) {
      const result = await readDash<PaymentIntentList>(
        session.config,
        `/dash/v1/${resource}?${apiQueryString(query)}`,
        session.accessToken,
        session.sessionToken,
      );
      if (!result.ok) {
        return result;
      }
      if (!Array.isArray(result.value?.data)) {
        return { ok: false, failure: notTheDocument("a list") };
      }
      // Read once and passed to both, so the flag this module answers with
      // and the flag it derives the cursors from cannot be two readings of
      // one field. `=== true` because a `200` that carried no `has_more` at
      // all reached `pageCursors` as `undefined` before, where it is falsy
      // and therefore silently "no more rows" — the same answer an empty
      // page gives, which is the reading `payments-query.ts` exists to stop.
      const hasMore = result.value.has_more === true;
      return {
        ok: true,
        value: {
          data: result.value.data,
          hasMore,
          cursor: pageCursors(query, result.value.data, hasMore),
        },
      };
    },

    async getOne({ resource, id }) {
      // `encodeURIComponent` and not a template alone: an id is a path
      // segment here, and a caller-supplied `../staff/session` would
      // otherwise be a different upstream route entirely.
      const result = await readDash<PaymentDetail>(
        session.config,
        `/dash/v1/${resource}/${encodeURIComponent(id)}`,
        session.accessToken,
        session.sessionToken,
      );
      if (!result.ok) {
        return result;
      }
      if (!isObject(result.value) || !isObject(result.value.payment_intent)) {
        return { ok: false, failure: notTheDocument("a payment detail") };
      }
      return result;
    },
  };
}

/** A non-null, non-array object — the only shape either read may be handed. */
function isObject(value: unknown): boolean {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * A `200` whose body is not the document this module asked for, as a refusal.
 *
 * `api.ts`'s `getJson` answers `(await response.json()) as T` — an unchecked
 * cast, which is the right trade for a typed client of a service this
 * repository also owns, and is not a claim that the body **is** a `T`. Until
 * this guard existed the claim was load-bearing anyway: a `200` carrying
 * `{}`, `null`, a bare string, or a `data` that is not an array reached
 * `pageCursors`, which indexes `rows[0]`, and the `TypeError` propagated out
 * of a Server Component render and out of `app/api/dash/**`'s route handlers
 * — where Next answers it as a `500` rather than as anything a caller can
 * read. Measured on this branch: five of six malformed `200`s threw, and the
 * sixth (`data: {}`) answered `200` with a page whose rows were an object.
 *
 * A refusal and **not** an empty list: an empty list is a claim about the
 * merchant's payments, and "vpay answered something this app does not
 * recognise" is a claim about vpay. AGENTS.md rule 2 is the difference.
 *
 * `502` and not `0`: there *was* a response, so this is a bad gateway rather
 * than `api.ts`'s "no response at all", and `bff.ts`'s `upstreamRefusal`
 * carries the status through untouched.
 */
function notTheDocument(expected: string): ApiFailure {
  return {
    status: 502,
    message: `vpay answered something other than ${expected}.`,
    requestId: null,
  };
}
