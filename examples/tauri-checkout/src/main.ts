/**
 * The whole front end of the Tauri checkout example.
 *
 * One screen, plain DOM, no framework: the point of this example is that the
 * plugin compiles and links inside a real Tauri v2 application on all three
 * platforms, and a framework would only add things that could break instead.
 *
 * Three rules this file keeps, each of them the brief's rather than this
 * example's:
 *
 *   * **D6 — nothing that renders here is a secret.** The result panel shows
 *     `result.kind`, the session id and the payment intent id, and nothing
 *     else. It never renders the session URL (whose fragment *is* the
 *     checkout session's secret), the publishable key, or a thrown value.
 *     There is no `console.*` call in this file either, and `@vpay/config`'s
 *     ESLint config makes that a build failure rather than a convention.
 *   * **D1 — this screen decides nothing about money.** It reports the
 *     `kind` the package resolved and says, in the page footer, that a
 *     merchant fulfils from the webhook.
 *   * **It runs outside Tauri too.** `VpayCheckout` defaults to
 *     `defaultHost()`, which is the Tauri IPC host inside a Tauri app and a
 *     `window.open` popup in a plain browser, so `pnpm dev` in a browser
 *     exercises the same screen against the web host.
 */
import {
  VpayCheckout,
  type VpayCheckoutResult,
} from "@vaam-apps/vpay-tauri-checkout";

/**
 * The three prefill values.
 *
 * `import.meta.env.VITE_*` is compiled in by vite at build time, so these are
 * a developer convenience for `pnpm tauri dev`, not configuration: a built
 * app still lets the payer type all three. They are deliberately *not* a
 * place to put anything secret — a `VITE_` variable is inlined into the
 * bundle, which is why the publishable key is the only key this example ever
 * mentions.
 */
const PREFILL = {
  baseUrl: import.meta.env.VITE_VPAY_BASE_URL ?? "",
  publishableKey: import.meta.env.VITE_VPAY_PUBLISHABLE_KEY ?? "",
  sessionUrl: import.meta.env.VITE_VPAY_SESSION_URL ?? "",
} as const;

/**
 * Looks up one element and narrows it, refusing rather than guessing.
 *
 * `document.querySelector` returns `Element | null` and this file would
 * otherwise be a run of non-null assertions. A missing id is a mistake in
 * `index.html`, and it should say so.
 */
function element<T extends Element>(
  selector: string,
  constructor: new () => T,
): T {
  const found = document.querySelector(selector);
  if (!(found instanceof constructor)) {
    throw new TypeError(`example: ${selector} is not a ${constructor.name}`);
  }
  return found;
}

const form = element("#checkout-form", HTMLFormElement);
const baseUrlInput = element("#base-url", HTMLInputElement);
const publishableKeyInput = element("#publishable-key", HTMLInputElement);
const sessionUrlInput = element("#session-url", HTMLInputElement);
const allowInsecureInput = element("#allow-insecure", HTMLInputElement);
const payButton = element("#pay", HTMLButtonElement);
const dismissButton = element("#dismiss", HTMLButtonElement);
const resultPanel = element("#result", HTMLElement);

baseUrlInput.value = PREFILL.baseUrl;
publishableKeyInput.value = PREFILL.publishableKey;
sessionUrlInput.value = PREFILL.sessionUrl;

/**
 * The checkout in flight, if any — `dismiss()` has to reach the same object
 * `start()` was called on, because the host it holds is the one with the
 * open window.
 */
let inFlight: VpayCheckout | undefined;

/** Writes one line into the result panel. `textContent`, never `innerHTML`. */
function report(text: string): void {
  resultPanel.textContent = text;
}

/**
 * Renders a result.
 *
 * The `switch` is exhaustive over the discriminated union at compile time —
 * adding a sixth `kind` to the package makes this file fail to typecheck,
 * which is the point of writing it out rather than reading `result.kind` and
 * interpolating whatever came back.
 *
 * `failed` renders `code` and `providerMessage`: both are the rail's own
 * words about a failure, carried as data, and neither is a secret. The
 * `unresolved` branch renders the error's `type` and `code` but **not** its
 * `message`, which is where a future error could grow an interpolated value.
 */
function renderResult(result: VpayCheckoutResult): void {
  const ids = `session ${result.sessionId || "(unknown)"} · intent ${
    result.paymentIntentId || "(unknown)"
  }`;
  switch (result.kind) {
    case "succeeded":
      report(`succeeded — ${ids}\n\nFulfil from the webhook, not from this.`);
      return;
    case "failed":
      report(
        `failed — ${ids}\ncode: ${result.code ?? "(none)"}\nprovider: ${
          result.providerMessage ?? "(none)"
        }`,
      );
      return;
    case "canceled":
      report(`canceled — ${ids}`);
      return;
    case "pending":
      report(
        `pending — ${ids}\n\nThe polling budget elapsed with the intent still moving. Not a failure.`,
      );
      return;
    case "unresolved":
      report(
        `unresolved — ${ids}\nerror type: ${result.error.type}\nerror code: ${
          result.error.code ?? "(none)"
        }\n\nNo outcome was observed. Not a failure either.`,
      );
      return;
  }
}

form.addEventListener("submit", (event) => {
  event.preventDefault();
  void pay();
});

dismissButton.addEventListener("click", () => {
  void dismiss();
});

async function pay(): Promise<void> {
  const baseUrl = baseUrlInput.value.trim();
  const publishableKey = publishableKeyInput.value.trim();
  const sessionUrl = sessionUrlInput.value.trim();
  if (baseUrl === "" || publishableKey === "" || sessionUrl === "") {
    report("Fill in the base URL, the publishable key and the session URL.");
    return;
  }

  let checkout: VpayCheckout;
  try {
    checkout = new VpayCheckout({
      baseUrl,
      publishableKey,
      allowInsecureBaseUrl: allowInsecureInput.checked,
    });
  } catch {
    // The constructor throws a `TypeError` on an empty key, an empty base
    // URL, or an `http://` base URL without the opt-in (D6). The thrown
    // value is not interpolated — it is written from fixed strings and
    // could not carry a secret today, but this screen does not depend on
    // that staying true.
    report(
      "Refused before anything opened: the base URL must be https, unless you tick the insecure opt-in.",
    );
    return;
  }

  inFlight = checkout;
  payButton.disabled = true;
  report("Opening the payer's browser…");
  try {
    // `start` never rejects — every failure is a `kind`. The `try` is here
    // for the contract's sake, not because a rejection is expected.
    const result = await checkout.start(sessionUrl);
    renderResult(result);
  } catch {
    report("unresolved — the checkout threw, which it is not supposed to.");
  } finally {
    payButton.disabled = false;
    if (inFlight === checkout) {
      inFlight = undefined;
    }
  }
}

async function dismiss(): Promise<void> {
  if (inFlight === undefined) {
    report("Nothing open to dismiss.");
    return;
  }
  await inFlight.dismiss();
  // Deliberately no "dismissed" message: `dismiss()` reports nothing on its
  // own. The close is observed as an ordinary dismissal, which then polls
  // (D4) exactly as a payer's own close does, and `pay()`'s pending `await`
  // is what renders the answer.
}
