/**
 * The payment-intent lifecycle, driven entirely by the **official `stripe`
 * package** against a **real vpay stack**.
 *
 * Nothing here stubs anything. The requests leave this process over TCP,
 * traverse vpay's real router, real Postgres and (on confirm) a real HTTP
 * call to the MTN WireMock host `compose.yml` configures — the same
 * mechanism a production rail is reached by (ADR-0006). If the stack is not
 * there, `src/preflight.ts` fails the run before this file is loaded.
 */
import Stripe from "stripe";
import { describe, expect, it } from "vitest";

import {
  AMOUNT,
  CURRENCY,
  RAIL,
  confirmIntent,
  createIntent,
  stripeClient,
} from "./client.js";

const stripe = stripeClient();

/**
 * How long the settlement case waits, and how often it asks.
 *
 * A ceiling, not an expectation, and the spread it has to cover is wide.
 * `confirm` enqueues the `poll_charge` job in the charge's own transaction
 * with `run_at = now()`, so the **first** poll is immediate — the worker's
 * idle loop picks it up within `vpay_worker::run_loop::IDLE_SLEEP` (1 s), and
 * `vpay_worker::poll_delay`'s 10 s / 20 s rungs govern only the *re*-polls
 * after a `PENDING`. Against the compose MTN stub, a status query for a
 * reference with no specific mapping answers `SUCCESSFUL` outright, so the
 * usual settlement is a second or two. It is ~10 s longer when the
 * `mtn-e2e-poll` scenario has been entered — `just demo` confirms with the
 * one MSISDN that enters it, and it then answers `PENDING` once — and this
 * suite may well be run on a stack a demo has already used.
 *
 * 120 s is generous enough that a cold stack, or one whose worker is still
 * applying migrations, does not fail the run, and tight enough that a worker
 * which is not running fails it in two minutes rather than sitting there.
 *
 * The per-case timeout is set from this value rather than left to
 * `vitest.config.ts`'s 30 s default, because a case that exceeds a *vitest*
 * timeout reports "test timed out" and says nothing about the worker.
 */
const SETTLE_WINDOW_MS = 120_000;
const SETTLE_POLL_MS = 2_000;

describe("payment intent lifecycle through stripe-node", () => {
  it("creates and retrieves an intent, and both carry a request id", async () => {
    const created = await createIntent(stripe, {
      metadata: { suite: "stripe-compat", case: "create-retrieve" },
      description: "created by @vpay/stripe-compat",
    });

    expect(created.object).toBe("payment_intent");
    expect(created.id).toMatch(/^pi_/);
    expect(created.status).toBe("requires_payment_method");
    expect(created.amount).toBe(AMOUNT);
    expect(created.currency).toBe(CURRENCY);
    expect(created.payment_method_types).toEqual([RAIL]);
    expect(created.metadata).toMatchObject({ suite: "stripe-compat" });
    expect(created.livemode).toBe(false);

    // `lastResponse.requestId` is populated only from a `request-id` response
    // header — stripe-node never reads `x-request-id`. This assertion is the
    // whole reason the server mirrors the id under a second name.
    expect(created.lastResponse.requestId).toMatch(
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/,
    );
    expect(created.lastResponse.statusCode).toBe(200);

    const retrieved = await stripe.paymentIntents.retrieve(created.id);
    expect(retrieved.id).toBe(created.id);
    expect(retrieved.status).toBe("requires_payment_method");
    expect(retrieved.amount).toBe(AMOUNT);
    // A different request, so a different id — the header is per-request, not
    // per-object.
    expect(retrieved.lastResponse.requestId).not.toBe(
      created.lastResponse.requestId,
    );
  });

  it("pages the list with a cursor, including stripe-node's own auto-pagination", async () => {
    const first = await createIntent(stripe, {
      metadata: { case: "page-1" },
    });
    const second = await createIntent(stripe, {
      metadata: { case: "page-2" },
    });
    const third = await createIntent(stripe, { metadata: { case: "page-3" } });

    // Newest first, and `has_more` answered by asking for one row more than
    // the limit rather than by comparing lengths.
    const page = await stripe.paymentIntents.list({ limit: 2 });
    expect(page.object).toBe("list");
    expect(page.data.map((intent) => intent.id)).toEqual([third.id, second.id]);
    expect(page.has_more).toBe(true);
    expect(page.url).toBe("/v1/payment_intents");

    const next = await stripe.paymentIntents.list({
      limit: 2,
      starting_after: second.id,
    });
    expect(next.data[0]?.id).toBe(first.id);

    // stripe-node's own cursor walker, which needs only `data[].id` and
    // `has_more` from the envelope. Two round trips at `limit: 2`.
    const walked = await stripe.paymentIntents
      .list({ limit: 2 })
      .autoPagingToArray({ limit: 3 });
    expect(walked.map((intent) => intent.id)).toEqual([
      third.id,
      second.id,
      first.id,
    ]);
  });

  it("cancels an unconfirmed intent", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "cancel" },
    });
    const canceled = await stripe.paymentIntents.cancel(created.id);

    expect(canceled.id).toBe(created.id);
    expect(canceled.status).toBe("canceled");

    const retrieved = await stripe.paymentIntents.retrieve(created.id);
    expect(retrieved.status).toBe("canceled");
  });

  it("confirms against the push rail and the intent moves to processing", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "confirm" },
    });
    const confirmed = await confirmIntent(stripe, created.id);

    // `processing` is a push rail's one success state: the rail has the
    // request and the payer has a prompt on their handset. There is no
    // `next_action` — that is a redirect rail's shape.
    expect(confirmed.status).toBe("processing");
    expect(confirmed.next_action).toBeNull();
    expect(confirmed.last_payment_error).toBeNull();

    // The confirm's response and a fresh read must be the same object, so a
    // status that was rendered but never committed fails here.
    //
    // `processing` OR `succeeded`, and the alternative is not slack: the
    // `poll_charge` job is enqueued with `run_at = now()`, so the worker may
    // legitimately have settled this charge in the milliseconds between the
    // confirm returning and this read. What both answers rule out is the
    // thing this assertion exists for — a `requires_payment_method` here
    // would mean the confirm rendered a status it never committed. Pinning
    // `processing` alone would make the case fail on a machine where the
    // worker won that race, which is a property of scheduling, not of vpay.
    const retrieved = await stripe.paymentIntents.retrieve(created.id);
    expect(["processing", "succeeded"]).toContain(retrieved.status);
    expect(retrieved.amount).toBe(confirmed.amount);
  });

  /**
   * **The assertion that inverted.**
   *
   * It used to read "stays in processing, because nothing on this branch
   * polls the rail", and it was correct: before the worker (Step 4) a
   * confirmed intent sat in `processing` forever and a suite that polled
   * `retrieve` "until succeeded" would have hung rather than passed.
   *
   * What settles it now is the `vpay-worker` container the stack brings up.
   * It claims the `poll_charge` job the confirm committed in the charge's own
   * transaction, asks the MTN WireMock host over HTTP, and settles the charge
   * when the rail answers `SUCCESSFUL`; a `PENDING` sends it round
   * `vpay_worker::poll_delay`'s ladder instead. Nothing in this file makes
   * that happen — `retrieve` is all a merchant integration can see and all
   * this case uses.
   *
   * **Bounded, and it fails rather than hangs.** {@link SETTLE_WINDOW_MS} is
   * a ceiling many times the usual settlement, not an expectation, and a
   * window that closes on `processing` is an assertion failure naming the
   * worker — the single most likely cause. A status that is neither
   * `processing` nor `succeeded` fails immediately instead of being polled
   * past: `canceled` and a decline (`requires_payment_method` with
   * `last_payment_error`) are both real, terminal answers about this payment.
   *
   * The rail is still a WireMock host. What is proven is that vpay settles a
   * confirmed intent and that a Stripe SDK sees it happen — not that MTN
   * approves anything.
   */
  it(
    "settles to succeeded, because the worker polls the rail",
    async () => {
      const created = await createIntent(stripe, {
        metadata: { case: "settles" },
      });
      const confirmed = await confirmIntent(stripe, created.id);
      expect(confirmed.status).toBe("processing");

      const deadline = Date.now() + SETTLE_WINDOW_MS;
      let last = confirmed.status;
      let polls = 0;
      while (Date.now() < deadline && last === "processing") {
        await new Promise((resolve) => setTimeout(resolve, SETTLE_POLL_MS));
        const intent = await stripe.paymentIntents.retrieve(created.id);
        polls += 1;
        last = intent.status;
      }

      expect(
        last,
        `after ${polls} polls over ${SETTLE_WINDOW_MS / 1000}s. "processing" ` +
          `here means nothing settled the charge — check that the vpay-worker ` +
          `container is running (\`docker compose logs vpay-worker\`).`,
      ).toBe("succeeded");
    },
    SETTLE_WINDOW_MS + 15_000,
  );

  /**
   * `expand` is **accepted and ignored**, and this is the case behind that
   * claim in `docs/flows/stripe-sdk-compat.md`.
   *
   * The interesting half is the encoding, not the ignoring. stripe-node
   * serialises an array **indexed** — `expand[0]=latest_charge`, never
   * `expand[]=latest_charge` — and `vpay_api::form` has to rebuild that into
   * an array for the request to decode at all. If it did not, the body would
   * be rejected outright rather than quietly dropping one unknown field, and
   * every `expand`-carrying request a Stripe integration already sends would
   * 400.
   *
   * The `expand[]` spelling `vpay_api::form` also accepts is **not** covered
   * here: stripe-node never emits it. Its evidence is that decoder's own unit
   * test, and the flow doc says so rather than implying this suite saw it.
   */
  it("accepts and ignores `expand`, which stripe-node encodes as `expand[0]`", async () => {
    const created = await createIntent(stripe, {
      metadata: { case: "expand" },
      expand: ["latest_charge"],
    });

    expect(created.id).toMatch(/^pi_/);
    expect(created.status).toBe("requires_payment_method");
    // Nothing was expanded, and the absence is visible in the response
    // itself — which is why this one is ignored rather than refused.
    expect(created.latest_charge).toBeUndefined();
  });

  it("accepts a Stripe client built exactly as the docs tell merchants to build one", () => {
    // Not a network assertion — a construction one. `new Stripe("", {...})`
    // with an authenticator and no key is the supported combination, and
    // stripe-node throws at construction for both of the other two.
    expect(stripe).toBeInstanceOf(Stripe);
    expect(() => new Stripe("")).toThrow();
  });
});
