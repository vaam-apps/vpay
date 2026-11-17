import { vi } from "vitest";

/**
 * A vpay that accepts the connection and never answers, for the tests that
 * prove a timeout is an outage.
 *
 * `AbortSignal.timeout` is driven by an internal timer that neither fake
 * timers nor a 5-second test can reach, so the signal is replaced by one the
 * test aborts itself — and the stub `fetch` settles **only** through the
 * signal it was handed, the way a real one does. A call that carries no signal
 * therefore never settles and the test times out, which is what makes the
 * tests built on this fail when `dash-fetch.ts` stops attaching one.
 *
 * Installs two stubs; the caller's `afterEach` must `vi.unstubAllGlobals()`
 * and `vi.restoreAllMocks()`.
 */
export function hangingVpay(): {
  /** Fires the timeout every call made so far is waiting on. */
  timeOut: () => void;
  /** The durations `AbortSignal.timeout` was asked for. */
  asked: () => readonly number[];
} {
  const controller = new AbortController();
  const durations: number[] = [];
  vi.spyOn(AbortSignal, "timeout").mockImplementation((ms: number) => {
    durations.push(ms);
    return controller.signal;
  });
  vi.stubGlobal(
    "fetch",
    vi.fn(
      (_input: string | URL | Request, init?: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(init.signal?.reason as Error);
          });
        }),
    ),
  );
  return {
    timeOut: () => {
      controller.abort(
        new DOMException("The operation timed out.", "TimeoutError"),
      );
    },
    asked: () => durations,
  };
}
