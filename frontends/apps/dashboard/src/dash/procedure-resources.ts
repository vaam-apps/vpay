/**
 * `@cratestack/refine`'s resource manifest, bound to vpay's **procedures**
 * rather than to generated model routes.
 *
 * # Why not the generated manifest
 *
 * `cratestack generate-typescript --refine` emits a manifest that maps every
 * `.cstack` **model** to its generated CRUD class. vpay mounts none of those
 * routes — only `procedure_router` (`backends/crates/vpay-db/src/schema.rs`)
 * — and the five money models cannot be served by one while
 * `cratestack-core`'s `Value::from_plain_json` demotes JSON integers to
 * `f64` (still true at 0.16.0). A manifest pointing at routes nobody serves
 * would make every resource look wired and answer `404`. So `just
 * gen-dash-client` does not pass `--refine`, and this file is the manifest.
 *
 * # Why a procedure fits
 *
 * `@cratestack/refine` duck-types a resource's `api`: it needs a
 * `list({ query })` answering `T[]` or CrateStack's `Page<T>`. Every
 * `search*` procedure answers exactly `Page<T>` — `items`, `totalCount`,
 * `pageInfo` — because it is CrateStack's own type on both ends. The adapter
 * below is the whole of the difference: refine's `limit`/`offset` become the
 * procedure's `page`, and refine's filter keys become its named `filter`.
 *
 * # What it refuses, loudly
 *
 * - **A filter key the procedure does not declare.** `@cratestack/refine`
 *   compiles refine's filters to `field` / `field__op` keys; anything not in
 *   {@link FILTER_FIELDS} throws a `400` naming the key. That includes the
 *   `id__in` its `getMany` sends. Dropping an unknown filter would render an
 *   unfiltered page as if it were filtered.
 * - **A sort.** No procedure takes one; each orders newest first.
 * - **`get`, `update`, `delete`.** There is no detail procedure for these
 *   four, and `/dash/v1` is read-only (ADR-0008). `create` is absent, which
 *   is how `@cratestack/refine` already spells "no create route".
 *
 * # Isomorphic on purpose
 *
 * Nothing here holds a credential or reaches the network. The `list` it is
 * given decides that, and the one that signs its requests with a paired key is
 * VP-6b's to write (ADR-0036).
 */
import type {
  CratestackModelApi,
  CratestackFetchQuery,
  CratestackPage,
  ResourceMap,
} from "@cratestack/refine";
import type {
  CheckoutSessionListFilter,
  CustomerListFilter,
  RefundListFilter,
  WebhookDeliveryListFilter,
} from "@vpay/dash-client";

import type { ApiFailure } from "../server/api";
import {
  CHECKOUT_SESSIONS,
  CUSTOMERS,
  PROCEDURE_RESOURCES,
  REFUNDS,
  WEBHOOK_DELIVERIES,
  type ProcedureResource,
} from "./resource-name";

/** A procedure's `page` argument, as CrateStack's `PageInput` declares it. */
export interface ProcedurePageInput {
  readonly limit: number | null;
  readonly offset: number | null;
}

/**
 * Reads one page of one resource. Throws {@link ProcedureRefusal} on a
 * refusal — `@cratestack/refine` routes every throw through its
 * `toRefineError`, which keys on the `status` this carries.
 */
export type ProcedureLister = (
  resource: ProcedureResource,
  page: ProcedurePageInput,
  filter: Readonly<Record<string, string>>,
) => Promise<CratestackPage<unknown>>;

/**
 * A refusal, carrying this app's own {@link ApiFailure} so `ReadFailure` can
 * still render the request id after `@cratestack/refine` has annotated it.
 *
 * `status` is the field `@cratestack/refine`'s `isCratestackHttpError` reads,
 * so the refine error that reaches `authProvider.onError` has a real
 * `statusCode` — a `401` signs out, everything else, a `403` included, is an
 * outage (issue #88 item 2).
 */
export class ProcedureRefusal extends Error {
  readonly status: number;
  readonly failure: ApiFailure;

  constructor(failure: ApiFailure) {
    super(failure.message);
    this.name = "ProcedureRefusal";
    this.status = failure.status;
    this.failure = failure;
  }
}

function refusal(status: number, message: string): ProcedureRefusal {
  return new ProcedureRefusal({ status, message, requestId: null });
}

/**
 * `@cratestack/refine`'s filter key → the procedure's filter field.
 *
 * An `eq` filter on `status` arrives as `status`; a `gte` on `created` as
 * `created__gte`. Each map is typed against the **generated** filter type, so
 * a field removed from `schemas/vpay.cstack` is a compile error here after
 * `just gen-dash-client`, not a `400` in production.
 */
type FilterMap<F> = Readonly<Record<string, keyof F & string>>;

const CREATED = {
  created__gte: "created_gte",
  created__lte: "created_lte",
} as const;

export const FILTER_FIELDS: {
  readonly [REFUNDS]: FilterMap<RefundListFilter>;
  readonly [WEBHOOK_DELIVERIES]: FilterMap<WebhookDeliveryListFilter>;
  readonly [CUSTOMERS]: FilterMap<CustomerListFilter>;
  readonly [CHECKOUT_SESSIONS]: FilterMap<CheckoutSessionListFilter>;
} = {
  [REFUNDS]: { status: "status", ...CREATED },
  [WEBHOOK_DELIVERIES]: {
    state: "state",
    event_type: "event_type",
    ...CREATED,
  },
  [CUSTOMERS]: { email: "email", phone: "phone", ...CREATED },
  [CHECKOUT_SESSIONS]: {
    status: "status",
    payment_status: "payment_status",
    ...CREATED,
  },
};

/** Which filter keys a transport can carry, per resource. */
export type FilterAllowlist = Readonly<
  Record<ProcedureResource, Readonly<Record<string, string>>>
>;

/** Refine's filters, as the procedure's named filter — or a `400`. */
function filterFor(
  resource: ProcedureResource,
  query: CratestackFetchQuery | undefined,
  allowed: FilterAllowlist,
): Record<string, string> {
  if (query?.sort !== undefined && query.sort.length > 0) {
    throw refusal(
      400,
      `${resource} cannot be sorted: the procedure orders newest first.`,
    );
  }
  const filter: Record<string, string> = {};
  for (const [key, value] of Object.entries(query?.filters ?? {})) {
    const field = allowed[resource][key];
    if (field === undefined) {
      throw refusal(400, `${resource} has no filter "${key}".`);
    }
    filter[field] = value;
  }
  return filter;
}

function procedureApi(
  resource: ProcedureResource,
  list: ProcedureLister,
  allowed: FilterAllowlist,
): CratestackModelApi {
  return {
    async list(options) {
      const query = options?.query;
      return list(
        resource,
        { limit: query?.limit ?? null, offset: query?.offset ?? null },
        filterFor(resource, query, allowed),
      );
    },
    get() {
      return Promise.reject(
        refusal(404, `${resource} has no detail read on /dash/v1.`),
      );
    },
    update() {
      return Promise.reject(refusal(403, "/dash/v1 is read-only."));
    },
    delete() {
      return Promise.reject(refusal(403, "/dash/v1 is read-only."));
    },
  };
}

/**
 * The manifest for every procedure-backed resource.
 *
 * `paged: true` because every procedure answers `Page<T>` with a
 * `totalCount` computed in the same snapshot as its rows. **One gap is
 * `@cratestack/refine`'s, not vpay's:** past the end of the set a procedure
 * answers `totalCount: null` ("unknown") rather than a guess, and the
 * provider reports `items.length` — `0` — for it. The screens therefore read
 * the count from the page itself (`procedure-list.ts`), never from refine's
 * `total`.
 *
 * @param list the transport
 * @param allowed which filter keys that transport can carry
 */
export function procedureResources(
  list: ProcedureLister,
  allowed: FilterAllowlist = FILTER_FIELDS,
): ResourceMap {
  return Object.fromEntries(
    PROCEDURE_RESOURCES.map((resource) => [
      resource,
      {
        api: procedureApi(resource, list, allowed),
        primaryKey: "id",
        paged: true,
      },
    ]),
  );
}
