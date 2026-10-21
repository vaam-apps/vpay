import {
  CHECKOUT_STATUS,
  DELIVERY_STATE,
  PAYMENT_STATUS,
  REFUND_STATUS,
  type PaymentStatus,
} from "@vpay/tokens";

import type { ApiFailure, ApiResult } from "../server/api";
import { callDashProcedure, type ProcedurePage } from "../server/dash-read";
import type { StaffContext } from "../server/session";
import {
  CHECKOUT_SESSIONS,
  CUSTOMERS,
  REFUNDS,
  WEBHOOK_DELIVERIES,
} from "./resource-name";
import { readProcedurePage } from "./procedure-list";

/** One row of `searchPaymentIntents`, exactly the fields this screen reads. */
export interface PaymentSummaryRow {
  readonly id: string;
  readonly amount: number;
  readonly currency_code: string;
  readonly status: string;
  readonly created_at: string;
}

/** A count, or the refusal that stopped it being read. Never a guess. */
export type Count = ApiResult<number>;

/** Everything the Overview shows, each part answered or refused on its own. */
export interface Overview {
  /** When these reads were made — what the screen says the counts are "as of". */
  readonly readAt: string;
  /** Every payment, and the newest {@link LATEST} of them. */
  readonly payments: ApiResult<{
    readonly total: number;
    readonly latest: readonly PaymentSummaryRow[];
  }>;
  readonly byStatus: Readonly<Record<PaymentStatus, Count>>;
  readonly failedRefunds: Count;
  readonly pendingRefunds: Count;
  readonly failedDeliveries: Count;
  readonly exhaustedDeliveries: Count;
  readonly openCheckouts: Count;
  readonly customers: Count;
}

/** How many payments the Overview lists under "Latest payments". */
export const LATEST = 6;

/**
 * Read the Overview: twelve procedure calls, in parallel, on the server.
 *
 * # Every number is `totalCount`, which vpay computes exactly
 *
 * The four search procedures answer `Page<T>`, whose `totalCount` is
 * `count(*) OVER ()` over the filtered statement — exact for that statement
 * (`search_payment_intents.rs`). So each count here is one call with
 * `limit: 1` and the procedure's own filter, and the Overview adds nothing,
 * divides nothing and estimates nothing: a number on the screen is one vpay
 * answered. That is also why the counts are not summed into a total — the
 * "All payments" figure is its own unfiltered call, not the sum of the five
 * statuses, which could disagree with it under concurrent writes and would
 * then be a number vpay never said.
 *
 * `totalCount` is `Option<i64>` upstream and, at offset 0, is present
 * whenever the statement matched a row and `0` when it matched none
 * (`search_payment_intents.rs`). A `null` here is therefore read as `0`
 * only at offset 0, which is the only offset this module asks for.
 *
 * # A refused read stays refused, per number
 *
 * One status the procedure refuses does not blank the screen: each count is
 * its own `ApiResult`, and the screen draws a refused one as a dash with the
 * refusal beside it. Rendering a refusal as `0` is the failure `CLAUDE.md`
 * names first.
 *
 * # Why in parallel is safe for the token
 *
 * `requireStaff` re-mints the access token at 80 % of its TTL before this
 * runs, so these calls start on a fresh token. If one still met a `401`, it
 * re-mints once and retries (`callDashProcedure`), and each re-mint writes
 * the same session row; none of them is ever answered with another's token.
 */
export async function readOverview(staff: StaffContext): Promise<Overview> {
  const readAt = new Date().toISOString();
  const searchPayments = (filter: Record<string, string>, limit: number) =>
    callDashProcedure<ProcedurePage<PaymentSummaryRow>>(
      staff.config,
      "searchPaymentIntents",
      { page: { limit, offset: 0 }, filter },
      staff.accessToken,
      staff.sessionToken,
    );
  const count = async (
    read: Promise<ApiResult<ProcedurePage<unknown>>>,
  ): Promise<Count> => {
    const result = await read;
    return result.ok
      ? { ok: true, value: result.value.totalCount ?? 0 }
      : { ok: false, failure: result.failure };
  };
  const procedureCount = (resource: string, filter: Record<string, string>) =>
    count(readProcedurePage<unknown>(staff, resource, 0, filter, 1));

  const [all, statusCounts, rest] = await Promise.all([
    searchPayments({}, LATEST),
    Promise.all(
      PAYMENT_STATUS.map((status) => count(searchPayments({ status }, 1))),
    ),
    Promise.all([
      procedureCount(REFUNDS, { status: refundStatus("failed") }),
      procedureCount(REFUNDS, { status: refundStatus("pending") }),
      procedureCount(WEBHOOK_DELIVERIES, { state: deliveryState("failed") }),
      procedureCount(WEBHOOK_DELIVERIES, { state: deliveryState("exhausted") }),
      procedureCount(CHECKOUT_SESSIONS, { status: checkoutStatus("open") }),
      procedureCount(CUSTOMERS, {}),
    ]),
  ]);

  const byStatus = Object.fromEntries(
    PAYMENT_STATUS.map((status, index) => [status, statusCounts[index]]),
  ) as Record<PaymentStatus, Count>;

  return {
    readAt,
    payments: all.ok
      ? {
          ok: true,
          value: {
            total: all.value.totalCount ?? 0,
            latest: all.value.items,
          },
        }
      : { ok: false, failure: all.failure },
    byStatus,
    failedRefunds: rest[0],
    pendingRefunds: rest[1],
    failedDeliveries: rest[2],
    exhaustedDeliveries: rest[3],
    openCheckouts: rest[4],
    customers: rest[5],
  };
}

/** The first refusal among the Overview's reads, to show once with its id. */
export function firstFailure(overview: Overview): ApiFailure | null {
  const results: readonly ApiResult<unknown>[] = [
    overview.payments,
    ...Object.values(overview.byStatus),
    overview.failedRefunds,
    overview.pendingRefunds,
    overview.failedDeliveries,
    overview.exhaustedDeliveries,
    overview.openCheckouts,
    overview.customers,
  ];
  for (const result of results) {
    if (!result.ok) {
      return result.failure;
    }
  }
  return null;
}

// The vocabularies are the database's own CHECK constraints (`@vpay/tokens`);
// these narrow a literal to one, so a renamed state is a compile error here
// rather than a filter vpay refuses at runtime.
function refundStatus(value: (typeof REFUND_STATUS)[number]): string {
  return value;
}
function deliveryState(value: (typeof DELIVERY_STATE)[number]): string {
  return value;
}
function checkoutStatus(value: (typeof CHECKOUT_STATUS)[number]): string {
  return value;
}
