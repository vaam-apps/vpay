/**
 * The Tauri host against a fake `invoke`/`Channel` pair.
 *
 * There is no Tauri IPC on Node, so the two members are injected. The casts
 * are deliberate and are on the *test's* side: `TauriHostDeps` is typed
 * `typeof invoke` / `typeof Channel` so that the real members need no cast
 * at a merchant, which leaves the fake as the thing that has to claim to be
 * one.
 */
import type { Channel, invoke } from "@tauri-apps/api/core";
import { describe, expect, it } from "vitest";

import { tauriHost } from "./host-tauri.js";
import type { CheckoutWindowEvent, ShowCheckoutRequest } from "./types.js";

interface InvokeCall {
  command: string;
  args: Record<string, unknown> | undefined;
}

/** The two members of `Channel` this package uses, and nothing else. */
class FakeChannel<T> {
  static last: FakeChannel<CheckoutWindowEvent> | undefined;

  onmessage: (message: T) => void = () => undefined;

  constructor() {
    FakeChannel.last = this as unknown as FakeChannel<CheckoutWindowEvent>;
  }
}

function harness(options: { rejectWith?: unknown } = {}): {
  calls: InvokeCall[];
  deps: { invoke: typeof invoke; Channel: typeof Channel };
} {
  const calls: InvokeCall[] = [];
  FakeChannel.last = undefined;
  const fakeInvoke = (
    command: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> => {
    calls.push({ command, args });
    if (options.rejectWith === undefined) {
      return Promise.resolve(undefined);
    }
    // A real `plugin:vpay-checkout|show` rejection is a plain **string**
    // (`already_open`, `no_activity`, `invalid_url`), not an `Error`. That
    // is the shape the host has to survive, so it is the shape the fake
    // produces.
    // eslint-disable-next-line @typescript-eslint/prefer-promise-reject-errors
    return Promise.reject(options.rejectWith);
  };
  return {
    calls,
    deps: {
      invoke: fakeInvoke as unknown as typeof invoke,
      Channel: FakeChannel as unknown as typeof Channel,
    },
  };
}

const REQUEST: ShowCheckoutRequest = {
  url: "https://checkout.example/c/cs_123?key=pk_test_1#cs_123_secret_abc",
  stopUrls: [
    { scheme: "https", host: "shop.example", port: 443, path: "/return" },
  ],
  allowInsecureUrl: false,
};

describe("tauriHost.show", () => {
  it("invokes plugin:vpay-checkout|show with the four camelCase keys the wire contract names", async () => {
    const { calls, deps } = harness();

    await tauriHost(deps).show(REQUEST, () => undefined);

    expect(calls).toHaveLength(1);
    expect(calls[0]?.command).toBe("plugin:vpay-checkout|show");
    expect(Object.keys(calls[0]?.args ?? {}).sort()).toEqual([
      "allowInsecureUrl",
      "onEvent",
      "stopUrls",
      "url",
    ]);
    expect(calls[0]?.args?.["url"]).toBe(REQUEST.url);
    expect(calls[0]?.args?.["stopUrls"]).toEqual(REQUEST.stopUrls);
    expect(calls[0]?.args?.["allowInsecureUrl"]).toBe(false);
    // The Channel itself goes on the wire: Tauri serialises it to the id
    // the Rust side turns back into a `tauri::ipc::Channel`.
    expect(calls[0]?.args?.["onEvent"]).toBe(FakeChannel.last);
  });

  it("forwards the channel's one message to onEvent", async () => {
    const { deps } = harness();
    const seen: CheckoutWindowEvent[] = [];

    await tauriHost(deps).show(REQUEST, (event) => seen.push(event));
    FakeChannel.last?.onmessage({
      outcome: "stopUrlReached",
      reachedUrl: "https://shop.example/return?x=1",
    });

    expect(seen).toEqual([
      {
        outcome: "stopUrlReached",
        reachedUrl: "https://shop.example/return?x=1",
      },
    ]);
  });

  it("ignores a second channel message — exactly one event per show", async () => {
    const { deps } = harness();
    const seen: CheckoutWindowEvent[] = [];

    await tauriHost(deps).show(REQUEST, (event) => seen.push(event));
    FakeChannel.last?.onmessage({ outcome: "dismissed", reachedUrl: null });
    FakeChannel.last?.onmessage({
      outcome: "stopUrlReached",
      reachedUrl: null,
    });

    expect(seen).toHaveLength(1);
    expect(seen[0]?.outcome).toBe("dismissed");
  });

  it("a rejected invoke rejects show, so start can map it to platform_window_failed", async () => {
    const { deps } = harness({ rejectWith: "already_open" });

    await expect(tauriHost(deps).show(REQUEST, () => undefined)).rejects.toBe(
      "already_open",
    );
  });
});

describe("tauriHost.dismiss", () => {
  it("invokes plugin:vpay-checkout|dismiss with no arguments", async () => {
    const { calls, deps } = harness();

    await tauriHost(deps).dismiss();

    expect(calls).toEqual([
      { command: "plugin:vpay-checkout|dismiss", args: undefined },
    ]);
  });
});
