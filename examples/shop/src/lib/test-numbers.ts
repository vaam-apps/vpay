/**
 * The fake mobile-money numbers the **demo stack** honours, and what each one
 * makes happen.
 *
 * None of these is a phone number. They are documentation MSISDNs from the
 * `2376000000xx` block, and they mean something only because
 * `demo-outcomes.json` under
 * `backends/tests/conformance/wiremock/mtn` and `.../orange` — the
 * WireMock hosts that stand in for MTN and Orange in `compose.yml` — are
 * configured to answer particular things for them. **There is no branch on
 * any of these values in vpay, in this shop, or in either adapter**: the
 * steering lives in the stub's configuration, exactly where AGENTS.md's
 * first rule puts it, and against a real rail these numbers do nothing at
 * all.
 *
 * The two rails do not offer the same outcomes, and that is the single most
 * useful thing this table shows a merchant. MTN's `FAILED` bodies carry a
 * `reason` from a nine-row vocabulary; Orange documents five statuses and no
 * sub-reason for `FAILED` at all
 * ([adapter-mtn-momo.md](../../../../docs/flows/adapter-mtn-momo.md),
 * [adapter-orange-money.md](../../../../docs/flows/adapter-orange-money.md)).
 * So `insufficient_funds` is reachable on MTN and not on Orange, and the
 * honest thing to do is say so rather than invent an Orange status.
 *
 * `test-numbers.test.ts` checks this table against
 * `examples/shop/README.md`, in both directions, so the panel a buyer sees
 * and the table a developer reads cannot drift apart.
 */
import type { FailureCode } from "./failures";

/** The rails this shop knows how to describe. */
export type RailCode = "mtn_momo" | "orange_money";

export interface TestNumber {
  /** The canonical `2376XXXXXXXX` a payer types. Digits only — vpay's checkout page refuses letters. */
  msisdn: string;
  /** The same value grouped the way a Cameroonian reads it. Display only. */
  display: string;
  /** What the payer ends up seeing, in one phrase. */
  outcome: string;
  /** What this shop's order becomes once the webhook lands. */
  orderStatus: "paid" | "failed";
  /** vpay's `last_payment_error.code`, or `null` when the charge settles. */
  failureCode: FailureCode | null;
  /** The rail's own word for it, so this table can be diffed against the adapter docs. */
  railReason: string;
}

export interface RailTestNumbers {
  rail: RailCode;
  /** The rail's name, as a payer knows it. */
  label: string;
  /** Where the payer types the number — the two rails differ, because their flows do. */
  where: string;
  numbers: TestNumber[];
  /** Outcomes this rail's documented vocabulary cannot express, and why. */
  cannotExpress: { outcome: string; why: string }[];
}

/**
 * Ordered rail first, then by the number, so the rendered panel and the
 * README table are in the same order and a diff of either is readable.
 */
export const TEST_NUMBERS: readonly RailTestNumbers[] = [
  {
    rail: "mtn_momo",
    label: "MTN MoMo",
    where:
      "on vpay's checkout page, in the mobile-money field — MTN is a push rail, so vpay prompts the handset",
    numbers: [
      {
        msisdn: "237600000000",
        display: "+237 6 00 00 00 00",
        outcome: "Pays. Any number not listed below does the same.",
        orderStatus: "paid",
        failureCode: null,
        railReason: "SUCCESSFUL",
      },
      {
        msisdn: "237600000101",
        display: "+237 6 00 00 01 01",
        outcome: "Declined — the wallet has too little money",
        orderStatus: "failed",
        failureCode: "insufficient_funds",
        railReason: "NOT_ENOUGH_FUNDS",
      },
      {
        msisdn: "237600000102",
        display: "+237 6 00 00 01 02",
        outcome: "The prompt expires — nobody enters the PIN",
        orderStatus: "failed",
        failureCode: "payer_timeout",
        railReason: "COULD_NOT_PERFORM_TRANSACTION",
      },
      {
        msisdn: "237600000400",
        display: "+237 6 00 00 04 00",
        outcome: "Refused at submit — the rail has no such account",
        orderStatus: "failed",
        failureCode: "invalid_payer",
        railReason: "PAYER_NOT_FOUND (HTTP 400)",
      },
      {
        msisdn: "237600000503",
        display: "+237 6 00 00 05 03",
        outcome: "The rail is unavailable",
        orderStatus: "failed",
        failureCode: "provider_unavailable",
        railReason: "SERVICE_UNAVAILABLE",
      },
    ],
    cannotExpress: [
      {
        outcome: "payer_declined",
        why: "MTN documents no reason for a payer who answered the prompt and refused it — its nine-row table has none, so no MSISDN can produce one. `FailureCode::PayerDeclined` is currently produced by no adapter in this repository.",
      },
    ],
  },
  {
    rail: "orange_money",
    label: "Orange Money",
    where:
      "on the rail's own payment page, after vpay redirects you — Orange is a redirect rail, so vpay never sees the number",
    numbers: [
      {
        msisdn: "237600000000",
        display: "+237 6 00 00 00 00",
        outcome: "Pays. Any number not listed below does the same.",
        orderStatus: "paid",
        failureCode: null,
        railReason: "SUCCESS",
      },
      {
        msisdn: "237600000102",
        display: "+237 6 00 00 01 02",
        outcome: "The payment window expires",
        orderStatus: "failed",
        failureCode: "payer_timeout",
        railReason: "EXPIRED",
      },
      {
        msisdn: "237600000400",
        display: "+237 6 00 00 04 00",
        outcome: "Refused, with no reason the rail will name",
        orderStatus: "failed",
        failureCode: "provider_error",
        railReason: "FAILED",
      },
    ],
    cannotExpress: [
      {
        outcome: "insufficient_funds",
        why: "Orange's documented statuses are INITIATED, PENDING, SUCCESS, EXPIRED and FAILED, and it documents no sub-reason for FAILED. A stub answering `NOT_ENOUGH_FUNDS` here would be this repository inventing a rail vocabulary.",
      },
      {
        outcome: "invalid_payer",
        why: "The number never reaches vpay on a redirect rail: the payer types it on Orange's own page, after the charge has already been submitted.",
      },
      {
        outcome: "provider_unavailable",
        why: "A rail that cannot answer is a transport failure on Orange, not a status — the poll ladder retries it for hours rather than failing the charge, which is right and is not something a demo can show in a minute.",
      },
    ],
  },
];

/** The rails in {@link TEST_NUMBERS} that this deployment actually offers. */
export function testNumbersFor(
  rails: readonly string[],
): readonly RailTestNumbers[] {
  return TEST_NUMBERS.filter((entry) => rails.includes(entry.rail));
}
