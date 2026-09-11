/**
 * The BFF, exercised as the thing a browser would reach: a real `Request` in,
 * a real `Response` out, one stubbed `fetch` standing in for vpay.
 *
 * Three cases here are the ones this lane exists for, and each is written so
 * that deleting the code it is about turns it red:
 *
 *  1. `reaches vpay not at all when the request carries no session` — the
 *     assertion is on the **call count of the stub**, not on the status.
 *     A `401` produced after an upstream round trip is a different program
 *     with the same status code, and only the first of the two is a surface
 *     an unauthenticated caller cannot use to make this server talk to vpay.
 *  2. `refuses a request whose Origin is not this deployment's` — delete the
 *     `originIsAllowed` call in `apiIsSameOrigin` and it fails. It is written
 *     with `Sec-Fetch-Site: same-origin` on purpose, so that the *other* half
 *     of that function cannot pass it for free.
 *  3. `puts the bearer token in no header and no body, ever` — the upstream
 *     stub echoes the token into a response header, into an extra top-level
 *     body field and into the envelope's `url`, and the assertion is against
 *     the **serialised** response: status line, every header, and the body
 *     text actually read back off it.
 *
 * The token is spelled out as a distinctive literal for that last case, so a
 * substring search cannot match it by accident.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DashboardConfig } from "../config/settings";
import { INTENT, OTHER_INTENT } from "../testing/fixtures";
import {
  apiIsSameOrigin,
  paymentIntentResponse,
  paymentIntentsListResponse,
  sessionCookieOf,
} from "./bff";
import { SESSION_COOKIE } from "./cookies";

const CONFIG: DashboardConfig = {
  apiBaseUrl: "http://vpay-server:8080",
  clientId: "vpay-dashboard",
  redirectUri: "http://localhost:3000/dash/v1/callback",
  scope: "dashboard:read",
  publicOrigin: "https://dash.example",
};

/** The value that must never leave this process. */
const BEARER = "bearer-token-that-must-never-be-served-9c1f";
const SESSION = "the-staff-session-token";

/** A staff session vpay would answer with: signed in, token in hand, fresh. */
function liveSession() {
  return {
    staff_id: "stf_example_1",
    display_name: "Example Operator",
    email: "operator@example.test",
    merchant_id: "mch_example_1",
    password_change_required: false,
    access_token: BEARER,
    access_token_expires_at: new Date(Date.now() + 600_000).toISOString(),
    access_token_ttl_seconds: 900,
  };
}

/** One upstream request the stub saw. */
interface Seen {
  readonly url: string;
  readonly authorization: string | null;
}

interface StubOptions {
  /** What `/staff/session` answers, or a status to refuse it with. */
  readonly session?: unknown;
  readonly sessionStatus?: number;
  /** What the `/payment_intents` read answers. */
  readonly read?: unknown;
  readonly readStatus?: number;
  /** Extra response headers on the `/payment_intents` read. */
  readonly readHeaders?: Record<string, string>;
}

/**
 * A `fetch` standing in for vpay, recording every request it was given.
 *
 * Nothing else is stubbed. The origin check, the cookie parse, the gate, the
 * projection and the response construction are all the shipping code.
 */
function stub(seen: Seen[], options: StubOptions = {}) {
  return vi.fn((input: string | URL | Request, init?: RequestInit) => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.href
          : input.url;
    const headers = (init?.headers ?? {}) as Record<string, string>;
    seen.push({ url, authorization: headers["authorization"] ?? null });

    if (url.includes("/staff/session")) {
      const status = options.sessionStatus ?? 200;
      return Promise.resolve(
        new Response(JSON.stringify(options.session ?? liveSession()), {
          status,
          headers: { "content-type": "application/json" },
        }),
      );
    }
    return Promise.resolve(
      new Response(
        JSON.stringify(
          options.read ?? {
            object: "list",
            data: [INTENT, OTHER_INTENT],
            has_more: true,
            url: "/dash/v1/payment_intents",
          },
        ),
        {
          status: options.readStatus ?? 200,
          headers: {
            "content-type": "application/json",
            ...(options.readHeaders ?? {}),
          },
        },
      ),
    );
  });
}

/** The headers a browser puts on a same-origin `fetch` from this app's page. */
const FROM_THE_PAGE = {
  host: "dash.example",
  "sec-fetch-site": "same-origin",
  cookie: `${SESSION_COOKIE}=${SESSION}`,
};

function request(
  url = "https://dash.example/api/dash/payment_intents",
  headers: Record<string, string> = FROM_THE_PAGE,
): Request {
  return new Request(url, { headers });
}

/** Status line, every header, and the body — everything that goes on the wire. */
async function serialised(response: Response): Promise<string> {
  const headers = [...response.headers.entries()]
    .map(([name, value]) => `${name}: ${value}`)
    .join("\n");
  return `HTTP ${response.status}\n${headers}\n\n${await response.text()}`;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("who may call it at all", () => {
  it("reaches vpay not at all when the request carries no session", async () => {
    // THE FIRST DECISIVE CASE.
    //
    // The assertion is the call count, not the status. An endpoint that
    // answers 401 *after* asking vpay about the absent session is a surface
    // any caller on the internet can use to make this server open a
    // connection to vpay, once per request, for free.
    const seen: Seen[] = [];
    const fetchStub = stub(seen);
    vi.stubGlobal("fetch", fetchStub);

    const response = await paymentIntentsListResponse(
      request("https://dash.example/api/dash/payment_intents", {
        host: "dash.example",
        "sec-fetch-site": "same-origin",
      }),
      CONFIG,
    );

    expect(fetchStub).not.toHaveBeenCalled();
    expect(seen).toEqual([]);
    expect(response.status).toBe(401);
  });

  it("reaches vpay not at all when the session cookie is empty", async () => {
    const seen: Seen[] = [];
    const fetchStub = stub(seen);
    vi.stubGlobal("fetch", fetchStub);

    const response = await paymentIntentsListResponse(
      request("https://dash.example/api/dash/payment_intents", {
        ...FROM_THE_PAGE,
        cookie: `${SESSION_COOKIE}=`,
      }),
      CONFIG,
    );

    expect(fetchStub).not.toHaveBeenCalled();
    expect(response.status).toBe(401);
  });

  it("refuses a request whose Origin is not this deployment's", async () => {
    // THE SECOND DECISIVE CASE.
    //
    // `Sec-Fetch-Site: same-origin` is on this request deliberately: it is
    // the value the *other* half of `apiIsSameOrigin` accepts, so this case
    // can only be refused by the `originIsAllowed` call. Delete that call and
    // the request is served — 200 rather than 403 — and this fails.
    //
    // A real browser never sends this pair; a caller that is not a browser
    // can send anything, which is the whole reason both halves are checked.
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    const response = await paymentIntentsListResponse(
      request("https://dash.example/api/dash/payment_intents", {
        ...FROM_THE_PAGE,
        origin: "https://evil.example",
      }),
      CONFIG,
    );

    expect(response.status).toBe(403);
    expect(seen).toEqual([]);
  });

  it("refuses the shape a real cross-site call would have", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    const response = await paymentIntentsListResponse(
      request("https://dash.example/api/dash/payment_intents", {
        ...FROM_THE_PAGE,
        origin: "https://evil.example",
        "sec-fetch-site": "cross-site",
      }),
      CONFIG,
    );

    expect(response.status).toBe(403);
    expect(seen).toEqual([]);
  });

  it("serves this app's own page", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    const response = await paymentIntentsListResponse(request(), CONFIG);

    expect(response.status).toBe(200);
    expect(seen).toHaveLength(2);
  });
});

describe("the origin rule itself", () => {
  it("accepts a same-origin fetch, which carries no Origin at all", () => {
    // A browser sends no `Origin` on a same-origin GET, and script cannot add
    // one — it is a forbidden header name. A rule that demanded it would
    // refuse every honest call this surface exists to serve.
    const headers = new Headers({ "sec-fetch-site": "same-origin" });
    expect(apiIsSameOrigin(headers, "https://dash.example")).toBe(true);
  });

  it("refuses a request that carries no fetch metadata at all", () => {
    // `curl`, or a browser older than 2020. The opposite of what
    // `signed-out.ts` does with the same absence, and deliberately: that
    // route serves a person arriving on a page, this one serves this app's
    // own script and nothing else.
    expect(apiIsSameOrigin(new Headers({}), "https://dash.example")).toBe(
      false,
    );
  });

  it("refuses a URL somebody typed into the address bar", () => {
    // `Sec-Fetch-Site: none` is a bookmark or a typed URL. This is not a
    // page, and an operator who lands on it should get a refusal rather than
    // their merchant's payments as raw JSON.
    const headers = new Headers({ "sec-fetch-site": "none" });
    expect(apiIsSameOrigin(headers, "https://dash.example")).toBe(false);
  });

  it("refuses same-site, which is another host on the same domain", () => {
    const headers = new Headers({ "sec-fetch-site": "same-site" });
    expect(apiIsSameOrigin(headers, "https://dash.example")).toBe(false);
  });

  it("falls back to Host exactly as csrf.ts does when nothing is configured", () => {
    const matching = new Headers({
      origin: "https://dash.example",
      host: "dash.example",
      "sec-fetch-site": "same-origin",
    });
    expect(apiIsSameOrigin(matching, null)).toBe(true);

    const mismatched = new Headers({
      origin: "https://evil.example",
      host: "dash.example",
      "sec-fetch-site": "same-origin",
    });
    expect(apiIsSameOrigin(mismatched, null)).toBe(false);
  });
});

describe("the session cookie", () => {
  it("is read by its own name and not by a suffix of it", () => {
    const headers = new Headers({
      cookie: `x_${SESSION_COOKIE}=wrong; ${SESSION_COOKIE}=right`,
    });
    expect(sessionCookieOf(headers)).toBe("right");
  });

  it("keeps a value containing an equals sign", () => {
    const headers = new Headers({ cookie: `${SESSION_COOKIE}=a=b=c` });
    expect(sessionCookieOf(headers)).toBe("a=b=c");
  });

  it("is null when there is no cookie header at all", () => {
    expect(sessionCookieOf(new Headers({}))).toBeNull();
  });
});

describe("what goes back on the wire", () => {
  it("puts the bearer token in no header and no body, ever", async () => {
    // THE THIRD DECISIVE CASE.
    //
    // vpay is stubbed to echo the token everywhere a careless proxy would
    // carry it through: a response header, an unexpected top-level field, and
    // the envelope's own `url`. The assertion is on the serialised response —
    // status line, every header, and the body as it is actually read back —
    // rather than on a mock's arguments, because "the code did not mean to
    // send it" and "it is not on the wire" are different claims.
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, {
        read: {
          object: "list",
          data: [INTENT],
          has_more: false,
          url: `/dash/v1/payment_intents?token=${BEARER}`,
          access_token: BEARER,
        },
        readHeaders: {
          authorization: `Bearer ${BEARER}`,
          "x-echoed-credential": BEARER,
          "set-cookie": `leaked=${BEARER}`,
        },
      }),
    );

    const response = await paymentIntentsListResponse(request(), CONFIG);
    const wire = await serialised(response);

    // The stub really did carry it, so this assertion is about the proxy and
    // not about an upstream that never had one.
    expect(seen.at(-1)?.authorization).toBe(`Bearer ${BEARER}`);
    expect(response.status).toBe(200);
    expect(wire).not.toContain(BEARER);
    expect(response.headers.get("x-echoed-credential")).toBeNull();
    expect(response.headers.get("authorization")).toBeNull();
    expect(response.headers.get("set-cookie")).toBeNull();
  });

  it("puts it in no part of the detail read either", async () => {
    // The same case for the other handler, and it is the one that pins the
    // *projection* rather than the framing: `getOne` answers the whole parsed
    // upstream document, so the named keys in `paymentIntentResponse` are the
    // only thing standing between an upstream field nobody expected and the
    // browser. Serve `result.value` verbatim instead and this goes red.
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, {
        read: {
          object: "dashboard.payment_detail",
          payment_intent: INTENT,
          charge: null,
          refunds: [],
          events: [],
          access_token: BEARER,
          staff_session: BEARER,
        },
        readHeaders: {
          authorization: `Bearer ${BEARER}`,
          "x-echoed-credential": BEARER,
        },
      }),
    );

    const response = await paymentIntentResponse(
      request("https://dash.example/api/dash/payment_intents/pi_example_1"),
      "pi_example_1",
      CONFIG,
    );
    const wire = await serialised(response);

    expect(seen.at(-1)?.authorization).toBe(`Bearer ${BEARER}`);
    expect(response.status).toBe(200);
    expect(wire).not.toContain(BEARER);
    expect(response.headers.get("x-echoed-credential")).toBeNull();
  });

  it("puts the bearer token in no part of a refusal either", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal(
      "fetch",
      stub(seen, {
        readStatus: 503,
        read: { error: { message: `upstream said ${BEARER}` } },
        readHeaders: { "x-echoed-credential": BEARER },
      }),
    );

    const response = await paymentIntentsListResponse(request(), CONFIG);
    const wire = await serialised(response);

    expect(response.status).toBe(503);
    // The message vpay wrote is rendered, so this case only proves the
    // headers and the framing carry nothing — which is what it claims.
    expect(response.headers.get("x-echoed-credential")).toBeNull();
    expect(wire).toContain("upstream said");
  });

  it("is never cached, by this browser or by anything in front of it", async () => {
    vi.stubGlobal("fetch", stub([]));
    const response = await paymentIntentsListResponse(request(), CONFIG);
    expect(response.headers.get("cache-control")).toBe("no-store");
  });

  it("answers the page and its two cursors, and nothing vpay sent besides", async () => {
    vi.stubGlobal("fetch", stub([]));
    const response = await paymentIntentsListResponse(request(), CONFIG);
    const body = (await response.json()) as Record<string, unknown>;

    expect(Object.keys(body).sort()).toEqual([
      "cursor",
      "data",
      "has_more",
      "object",
    ]);
    expect(body["has_more"]).toBe(true);
    expect(body["cursor"]).toEqual({ newer: null, older: OTHER_INTENT.id });
  });
});

describe("what the caller may and may not steer", () => {
  it("forwards the five filters it knows and no parameter it does not", async () => {
    // The tenancy decision is vpay's: `MerchantScope::for_dashboard` comes
    // from the YAML dashboard binding, not from the token and certainly not
    // from a query string. These parameters are not denied — they are never
    // read, which is the property that also holds for the next one somebody
    // invents.
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    await paymentIntentsListResponse(
      request(
        "https://dash.example/api/dash/payment_intents?status=succeeded" +
          "&merchant_id=mch_someone_else&merchant=mch_someone_else" +
          "&audience=vpay-merchant&scope=merchant:write&aud=x&limit=9999",
      ),
      CONFIG,
    );

    const upstream = new URL(seen.at(-1)?.url ?? "");
    expect(upstream.pathname).toBe("/dash/v1/payment_intents");
    expect(upstream.searchParams.get("status")).toBe("succeeded");
    expect(upstream.searchParams.get("limit")).toBe("25");
    expect([...upstream.searchParams.keys()].sort()).toEqual([
      "limit",
      "status",
    ]);
  });

  it("takes the FIRST value of a repeated parameter, as the page does", async () => {
    // `payments-query.ts`'s `one()` is explicit that the first wins, and
    // `Object.fromEntries(searchParams.entries())` — the obvious spelling for
    // turning a URL into the record it takes — keeps the last. The page and
    // this surface disagreeing about what one URL asks for is exactly what
    // the seam exists to prevent.
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen));

    await paymentIntentsListResponse(
      request(
        "https://dash.example/api/dash/payment_intents?status=succeeded&status=canceled",
      ),
      CONFIG,
    );

    const upstream = new URL(seen.at(-1)?.url ?? "");
    expect(upstream.searchParams.get("status")).toBe("succeeded");
  });

  it("cannot be pointed at another upstream route through the id", async () => {
    const seen: Seen[] = [];
    vi.stubGlobal("fetch", stub(seen, { read: {} }));

    await paymentIntentResponse(
      request("https://dash.example/api/dash/payment_intents/x"),
      "../staff/session",
      CONFIG,
    );

    expect(seen.at(-1)?.url).toBe(
      "http://vpay-server:8080/dash/v1/payment_intents/..%2Fstaff%2Fsession",
    );
  });
});

describe("the session gate", () => {
  it("carries a 401 through as a 401, so a client can end the session", async () => {
    vi.stubGlobal("fetch", stub([], { sessionStatus: 401, session: {} }));
    const response = await paymentIntentsListResponse(request(), CONFIG);
    expect(response.status).toBe(401);
  });

  it("carries a 403 through as a 403 — an outage, never a sign-out", async () => {
    // issue #88 item 2. A client applying `refusalFor` cannot tell an outage
    // from a dead session if this layer flattens the status, and a rolling
    // vpay restart signing every staff member out is what that costs.
    vi.stubGlobal("fetch", stub([], { readStatus: 403, read: {} }));
    const response = await paymentIntentsListResponse(request(), CONFIG);
    expect(response.status).toBe(403);
  });

  it("answers 502 for a vpay it could not reach at all", async () => {
    // `api.ts` turns a rejected fetch into `status: 0`, which is not an HTTP
    // status and must not be written as one.
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new Error("ECONNREFUSED"))),
    );
    const response = await paymentIntentsListResponse(request(), CONFIG);
    expect(response.status).toBe(502);
  });

  it("refuses a session that still owes a password change", async () => {
    // ADR-0017 decision 1: such a session cannot obtain a `/dash/v1` token at
    // all, so there is nothing to serve it with. A page redirects; this
    // cannot, and says so rather than pretending.
    vi.stubGlobal(
      "fetch",
      stub([], {
        session: { ...liveSession(), password_change_required: true },
      }),
    );
    const response = await paymentIntentsListResponse(request(), CONFIG);
    expect(response.status).toBe(403);
  });

  it("refuses when the deployment is not configured, before anything else", async () => {
    const seen: Seen[] = [];
    const fetchStub = stub(seen);
    vi.stubGlobal("fetch", fetchStub);
    const response = await paymentIntentsListResponse(request(), null);
    expect(response.status).toBe(503);
    expect(fetchStub).not.toHaveBeenCalled();
  });

  it("carries the detail read's 404 through with vpay's own sentence", async () => {
    // `/dash/v1` answers the identical body for a foreign merchant's id and
    // for one that never existed. Nothing here may add the id to one branch
    // and not the other.
    vi.stubGlobal(
      "fetch",
      stub([], {
        readStatus: 404,
        read: { error: { message: "No such payment_intent." } },
      }),
    );
    const response = await paymentIntentResponse(
      request("https://dash.example/api/dash/payment_intents/pi_example_1"),
      "pi_example_1",
      CONFIG,
    );
    const body = (await response.json()) as { error: { message: string } };
    expect(response.status).toBe(404);
    expect(body.error.message).toBe("No such payment_intent.");
    expect(JSON.stringify(body)).not.toContain("pi_example_1");
  });
});
