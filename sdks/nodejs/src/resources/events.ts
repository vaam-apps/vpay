import type { HttpClient } from "../http.js";
import type { Event, List, ListEventsParams } from "../types.js";

/** `/v1/events` — docs/flows/merchant-auth.md, "Resources". */
export class EventsResource {
  readonly #http: HttpClient;

  constructor(http: HttpClient) {
    this.#http = http;
  }

  async list(params?: ListEventsParams): Promise<List<Event>> {
    return this.#http.request<List<Event>>("GET", "/events", params);
  }
}
