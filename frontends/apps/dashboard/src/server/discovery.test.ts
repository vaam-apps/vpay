/**
 * `GET /.well-known/vpay-auth`, read by the login page.
 *
 * The line this file defends: **a discovery that could not be read is never
 * read as "this deployment serves no password sign-in".** The first would send
 * a person to an operator for a fault that heals itself.
 */
import { afterEach, describe, expect, it, vi } from "vitest";

import { hangingVpay } from "../testing/hanging-fetch";
import { DISCOVERY_PATH, readDiscovery } from "./discovery";

function answer(body: unknown, init: ResponseInit = {}): void {
  vi.stubGlobal(
    "fetch",
    vi.fn(() =>
      Promise.resolve(
        new Response(typeof body === "string" ? body : JSON.stringify(body), {
          status: 200,
          ...init,
        }),
      ),
    ),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("a deployment that serves human_login", () => {
  it("is `served`, with the methods it listed", async () => {
    answer({ methods: ["keypair", "human_login"] });
    expect(await readDiscovery("http://vpay:8080")).toEqual({
      kind: "served",
      methods: ["keypair", "human_login"],
    });
  });

  it("asks the well-known path, with no credential", async () => {
    answer({ methods: ["human_login"] });
    await readDiscovery("http://vpay:8080");
    const [url, init] = (
      vi.mocked(fetch).mock.calls[0] as unknown as [string, RequestInit]
    ).slice(0, 2) as [string, RequestInit];
    expect(url).toBe(`http://vpay:8080${DISCOVERY_PATH}`);
    expect(DISCOVERY_PATH).toBe("/.well-known/vpay-auth");
    const names = Object.keys(init.headers as Record<string, string>);
    expect(names).not.toContain("authorization");
    expect(names).not.toContain("x-vpay-staff-session");
  });
});

describe("a deployment that does not", () => {
  it("is `paired-key-only` when the list parsed and names no human_login", async () => {
    answer({ methods: ["keypair", "machine_tokens"] });
    const result = await readDiscovery("http://vpay:8080");
    expect(result.kind).toBe("paired-key-only");
  });

  it("is `paired-key-only` for an empty list", async () => {
    answer({ methods: [] });
    expect((await readDiscovery("http://vpay:8080")).kind).toBe(
      "paired-key-only",
    );
  });
});

describe("a discovery that could not be read", () => {
  it("is an outage, with the request id, when vpay answered 503", async () => {
    answer(
      { error: { message: "Starting up." } },
      { status: 503, headers: { "request-id": "req_01J8" } },
    );
    const result = await readDiscovery("http://vpay:8080");
    expect(result.kind).toBe("unreadable");
    if (result.kind === "unreadable") {
      expect(result.failure.status).toBe(503);
      expect(result.failure.requestId).toBe("req_01J8");
      expect(result.failure.message).toBe("Starting up.");
    }
  });

  it("is an outage when the connection fails", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() => Promise.reject(new Error("ECONNREFUSED"))),
    );
    const result = await readDiscovery("http://vpay:8080");
    expect(result.kind).toBe("unreadable");
    if (result.kind === "unreadable") {
      expect(result.failure.status).toBe(0);
      expect(result.failure.message).toContain("ECONNREFUSED");
    }
  });

  it("is an outage for a vpay that has no such route — it is not 'paired key only'", async () => {
    answer({ error: { message: "No such route." } }, { status: 404 });
    expect((await readDiscovery("http://vpay:8080")).kind).toBe("unreadable");
  });

  it.each<[string, unknown]>([
    ["an HTML error page behind a 200", "<html><body>502</body></html>"],
    ["a body with no methods", {}],
    ["methods that is not an array", { methods: "human_login" }],
    ["methods with a non-string in it", { methods: ["keypair", 7] }],
    ["null", "null"],
  ])(
    "is an outage for %s, never a deployment without a sign-in",
    async (_name, body) => {
      answer(body);
      expect((await readDiscovery("http://vpay:8080")).kind).toBe("unreadable");
    },
  );
});

describe("a vpay that accepts the connection and never answers", () => {
  it("is `unreadable` — an outage — and never `paired-key-only`", async () => {
    // THE DECISIVE MUTATION: no deadline on the discovery call. Then this
    // never settles at all, and a login page waits on it with nothing shown;
    // were the timeout read as "no human_login", an operator would be sent to
    // fix a deployment that is merely wedged.
    const { timeOut, asked } = hangingVpay();
    const pending = readDiscovery("http://vpay:8080");
    expect(asked()).toEqual([5000]);
    timeOut();
    const result = await pending;
    expect(result.kind).toBe("unreadable");
    if (result.kind === "unreadable") {
      expect(result.failure.status).toBe(0);
      expect(result.failure.message).toMatch(/could not be reached/);
    }
  });
});
