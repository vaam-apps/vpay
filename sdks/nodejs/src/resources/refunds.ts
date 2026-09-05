import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import { assertIntegerAmount } from "../validate.js";
import type { CreateRefundParams, Refund, RequestOptions } from "../types.js";

/** `/v1/refunds` — docs/flows/merchant-auth.md, "Resources". */
export class RefundsResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  async create(
    params: CreateRefundParams,
    options?: RequestOptions,
  ): Promise<Refund> {
    if (params.amount !== undefined) {
      assertIntegerAmount(params.amount);
    }
    const body: Record<string, FormValue> = {
      payment_intent: params.payment_intent,
    };
    if (params.amount !== undefined) {
      body["amount"] = params.amount;
    }
    if (params.reason !== undefined) {
      body["reason"] = params.reason;
    }
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    return this.#http.request<Refund>("POST", "/refunds", body, options);
  }

  /**
   * `GET /v1/refunds/{id}`.
   *
   * A refund is asynchronous on this rail — `RefundStatus` has a
   * non-terminal `"pending"` — and webhook delivery is at-least-once and
   * unordered, so this is the authoritative read a reconciliation job falls
   * back to when a `charge.refund.updated` was missed or arrived out of
   * order.
   */
  async retrieve(id: string): Promise<Refund> {
    return this.#http.request<Refund>(
      "GET",
      `/refunds/${encodeURIComponent(id)}`,
    );
  }
}
