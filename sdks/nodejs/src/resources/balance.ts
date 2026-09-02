import type { HttpClient } from "../http.js";
import type { Balance } from "../types.js";

/** `/v1/balance` — docs/flows/merchant-auth.md, "Resources". */
export class BalanceResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  async retrieve(): Promise<Balance> {
    return this.#http.request<Balance>("GET", "/balance");
  }
}
