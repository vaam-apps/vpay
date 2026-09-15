/**
 * `/v1/refunds` — the five merchant operations on the Refund object
 * (RFC-0003 § 2).
 *
 * # No refund any of these creates has ever moved money
 *
 * Read this before any sentence elsewhere that says a refund "works". The
 * routes are real and so is every refusal they make, but nothing on the other
 * side of them pays anybody:
 *
 * - `orange_money`'s refund is a declared `NotImplemented` token — an Orange
 *   refund is an outbound transfer this repository has no specification for;
 * - `mtn_momo`'s is MTN's Disbursements `transfer`, WireMock-proven and
 *   rail-unproven: **no real MTN Disbursements credential exists in this
 *   project** — the only `disbursement_subscription_key` anywhere is the stub
 *   the e2e/demo stack points at a WireMock container — and the product has
 *   never been called;
 * - **nothing settles a `pending` refund.** There is no refund poll ladder,
 *   so a refund created here stays `"pending"` until an operator or the
 *   settlement path moves it.
 *
 * `docs/status.md` carries all three. A `200` from
 * {@link RefundsResource.create} means a refund row exists and its amount is
 * reserved against the intent. It does not mean the payee has been paid.
 *
 * # The state machine is the server's `WHERE` clause
 *
 * {@link RefundsResource.cancel} works while the refund is `"pending"` and
 * answers a `409` naming the current status otherwise. This SDK does not try
 * to predict which — reading the refund first to decide whether to call is a
 * race. Call, and read the refusal.
 */
import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import { assertIntegerAmount } from "../validate.js";
import type {
  CreateRefundParams,
  List,
  ListRefundsParams,
  Refund,
  RefundDestination,
  RequestOptions,
  UpdateRefundParams,
} from "../types.js";

/**
 * The rail-agnostic envelope, built around the rail-specific interior.
 *
 * The rail's own code is the **outer key** and the interior is what that
 * rail's adapter parses; see {@link RefundDestination}. One function so the
 * envelope is written in one place and no caller can flatten it, and so the
 * `msisdn` goes to the wire byte for byte as the merchant wrote it — this SDK
 * does not canonicalise a payee's number (the server does, and it requires a
 * leading `+`).
 */
function destinationForm(destination: RefundDestination): FormValue {
  return { [destination.payment_method_type]: { msisdn: destination.msisdn } };
}

/** `client.refunds` — see the module doc. */
export class RefundsResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  /**
   * `POST /v1/refunds`.
   *
   * Field order is the wire order `sdks/rust/tests/resources.rs` pins byte
   * for byte; an unset field is omitted from the body entirely.
   *
   * A `400` naming `destination` for a payee this rail will not take — and
   * for **no** payee at all, since both rails vpay carries need one; a `400`
   * naming `amount` for a partial refund on a rail that has no partials; a
   * `409` for an intent with nothing left to refund.
   *
   * See the module doc for what a `200` here does and does not mean.
   */
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
    if (params.destination !== undefined) {
      body["destination"] = destinationForm(params.destination);
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

  /**
   * `POST /v1/refunds/{id}` — the **metadata** update, and nothing else.
   *
   * See {@link UpdateRefundParams}: the other four fields of a refund are
   * fixed once it exists, the merge is key-wise, and an empty value deletes a
   * key. A refund whose metadata actually changes emits
   * `charge.refund.updated`; one sent no `metadata` is read back unchanged
   * and emits nothing.
   *
   * A `404` for a refund this merchant has none of, and a `400` naming the
   * parameter for anything but `metadata` — which this params type has no way
   * to send.
   */
  async update(
    id: string,
    params: UpdateRefundParams,
    options?: RequestOptions,
  ): Promise<Refund> {
    const body: Record<string, FormValue> = {};
    if (params.metadata !== undefined) {
      body["metadata"] = params.metadata;
    }
    return this.#http.request<Refund>(
      "POST",
      `/refunds/${encodeURIComponent(id)}`,
      body,
      options,
    );
  }

  /** `GET /v1/refunds` — this merchant's refunds, newest first. */
  async list(params?: ListRefundsParams): Promise<List<Refund>> {
    return this.#http.request<List<Refund>>("GET", "/refunds", params);
  }

  /**
   * `POST /v1/refunds/{id}/cancel` — **while it is still pending**. No
   * request fields.
   *
   * The reservation this refund holds against its intent is released and
   * `charge.refund.updated` is written. A `404` for a refund this merchant
   * has none of, and a `409` naming the current status for one that is no
   * longer `"pending"`.
   *
   * Nothing here reaches a rail: a cancel is a vpay-side release, because no
   * rail was ever told to start. See the module doc.
   */
  async cancel(id: string, options?: RequestOptions): Promise<Refund> {
    return this.#http.request<Refund>(
      "POST",
      `/refunds/${encodeURIComponent(id)}/cancel`,
      {},
      options,
    );
  }
}
