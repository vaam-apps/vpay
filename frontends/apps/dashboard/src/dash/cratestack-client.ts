/**
 * The generated CrateStack client, as this app is allowed to use it: its
 * **procedures**, server side, with the session's token.
 *
 * # Procedures only, and ESLint holds the line
 *
 * `@vpay/dash-client` is generated from `schemas/vpay.cstack` by `just
 * gen-dash-client` and carries a class for every model as well as the
 * procedures. **None of those model routes is mounted** — vpay serves
 * `procedure_router` and nothing else of the generated transport — and one of
 * them is worse than absent: `PaymentIntentApi.list()` would reach the
 * hand-written, cursor-paged `GET /dash/v1/payment_intents` and cast its
 * `ListObject` to an array. So this module is the only one that may import
 * the package at runtime (`no-restricted-imports` in `eslint.config.js`;
 * type imports are allowed anywhere), and it hands out `procedures` alone.
 *
 * # The token never leaves this process
 *
 * The bearer is read from the `staff_sessions` row by `requireStaff` or the
 * BFF and passed in here. Nothing in this module is imported by a client
 * component, and nothing it returns carries a token.
 *
 * # One `401` re-mints once
 *
 * {@link callProcedure} runs every call through `withOneRemint`, the same
 * eight lines `readDash` uses, so the generated path and the hand-written one
 * cannot disagree about when a session is over. A `403` is never retried.
 */
import {
  CratestackHttpError,
  VpayDashClientClient,
  type Page,
} from "@vpay/dash-client";

import type { DashboardConfig } from "../config/settings";
import {
  envelopeMessage,
  requestIdOf,
  unreachable,
  type ApiFailure,
  type ApiResult,
} from "../server/api";
import { withOneRemint } from "../server/dash-read";
import type { ProcedurePageInput } from "./procedure-resources";
import {
  CHECKOUT_SESSIONS,
  CUSTOMERS,
  REFUNDS,
  WEBHOOK_DELIVERIES,
  type ProcedureResource,
} from "./resource-name";

/** The generated client's procedures, and nothing else of it. */
export type DashProcedures = VpayDashClientClient["procedures"];

/** What a procedure call needs to know about the session it reads for. */
export interface ProcedureReader {
  readonly config: DashboardConfig;
  /** The `/dash/v1` bearer token, read back from the session row. */
  readonly accessToken: string;
  /** The staff session token, to mint a replacement with. */
  readonly sessionToken: string;
}

/**
 * The procedures of a client that presents `bearer`.
 *
 * `cache: "no-store"` and `redirect: "error"` are `getJson`'s, for its
 * reasons: an operator reads this to decide whether something is still
 * happening, and a redirect from this surface is a misconfiguration. The
 * global `fetch` is looked up per call rather than bound once, so the test
 * suites' network stubs see these calls exactly as they saw the hand-written
 * ones.
 */
export function dashProcedures(
  config: DashboardConfig,
  bearer: string,
): DashProcedures {
  return new VpayDashClientClient(config.apiBaseUrl, {
    basePath: "/dash/v1",
    headers: { authorization: `Bearer ${bearer}` },
    fetch: (input, init) =>
      fetch(input, { ...init, cache: "no-store", redirect: "error" }),
  }).procedures;
}

/**
 * A thrown generated-client error, as this app's {@link ApiFailure}.
 *
 * The runtime decodes an error body only when it is labelled JSON, so a body
 * that arrives as text is parsed here before `envelopeMessage` reads it —
 * and, as everywhere in this app, an unparseable one is never rendered raw.
 */
export function procedureFailure(error: unknown): ApiFailure {
  if (error instanceof CratestackHttpError) {
    let body = error.payload;
    if (typeof body === "string") {
      try {
        body = JSON.parse(body) as unknown;
      } catch {
        body = null;
      }
    }
    return {
      status: error.status,
      message: envelopeMessage(error.status, body),
      requestId: requestIdOf(error.response),
    };
  }
  return unreachable(error);
}

/**
 * Call one or more procedures as `reader`, answering an {@link ApiResult}
 * rather than throwing — every screen in this app branches on that shape.
 */
export async function callProcedure<T>(
  reader: ProcedureReader,
  call: (procedures: DashProcedures) => Promise<T>,
): Promise<ApiResult<T>> {
  return withOneRemint(
    reader.config,
    reader.accessToken,
    reader.sessionToken,
    async (bearer) => {
      try {
        return {
          ok: true,
          value: await call(dashProcedures(reader.config, bearer)),
        };
      } catch (error) {
        return { ok: false, failure: procedureFailure(error) };
      }
    },
  );
}

/**
 * Which procedure lists which resource — a **call**, not a name.
 *
 * The filter arrives already narrowed to the fields
 * `procedure-resources.ts`' `FILTER_FIELDS` maps onto each generated filter
 * type. Every generated filter field is an optional string, so a narrowed
 * record is assignable to each without a cast — and a non-string field added
 * to one would make this table stop compiling, which is the point.
 */
const LIST_OF: Readonly<
  Record<
    ProcedureResource,
    (
      procedures: DashProcedures,
      page: ProcedurePageInput,
      filter: Readonly<Record<string, string>>,
    ) => Promise<Page<unknown>>
  >
> = {
  [REFUNDS]: (p, page, filter) => p.searchRefunds({ page, filter }),
  [WEBHOOK_DELIVERIES]: (p, page, filter) =>
    p.searchWebhookDeliveries({ page, filter }),
  [CUSTOMERS]: (p, page, filter) => p.searchCustomers({ page, filter }),
  [CHECKOUT_SESSIONS]: (p, page, filter) =>
    p.searchCheckoutSessions({ page, filter }),
};

/** One page of a procedure-backed resource, read as `reader`. */
export function listProcedurePage(
  reader: ProcedureReader,
  resource: ProcedureResource,
  page: ProcedurePageInput,
  filter: Readonly<Record<string, string>>,
): Promise<ApiResult<Page<unknown>>> {
  return callProcedure(reader, (procedures) =>
    LIST_OF[resource](procedures, page, filter),
  );
}
