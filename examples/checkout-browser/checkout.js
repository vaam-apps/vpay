// A plain-JS payer page against `@vpay/stripe-js`. No bundler: this file is
// served as-is and imports the vendored package build by relative path.
//
// `./dist/stripe-js/index.js` is NOT checked in — it is `sdks/stripe-js/dist/`
// copied here by `just build-checkout-browser` (see the justfile and this
// example's README). `dist/` is gitignored repo-wide, which is why the
// vendored copy lives under that name rather than a bespoke one.
import { loadStripe } from "./dist/stripe-js/index.js";

const params = new URLSearchParams(window.location.search);
const publishableKey = params.get("pk");
const clientSecret = params.get("client_secret");
// Not part of Stripe.js's contract (Stripe's origin is hardcoded), but vpay
// is not `js.stripe.com` — this page has to be told where `/v1/browser`
// lives. Defaults to the compose stack's published port so the README's
// steps work with no query parameter beyond `pk`/`client_secret` most of the
// time; override with `&api=http://localhost:18080` when `just
// demo_port=18080 demo` moved it.
const apiBaseUrl = params.get("api") ?? "http://localhost:8080";

const el = {
  error: document.getElementById("error"),
  summary: document.getElementById("summary"),
  intentId: document.getElementById("intent-id"),
  amount: document.getElementById("amount"),
  status: document.getElementById("status"),
  form: document.getElementById("confirm-form"),
  msisdn: document.getElementById("msisdn"),
  button: document.getElementById("confirm-button"),
  waiting: document.getElementById("waiting"),
};

function showError(err) {
  const code = err.code ? `/${err.code}` : "";
  el.error.hidden = false;
  el.error.textContent = `${err.type}${code}: ${err.message ?? "no message"}`;
}

function clearError() {
  el.error.hidden = true;
  el.error.textContent = "";
}

function renderIntent(paymentIntent) {
  el.summary.hidden = false;
  el.intentId.textContent = paymentIntent.id;
  el.amount.textContent = `${paymentIntent.amount} ${paymentIntent.currency}`;
  el.status.textContent = paymentIntent.status;
  // What `frontends/tests/e2e/cypress/e2e/checkout.cy.ts` asserts on —
  // mirrors `data-status` on `frontends/apps/dashboard`'s own scaffold
  // (`cypress/e2e/dashboard.cy.ts`), so both specs read status the same way.
  el.status.dataset.status = paymentIntent.status;
}

async function main() {
  if (!publishableKey || !clientSecret) {
    showError({
      type: "integration_error",
      message:
        "missing ?pk=...&client_secret=... in this page's URL — see README.md.",
    });
    return;
  }

  let stripe;
  try {
    stripe = await loadStripe(publishableKey, { baseUrl: apiBaseUrl });
  } catch (err) {
    showError({
      type: "integration_error",
      message: err instanceof Error ? err.message : String(err),
    });
    return;
  }

  const initial = await stripe.retrievePaymentIntent(clientSecret);
  if (initial.error) {
    showError(initial.error);
    return;
  }
  renderIntent(initial.paymentIntent);

  if (initial.paymentIntent.status !== "requires_payment_method") {
    // Already confirmed on an earlier visit, or terminal — nothing left for
    // this page to collect. Reloading a succeeded checkout should not offer
    // to charge the payer again.
    return;
  }

  el.form.hidden = false;
  el.form.addEventListener("submit", async (event) => {
    event.preventDefault();
    clearError();
    el.button.disabled = true;

    const msisdn = el.msisdn.value.trim();
    const confirmed = await stripe.confirmMobileMoneyPayment(clientSecret, {
      type: "mtn_momo",
      msisdn,
    });

    if (confirmed.error) {
      showError(confirmed.error);
      el.button.disabled = false;
      return;
    }

    renderIntent(confirmed.paymentIntent);
    el.form.hidden = true;
    el.waiting.hidden = false;

    // The worker settles the intent asynchronously (docs/flows/crash-safety.md);
    // this is how a merchant page without its own websocket/webhook listener
    // learns the outcome.
    const settled = await stripe.waitForPaymentIntent(clientSecret);
    el.waiting.hidden = true;

    if (settled.error) {
      showError(settled.error);
      return;
    }
    renderIntent(settled.paymentIntent);
  });
}

main();
