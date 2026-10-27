import { initTRPC } from "@trpc/server";
import { afterEach, describe, expect, it, vi } from "vitest";

// A router whose one procedure fails the way a failed call to vpay does.
vi.mock("@/server/routers", () => {
  const t = initTRPC.create();
  return {
    appRouter: t.router({
      boom: t.procedure.mutation(() => {
        throw new Error("the sealing module could not be loaded", {
          cause: new Error("ENOENT: /ROOT/wasm"),
        });
      }),
    }),
  };
});
vi.mock("@/server/context", () => ({ shopContext: () => ({}) }));

import { POST } from "./route";

describe("the tRPC route", () => {
  afterEach(() => vi.restoreAllMocks());

  it("reports a failed procedure on the server's own log", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => {});
    const response = await POST(
      new Request("http://shop.test/api/trpc/boom", {
        method: "POST",
        headers: { "content-type": "application/json" },
      }),
    );
    expect(response.status).toBe(500);
    expect(log).toHaveBeenCalledTimes(1);
    const [message, cause] = log.mock.calls[0] as [string, Error];
    expect(message).toContain("tRPC boom failed");
    expect(message).toContain("the sealing module could not be loaded");
    // tRPC wraps what a procedure throws; the original, with its own cause
    // chain, is what the log must carry.
    expect(cause.message).toContain("sealing module");
    expect((cause.cause as Error).message).toContain("ENOENT");
  });
});
