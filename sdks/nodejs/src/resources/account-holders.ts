import type { HttpClient } from "../http.js";
import type { FormValue } from "../form.js";
import type { AccountHolder, RetrieveAccountHolderParams } from "../types.js";

/**
 * `/v1/account_holders` — whose mobile-money account a number is
 * (issue #47).
 *
 * The one resource on this surface that returns a **third party's** name.
 * vpay projects the rail's answer down to a name, logs neither the name nor
 * the unmasked number, and stores nothing;
 * `docs/flows/account-holder-lookup.md` is the policy, and an integrator
 * holding the returned name inherits the same obligation.
 */
export class AccountHoldersResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  /**
   * `GET /v1/account_holders`.
   *
   * `name` is `null` when **the rail has no record** of the number. It is
   * never `null` because vpay could not ask: that case throws
   * (`VpayApiError`, `502` for a rail that could not be reached, `400` for a
   * rail with no such API), so a caller matching a name can tell "not
   * registered" from "not checked". Both are refusals; only one of them is
   * the payer's to fix.
   *
   * Nothing is validated locally — not the number's shape, not whether the
   * rail can answer. The first is a *market* rule vpay owns and may widen,
   * and an SDK copy would refuse offline a number a later server version
   * accepts; the second is a property of the deployment. Both come back as a
   * `400` naming the parameter, which is the same information one round trip
   * later. `sdks/rust`'s `RetrieveAccountHolderParams` takes the identical
   * line, which is what keeps the two at parity on the refusals as well as
   * the acceptances.
   */
  async retrieve(params: RetrieveAccountHolderParams): Promise<AccountHolder> {
    // Built explicitly rather than spread from `params`, and in this order:
    // the encoder walks insertion order, and `sdks/rust/tests/resources.rs`
    // pins the identical query string byte for byte. A spread would make the
    // wire depend on how a caller happened to write their object literal.
    const query: Record<string, FormValue> = {
      msisdn: params.msisdn,
      payment_method_type: params.payment_method_type,
    };
    return this.#http.request<AccountHolder>("GET", "/account_holders", query);
  }
}
