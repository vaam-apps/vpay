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
}
