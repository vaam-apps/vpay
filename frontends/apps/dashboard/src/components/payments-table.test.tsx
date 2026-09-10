import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

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

  it("takes the status pill tone from @vpay/tokens, never a local map", () => {
    const { container } = render(
      <PaymentsTable rows={[INTENT, OTHER_INTENT]} />,
    );
    // `badge-success` for succeeded, `badge-info` for processing — the same
    // classes StatusBadge renders anywhere else in the product.
    expect(container.querySelector(".badge-success")).not.toBeNull();
    expect(container.querySelector(".badge-info")).not.toBeNull();
  });

  it("renders a status this build cannot name as text, never as a coloured pill", () => {
    const { container } = render(
      <PaymentsTable rows={[{ ...INTENT, status: "disputed" }]} />,
    );
    expect(screen.getByText("disputed")).toBeInTheDocument();
    expect(container.querySelector(".badge")).toBeNull();
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
