/**
 * The refund surface against a **real** `vpay-server`, over a socket.
 *
 * # Why this file exists at all
 *
 * `client.test.ts` proves what this package *sends*, against
 * `src/testing/test-server.ts`, which answers whatever the case told it to.
 * That is not evidence that the server accepts it, and this repository has
 * been burned by exactly that twice: `cargo xtask verify-sdk-parity` proves a
 * **name** exists and not that the SDK sends the field (issue #122), and a
 * fixture once invented a `client_secret` the server never sends — unit
 * tests, review, mutation testing and CI all passed while the client was
 * broken against real HTTP.
 *
 * The refund surface is the one where that would cost the most. Its
 * `destination` is a rail-agnostic envelope around a rail-specific interior
 * (`destination[<payment_method_type>][msisdn]`), the server strips the outer
 * key and hands the interior to that rail's own adapter, and the MSISDN rule
 * it applies there is **not** the rule `GET /v1/account_holders` applies: a
 * payee must be international, starting with `+`.
 *
 * `sdks/rust/tests/live_refunds.rs` is this file's twin and asserts the same
 * sequence through the other SDK.
 *
 * # What it does NOT prove
 *
 * **That any money moved, or that MTN refunds work.** The rail here is a
 * `wiremock/wiremock` container answering `POST /disbursement/v1_0/transfer`
 * with the `202` this repository transcribed from MTN's documentation. MTN's
 * Disbursements product has never been called from this repository. And
 * **nothing settles a `pending` refund** — there is no refund poll ladder
 * (RFC-0003 open question 8) — which is why the lifecycle case asserts
 * `"pending"` after a create and would fail the day something started
 * claiming otherwise without one.
 *
 * # It never skips
 *
 * `pnpm test` does not collect this file — `vitest.config.ts` excludes the
 * glob and `vitest.live.config.ts` is the only thing that includes it — and
 * `pnpm test:live` fails, rather than skipping, when there is no stack.
 * There is no `it.skip` and no `if (!process.env…) return` below.
 *
 * # Running it
 *
 * ```text
 * just sdk-live
 * ```
 */
import { inspect } from "node:util";
import { beforeAll, describe, expect, it } from "vitest";

import { VpayClient } from "./client.js";
import { VpayApiError } from "./errors.js";
import { readLiveEnv } from "./testing/live-env.js";

/**
 * The payer the demo MTN stub settles for —
 * `wiremock/mtn/mappings/requesttopay-scenario.json`'s `mtn-e2e-poll` walk,
 * `PENDING` then `SUCCESSFUL`.
 *
 * **Moved off `237600000100` on 2026-09-17.** Unlike the payee constants
 * below, this one is the **confirm**, and issue #186 put real phone
 * validation on that path: the old number earned a `400` —
 * "`payment_method_data[mtn_momo][msisdn]` must be a valid phone number" —
 * before the rail was asked. `60` is not a Cameroon mobile prefix, so
 * nothing in the `2376000000xx` block validates. `237670000900` does, and is
 * already in that scenario's own `payer.partyId` matcher.
 */
const PAYER_MSISDN = "237670000900";

/**
 * The payee, **in the form the refund path requires** — international, with a
 * `+`. The stub answers a named holder for its digits, which is what the
 * handler's account-holder check asks for.
 */
const PAYEE_MSISDN = "+237600000200";

/** The payee the rail has no record of: `basicuserinfo` answers `404`. */
const UNKNOWN_PAYEE_MSISDN = "+237600000404";

/** A ceiling many times the usual settlement, not an expectation. */
const SETTLE_WINDOW_MS = 60_000;

let client: VpayClient;

beforeAll(() => {
  const env = readLiveEnv();
  client = new VpayClient({
    baseUrl: env.baseUrl,
    clientId: env.clientId,
    privateKey: env.privateKey,
  });
});

/**
 * The refusal a call produced, or a failure naming what came back instead —
 * so a case that expected a refusal and got a refund says so.
 */
async function refusalOf(
  call: Promise<unknown>,
  what: string,
): Promise<VpayApiError> {
  let caught: unknown;
  try {
    await call;
  } catch (error) {
    caught = error;
  }
  if (!(caught instanceof VpayApiError)) {
    throw new Error(
      `${what}: expected a VpayApiError, got ${inspect(caught ?? "a success")}`,
    );
  }
  // The rule the server applies must never reach a merchant with the number
  // in it: the payee is a third party. Asserted on the real body, because
  // this is the only place where the real message and the real number are
  // both in hand.
  expect(caught.message, `${what} echoed the payee's number`).not.toContain(
    "600000200",
  );
  expect(caught.message, `${what} echoed the payee's number`).not.toContain(
    "600000404",
  );
  return caught;
}

/**
 * An intent charged on MTN and settled by the worker, which is what a refund
 * needs: `POST /v1/refunds` reads the rail off the **charge**, and an intent
 * with no charge is a `409` before any destination is looked at.
 *
 * Bounded, and it FAILS rather than hangs. A status that is neither
 * `processing` nor `succeeded` fails immediately instead of being polled
 * past: a decline is a real, terminal answer about this payment.
 */
async function aSettledIntent(amount: number): Promise<string> {
  const created = await client.paymentIntents.create({
    amount,
    currency: "XAF",
    payment_method_types: ["mtn_momo"],
  });
  const confirmed = await client.paymentIntents.confirm(created.id, {
    payment_method_data: {
      type: "mtn_momo",
      mtn_momo: { msisdn: PAYER_MSISDN },
    },
  });
  expect(["processing", "succeeded"]).toContain(confirmed.status);

  const deadline = Date.now() + SETTLE_WINDOW_MS;
  for (;;) {
    const read = await client.paymentIntents.retrieve(created.id);
    if (read.status === "succeeded") return created.id;
    expect(
      read.status,
      "the intent reached a terminal status that is not a charge",
    ).toBe("processing");
    if (Date.now() >= deadline) {
      throw new Error(
        `the intent was still processing after ${SETTLE_WINDOW_MS} ms: the vpay-worker ` +
          `container is the most likely cause`,
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}

describe("refunds against a running vpay", () => {
  /**
   * **The MSISDN rule, against the real server.**
   *
   * The bare national `600000200` is refused — even though
   * `GET /v1/account_holders` accepts it, because that route is
   * Cameroon-specific and `vpay-provider` is not: read as an international
   * number, `600000200` begins with country code `6` and names a payee in
   * Malaysia.
   *
   * This is the case a fixture cannot write honestly. An SDK that normalised
   * the number, or that flattened the envelope, passes every case in
   * `client.test.ts` and lands here.
   */
  it("refuses a refund with no payee, a payee with no +, and a payee the rail has never seen", async () => {
    const intent = await aSettledIntent(5_000);

    // 1. No destination at all — the regression Arm F left and this closes:
    //    it was an honest `404` before the route was mounted.
    const noDestination = await refusalOf(
      client.refunds.create({ payment_intent: intent }),
      "no destination",
    );
    expect(noDestination.status).toBe(400);
    expect(noDestination.param).toBe("destination");

    // 2. A number with no `+`. It reaches the adapter's own parser — which is
    //    the half that proves the envelope was read — and is refused there.
    const noPlus = await refusalOf(
      client.refunds.create({
        payment_intent: intent,
        destination: {
          kind: "mobile_money",
          payment_method_type: "mtn_momo",
          msisdn: "600000200",
        },
      }),
      "no leading +",
    );
    expect(noPlus.status).toBe(400);
    expect(noPlus.param).toBe("destination");

    // 3. A well-formed number the rail has no holder for. Past the parser,
    //    refused by the account-holder lookup — so this one proves the
    //    interior really was handed to `mtn_momo`'s adapter.
    const unknown = await refusalOf(
      client.refunds.create({
        payment_intent: intent,
        destination: {
          kind: "mobile_money",
          payment_method_type: "mtn_momo",
          msisdn: UNKNOWN_PAYEE_MSISDN,
        },
      }),
      "unregistered payee",
    );
    expect(unknown.status).toBe(400);
    expect(unknown.param).toBe("destination");

    // None of the three wrote a refund. A refusal that cost a row would show
    // up here, and the merchant's next full refund would be short by its
    // amount.
    const page = await client.refunds.list({ payment_intent: intent });
    expect(page.data).toHaveLength(0);
  });

  /**
   * **create → retrieve → update → list → refused cancel**, against a running
   * vpay.
   *
   * One case for the sequence rather than five, because the states are
   * sequential: a refund must exist to be read, and what the cancel proves is
   * about the refund the four steps before it built.
   *
   * What it pins that the stub server could not: the
   * `destination[<rail>][msisdn]` envelope this package writes is one the
   * real server strips, hands to the real `mtn_momo` adapter and accepts; a
   * created refund is **`pending`**, because nothing settles one; the update
   * really merges metadata key-wise and an empty value really deletes a key;
   * and a refund the rail has already been given is not cancelable, with its
   * reservation still held after the refusal.
   *
   * The cancel that *does* fire serves the crashed-create state, which a
   * client driving a healthy server over HTTP cannot produce;
   * `backends/tests/integration/tests/refunds.rs` stages it directly.
   */
  it("creates a refund with a registered payee, reads, patches, lists it, and cannot cancel it", async () => {
    const intent = await aSettledIntent(5_000);

    const refund = await client.refunds.create({
      payment_intent: intent,
      amount: 2_000,
      reason: "requested_by_customer",
      destination: {
        kind: "mobile_money",
        payment_method_type: "mtn_momo",
        msisdn: PAYEE_MSISDN,
      },
      metadata: { order_id: "w3sdks-node", stale: "delete-me" },
    });

    expect(refund.id.startsWith("re_")).toBe(true);
    expect(refund.object).toBe("refund");
    expect(refund.payment_intent).toBe(intent);
    expect(refund.amount).toBe(2_000);
    expect(refund.currency).toBe("xaf");
    expect(refund.reason).toBe("requested_by_customer");
    // **The claim this file must not let drift.** Nothing settles a refund,
    // so an accepted instruction leaves it pending; this fails the day
    // something starts writing `succeeded` without a poll ladder.
    expect(refund.status).toBe("pending");
    // No rail reports a refund fee and nothing here stores one. `toBeNull`
    // and not `toBeFalsy`: an absent key would read `undefined` and mean
    // something else.
    expect(refund.fee).toBeNull();

    // The destination is on no response — it is a third party's personal
    // data, it is in no column, and RFC-0003 rejected carrying it in
    // `metadata` because metadata is inside every signed webhook.
    expect(JSON.stringify(refund)).not.toContain("600000200");

    const read = await client.refunds.retrieve(refund.id);
    expect(read.id).toBe(refund.id);
    expect(read.status).toBe("pending");
    expect(read.metadata["order_id"]).toBe("w3sdks-node");

    const updated = await client.refunds.update(refund.id, {
      metadata: {
        order_id: "w3sdks-node-updated",
        added: "yes",
        // Stripe's per-key delete.
        stale: "",
      },
    });
    expect(updated.metadata["order_id"]).toBe("w3sdks-node-updated");
    expect(updated.metadata["added"]).toBe("yes");
    expect(Object.keys(updated.metadata)).not.toContain("stale");

    // A negative amount is refused one layer earlier still — before any
    // request leaves this process, as a TypeError from `validate.ts`.
    await expect(
      client.refunds.create({ payment_intent: intent, amount: -1 }),
    ).rejects.toBeInstanceOf(TypeError);

    const page = await client.refunds.list({ payment_intent: intent });
    expect(page.data.map((item) => item.id)).toContain(refund.id);
    expect(page.url).toBe("/v1/refunds");

    // **The rail already has this instruction, so it cannot be cancelled.**
    // `POST /v1/refunds` writes the `provider_requests` row before it sends
    // the transfer, and `vpay_db::refunds::cancel_in_tx`'s `NOT EXISTS` reads
    // that row as "a rail may already be moving this money". A cancel would
    // write `canceled` — a promise that no money will move — and hand the
    // reservation back: measured before the guard existed as two 5 000
    // transfers on WireMock's journal against one 5 000 charge (commit
    // 4bcf6491).
    //
    // This expected a `canceled` refund until 2026-09-16, as its Rust twin
    // did. Both were written against the contract as it stood and neither was
    // re-run against a server carrying the guard.
    const refusal = await refusalOf(
      client.refunds.cancel(refund.id),
      "cancel after the rail was instructed",
    );
    expect(refusal.status).toBe(409);

    // And the refusal cost nothing. A cancel that failed *after* releasing
    // the reservation would be the same double refund with an error code on
    // it, so both halves are asserted.
    const afterRefusal = await client.refunds.retrieve(refund.id);
    expect(afterRefusal.status).toBe("pending");

    // The reservation, read the only way a merchant can: through what is left
    // to refund. 2 000 of the 5 000 is spoken for, so a refund naming no
    // `amount` is the remaining 3 000 — a 5 000 here would be the over-refund
    // the reservation exists to prevent.
    const rest = await client.refunds.create({
      payment_intent: intent,
      destination: {
        kind: "mobile_money",
        payment_method_type: "mtn_momo",
        msisdn: PAYEE_MSISDN,
      },
    });
    expect(rest.amount).toBe(3_000);

    // Nothing can be cancelled back, so the intent is now fully spoken for.
    const overRefund = await refusalOf(
      client.refunds.create({
        payment_intent: intent,
        amount: 1,
        destination: {
          kind: "mobile_money",
          payment_method_type: "mtn_momo",
          msisdn: PAYEE_MSISDN,
        },
      }),
      "over-refund",
    );
    expect(overRefund.status).toBe(409);
  });
});
