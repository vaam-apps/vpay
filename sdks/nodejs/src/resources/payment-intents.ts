import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import { assertIntegerAmount } from "../validate.js";
import type {
  ConfirmPaymentIntentParams,
  CreatePaymentIntentParams,
  List,
  ListParams,
  PaymentIntent,
  RequestOptions,
} from "../types.js";

/** `/v1/payment_intents` — docs/flows/merchant-auth.md, "Resources". */
export class PaymentIntentsResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  async create(
    params: CreatePaymentIntentParams,
    options?: RequestOptions,
  ): Promise<PaymentIntent> {
    assertIntegerAmount(params.amount);
    const body: Record<string, FormValue> = {
      amount: params.amount,
      currency: params.currency.toLowerCase(),
      payment_method_types: params.payment_method_types,
    };
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    if (params.description !== undefined) {
      body["description"] = params.description;
    }
    return this.#http.request<PaymentIntent>(
      "POST",
      "/payment_intents",
      body,
      options,
    );
  }

  async retrieve(id: string): Promise<PaymentIntent> {
    return this.#http.request<PaymentIntent>(
      "GET",
      `/payment_intents/${encodeURIComponent(id)}`,
    );
  }

  async confirm(
    id: string,
    params: ConfirmPaymentIntentParams,
    options?: RequestOptions,
  ): Promise<PaymentIntent> {
    const body: Record<string, FormValue> = {
      payment_method_data: params.payment_method_data,
    };
    if ("return_url" in params) {
      body["return_url"] = params.return_url;
    }
    return this.#http.request<PaymentIntent>(
      "POST",
      `/payment_intents/${encodeURIComponent(id)}/confirm`,
      body,
      options,
    );
  }

  async cancel(id: string, options?: RequestOptions): Promise<PaymentIntent> {
    return this.#http.request<PaymentIntent>(
      "POST",
      `/payment_intents/${encodeURIComponent(id)}/cancel`,
      {},
      options,
    );
  }

  async list(params?: ListParams): Promise<List<PaymentIntent>> {
    return this.#http.request<List<PaymentIntent>>(
      "GET",
      "/payment_intents",
      params,
    );
  }
}
