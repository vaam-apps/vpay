/**
 * The types this package's public surface is written in: the wire contract
 * it shares with every host, the result union a merchant switches over, and
 * the two seams (`CheckoutHost`, `VpayClock`) that make the state machine
 * testable without a window and without a timer.
 *
 * The wire types are the TypeScript side of `src/models.rs`'s
 * `#[serde(rename_all = "camelCase")]` shapes and of the Kotlin and Swift
 * hosts' own. They are the plugin's **JS** wire, not vpay's `/v1`, which is
 * why they are camelCase where every `@vaam-apps/vpay-stripe-js` type is
 * snake_case.
 */
import type { VpayError } from "./errors.js";

/**
 * One stop URL, normalised: scheme, host, port and path only. Query and
 * fragment are never compared (D2), so they are not even carried.
 *
 * Built by {@link StopUrlSpec.fromConfigured} from the session's own
 * `success_url`/`cancel_url` — never configured by the merchant on this
 * side, which is the property that stops the two disagreeing.
 */
export interface CheckoutStopUrl {
  scheme: string;
  host: string;
  /** Default ports normalised: 443 for https, 80 for http, 0 otherwise. */
  port: number;
  path: string;
}

/**
 * The `plugin:vpay-checkout|show` arguments, key for key.
 *
 * `url` is the session's own hosted URL and the host loads exactly it —
 * nothing here re-derives, rewrites or inspects it, and in particular
 * nothing reads its fragment a second time.
 */
export interface ShowCheckoutRequest {
  url: string;
  stopUrls: CheckoutStopUrl[];
  /**
   * D6's named opt-in, forwarded so a host never re-derives it from the
   * scheme. A host that saw `http://` and decided for itself would be a
   * second policy that could disagree with this one.
   */
  allowInsecureUrl: boolean;
}

/**
 * The two things a checkout window can report. There is deliberately no
 * `succeeded`, `canceled` or `failed`: the host decides nothing about money
 * (D1).
 */
export type CheckoutWindowOutcome = "dismissed" | "stopUrlReached";

/**
 * The one event a host sends, exactly once per {@link CheckoutHost.show}.
 *
 * `reachedUrl` is **diagnostics only**. Nothing in this package reads it,
 * and D1 is the reason: an outcome inferred from a URL is an outcome
 * nobody observed. `resolveAfterStopUrlReached` does not even take a URL
 * parameter, so there is no place to pass it.
 */
export interface CheckoutWindowEvent {
  outcome: CheckoutWindowOutcome;
  reachedUrl: string | null;
}

/**
 * Where the payer's checkout page is shown. Three implementations ship with
 * this package — {@link tauriHost}, {@link webHost} and the
 * {@link defaultHost} that picks between them — and a fourth, a fake, is
 * what the suite drives the state machine with.
 */
export interface CheckoutHost {
  /**
   * Shows `request.url` and resolves once the window is **showing**.
   *
   * `onEvent` is called at most once, with the one outcome the window
   * reported. It is wired before this promise is awaited, deliberately:
   * see {@link VpayCheckout.start}.
   *
   * # Errors
   *
   * Rejects when the window could not be opened at all. The rejected value
   * is never read: `start` maps every rejection to the fixed
   * `platform_window_failed` error, because a host's own message can quote
   * the URL and that URL carries the session secret (D6).
   */
  show(
    request: ShowCheckoutRequest,
    onEvent: (event: CheckoutWindowEvent) => void,
  ): Promise<void>;
  /** Closes the window if one is open; a no-op otherwise. */
  dismiss(): Promise<void>;
}

/**
 * Injected so every wait in the state machine is fake in a test and real on
 * a device — the "no timers of its own" half of `controller.ts`'s purity.
 */
export interface VpayClock {
  /** Milliseconds since the epoch. */
  now(): number;
  delay(ms: number): Promise<void>;
}

/**
 * The real clock. What {@link VpayCheckout} uses unless a caller replaces
 * it.
 */
export const systemClock: VpayClock = {
  now: () => Date.now(),
  delay: (ms: number) =>
    new Promise<void>((resolve) => {
      setTimeout(resolve, ms);
    }),
};

/**
 * D4's result type. `pending` exists so this package never has to choose
 * between lying (`succeeded`/`canceled` for a payment still in flight) and
 * throwing.
 *
 * A discriminated union, so every `switch (result.kind)` a merchant's app
 * writes is exhaustive at compile time.
 */
export type VpayCheckoutResult =
  /**
   * D1: a UI fact, not a settlement. A merchant's server must still act on
   * `payment_intent.succeeded` from the webhook — stated here rather than
   * only in the design doc, because this is the type a merchant reads.
   */
  | { kind: "succeeded"; sessionId: string; paymentIntentId: string }
  /**
   * The rail failed the payment. `code` is `vpay_core::failure`'s closed
   * vocabulary (`docs/flows/failures.md`); `providerMessage` is the rail's
   * own words, carried as data and labelled as the provider's.
   */
  | {
      kind: "failed";
      sessionId: string;
      paymentIntentId: string;
      code: string | null;
      providerMessage: string | null;
    }
  /**
   * The intent itself reached `canceled` — never produced merely because
   * the payer dismissed the window (D4); only a poll of the intent can
   * produce this.
   */
  | { kind: "canceled"; sessionId: string; paymentIntentId: string }
  /**
   * D4: still processing. Produced when a poll's budget elapsed with the
   * intent not yet terminal — after a reached stop URL, or after a
   * dismissal. Not a failure, and not lied about as one.
   */
  | { kind: "pending"; sessionId: string; paymentIntentId: string }
  /**
   * This package could not decide — a typed {@link VpayError}, never a
   * silent guess. Produced by a pre-flight that failed, a window that never
   * opened, or a poll that itself errored, rather than by the intent
   * reaching any particular status.
   */
  | {
      kind: "unresolved";
      sessionId: string;
      paymentIntentId: string;
      error: VpayError;
    };

/** Options for {@link VpayCheckout}. */
export interface VpayCheckoutOptions {
  /** The API origin, e.g. `https://api.vpay.example`. */
  baseUrl: string;
  publishableKey: string;
  /**
   * D6: demo stack only, never inferred. Without it a non-`https`
   * {@link baseUrl} is refused by the constructor — `loadStripe` itself
   * does not refuse one, so this package does it first.
   */
  allowInsecureBaseUrl?: boolean | undefined;
  /** Injected `fetch`, for tests and for a host that wants its own instrumentation. */
  fetch?: typeof fetch | undefined;
  /** Defaults to {@link defaultHost}: {@link tauriHost} inside Tauri, {@link webHost} otherwise. */
  host?: CheckoutHost | undefined;
  /** Injected clock, for tests. Defaults to {@link systemClock}. */
  clock?: VpayClock | undefined;
}
