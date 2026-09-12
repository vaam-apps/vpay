import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { PAYMENT_STATUS_SYSTEM } from "../payment-status";
import { INTENT, OTHER_INTENT } from "../testing/fixtures";
import { PaymentsTable } from "./payments-table";

describe("the payments table", () => {
  it("renders one row per payment, with the id linking to its detail page", () => {
    render(<PaymentsTable rows={[INTENT, OTHER_INTENT]} />);
    expect(screen.getAllByRole("row")).toHaveLength(3); // header + two
    expect(screen.getByRole("link", { name: "pi_example_1" })).toHaveAttribute(
      "href",
      "/payments/pi_example_1",
    );
  });

  it("renders the amount in minor units as its currency spells it", () => {
    render(<PaymentsTable rows={[INTENT]} />);
    expect(screen.getByText("5,000 XAF")).toBeInTheDocument();
  });

  it("takes the status pill presentation from one shared table, never a local map", () => {
    // The mechanism changed with the cutover (`.badge-success`/`.badge-info`
    // died with `StatusBadge`), but the guarantee did not: a status's
    // presentation comes from `PAYMENT_STATUS_SYSTEM`, not from anything
    // local to this file. Read the expected names OFF the table rather than
    // typing them as literals, so a change to the table is caught here too
    // and a hard-coded string in the component is caught as a mismatch.
    // `INTENT` is "succeeded", `OTHER_INTENT` is "processing".
    render(<PaymentsTable rows={[INTENT, OTHER_INTENT]} />);
    const pills = screen.getAllByRole("img");
    expect(pills).toHaveLength(2);
    expect(pills[0]).toHaveAccessibleName(
      `succeeded — ${PAYMENT_STATUS_SYSTEM.succeeded.label}`,
    );
    expect(pills[1]).toHaveAccessibleName(
      `processing — ${PAYMENT_STATUS_SYSTEM.processing.label}`,
    );
    // Mutation note: point `processing`'s row at `succeeded`'s meta in
    // `payment-status.ts` and this assertion must fail — it currently does.
  });

  it("renders a status this build cannot name as text, never as a coloured pill", () => {
    render(<PaymentsTable rows={[{ ...INTENT, status: "disputed" }]} />);
    expect(screen.getByText("disputed")).toBeInTheDocument();
    expect(screen.queryAllByRole("img")).toHaveLength(0);
  });

  it('heads the rail column "Methods", because that is what the list carries', () => {
    // `payment_method_types` is the set of rails an intent MAY be confirmed
    // against; the rail that actually took it is `charge.provider_code`, and
    // GET /dash/v1/payment_intents returns no charge at all. A column headed
    // "Rail" filled from this would be wrong for every intent that offers
    // two and was taken by one.
    render(<PaymentsTable rows={[INTENT]} />);
    expect(
      screen.getByRole("columnheader", { name: "Methods" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("columnheader", { name: "Rail" })).toBeNull();
    expect(screen.getByText("mtn_momo, orange_money")).toBeInTheDocument();
  });

  it("has no payer column — the list response carries no payer field at all", () => {
    // `charges.payer_ref_masked` is never written (docs/status.md) AND is
    // absent from this response. The detail page renders it from the column
    // as a dash; a column here would be sourced from nothing.
    render(<PaymentsTable rows={[INTENT]} />);
    expect(screen.queryByRole("columnheader", { name: /payer/i })).toBeNull();
  });

  it("renders nothing but a header for an empty result set", () => {
    // No placeholder rows, ever. The page renders EmptyState instead.
    render(<PaymentsTable rows={[]} />);
    const table = screen.getByRole("table");
    expect(within(table).getAllByRole("row")).toHaveLength(1);
  });
});
