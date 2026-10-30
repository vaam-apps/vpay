/**
 * The one place a credential is attached to a request to vpay.
 *
 * `/dash/v1` answers a request that carries a signature **and** an
 * `Authorization` header with `400` (ADR-0037): one caller, one credential.
 * This app signs nothing — the signed reader is VP-6b's — so what these tests
 * hold is the other half of that rule: no request this app builds carries two
 * credentials, none carries a credential it was not given, and none copies a
 * header from anywhere.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  bearerCredential,
  buildInit,
  credentialHeaders,
  dashFetch,
  SESSION_HEADER,
  sessionCredential,
  type Credential,
} from "./dash-fetch";
import { hangingVpay } from "../testing/hanging-fetch";

function headersOf(init: RequestInit): Record<string, string> {
  return init.headers as Record<string, string>;
}

/** Every header name that authenticates a request to `/dash/v1`. */
const CREDENTIAL_NAMES = [SESSION_HEADER, "authorization"];

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("a session credential", () => {
  it("travels in X-Vpay-Staff-Session and nowhere else", () => {
    const headers = credentialHeaders(sessionCredential("sess-1"));
    expect(headers).toEqual({ "x-vpay-staff-session": "sess-1" });
  });

  it("never adds an Authorization header to a staff route", () => {
    // A staff route called with a bearer attached is a request vpay may refuse
    // for carrying two credentials, and the bearer is a live /dash/v1
    // credential with no business on a sign-in step.
    const init = buildInit({
      method: "POST",
      form: { code: "123456" },
      credential: sessionCredential("sess-1"),
    });
    expect(Object.keys(headersOf(init))).not.toContain("authorization");
    expect(headersOf(init)[SESSION_HEADER]).toBe("sess-1");
  });
});

describe("a bearer credential", () => {
  it("travels in Authorization: Bearer and nowhere else", () => {
    const headers = credentialHeaders(bearerCredential("tok-1"));
    expect(headers).toEqual({ authorization: "Bearer tok-1" });
  });

  it("never adds a session header to a read", () => {
    const init = buildInit({
      method: "GET",
      credential: bearerCredential("tok-1"),
    });
    expect(Object.keys(headersOf(init))).not.toContain(SESSION_HEADER);
    expect(headersOf(init)["authorization"]).toBe("Bearer tok-1");
  });
});

describe("one credential per request", () => {
  it.each<[string, Credential | undefined, number]>([
    ["a session", sessionCredential("s"), 1],
    ["a bearer", bearerCredential("b"), 1],
    ["none (discovery, the password step)", undefined, 0],
  ])(
    "%s carries exactly %i credential header(s)",
    (_name, credential, count) => {
      const init = buildInit({
        method: "GET",
        ...(credential === undefined ? {} : { credential }),
      });
      const present = Object.keys(headersOf(init)).filter((name) =>
        CREDENTIAL_NAMES.includes(name.toLowerCase()),
      );
      expect(present).toHaveLength(count);
    },
  );

  it("refuses a console-key credential, which only a cast can produce", () => {
    // The variant's payload is `never`, so no value exists. A value forced past
    // the compiler must throw rather than be sent with no credential at all.
    const forced = { kind: "console-key" } as unknown as Credential;
    expect(() => credentialHeaders(forced)).toThrow(/VP-6b/);
    expect(() => buildInit({ method: "GET", credential: forced })).toThrow();
  });
});

describe("the header set is built here, never copied", () => {
  it("is accept, content-type for a body, and the credential — nothing else", () => {
    const get = headersOf(
      buildInit({ method: "GET", credential: bearerCredential("b") }),
    );
    expect(Object.keys(get).sort()).toEqual(["accept", "authorization"]);

    const post = headersOf(
      buildInit({
        method: "POST",
        form: { a: "1" },
        credential: sessionCredential("s"),
      }),
    );
    expect(Object.keys(post).sort()).toEqual([
      "accept",
      "content-type",
      SESSION_HEADER,
    ]);
  });

  it("takes no headers from a caller, so a browser's COSE message cannot be forwarded by being copied", async () => {
    // The browser's request to this app may carry anything; a server action
    // never sees those headers, and this function has no parameter for them.
    // What reaches the wire is checked end to end, through `fetch`.
    const fetchStub = vi.fn(() => Promise.resolve(new Response("{}")));
    vi.stubGlobal("fetch", fetchStub);

    await dashFetch("http://vpay/dash/v1/staff/session", {
      method: "GET",
      credential: sessionCredential("s"),
      // Extra properties are a type error; cast them in to prove the runtime
      // ignores what the type forbids.
      ...({
        headers: {
          authorization: "Bearer from-the-browser",
          signature: "COSE",
        },
      } as object),
    });

    const sent = fetchStub.mock.calls[0] as unknown as [string, RequestInit];
    const names = Object.keys(headersOf(sent[1])).map((name) =>
      name.toLowerCase(),
    );
    expect(names).not.toContain("authorization");
    expect(names).not.toContain("signature");
    expect(names).toContain(SESSION_HEADER);
  });
});

describe("the request itself", () => {
  it("never caches and never follows a redirect unless asked", () => {
    const plain = buildInit({ method: "GET" });
    expect(plain.cache).toBe("no-store");
    expect(plain.redirect).toBe("error");
    expect(buildInit({ method: "GET", redirect: "manual" }).redirect).toBe(
      "manual",
    );
  });

  it("form-encodes a body, because every staff handler extracts axum::Form", () => {
    const init = buildInit({
      method: "POST",
      form: { email: "a@b.c", n: "x y" },
    });
    expect(init.body).toBe("email=a%40b.c&n=x+y");
    expect(headersOf(init)["content-type"]).toBe(
      "application/x-www-form-urlencoded",
    );
  });
});

describe("a call that never answers", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("gives every call a five-second deadline, a fresh one each time", () => {
    // THE DECISIVE MUTATION: leave `signal` out of `buildInit`. A `fetch` has
    // no timeout of its own, and a wedged vpay would hold the render open.
    const { asked } = hangingVpay();
    const get = buildInit({ method: "GET" });
    const post = buildInit({ method: "POST", form: { a: "b" } });
    const authorize = buildInit({ method: "GET", redirect: "manual" });
    expect(get.signal).toBeInstanceOf(AbortSignal);
    expect(post.signal).toBeInstanceOf(AbortSignal);
    expect(authorize.signal).toBeInstanceOf(AbortSignal);
    expect(asked()).toEqual([5000, 5000, 5000]);
  });

  it("rejects when the deadline fires, which every caller already shapes into an outage", async () => {
    const { timeOut } = hangingVpay();
    const pending = dashFetch("http://vpay:8080/x", { method: "GET" });
    timeOut();
    await expect(pending).rejects.toMatchObject({ name: "TimeoutError" });
  });
});
