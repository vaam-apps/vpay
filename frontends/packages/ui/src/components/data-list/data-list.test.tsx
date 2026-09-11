import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { DataList, DataListRow } from "./data-list";

describe("DataList", () => {
  it("names each value with its own row header", () => {
    const { unmount } = render(
      <DataList data-testid="list">
        <DataListRow label="Id">pi_1</DataListRow>
        <DataListRow label="Amount">10,000 XAF</DataListRow>
      </DataList>,
    );
    // The row header is what makes the value readable out of context: a
    // screen reader announces "Amount, 10,000 XAF" only if `scope="row"` is
    // on the header cell.
    const header = screen.getByRole("rowheader", { name: "Id" });
    expect(header.getAttribute("scope")).toBe("row");
    expect(screen.getByRole("cell", { name: "pi_1" })).toBeTruthy();
    expect(screen.getAllByRole("row")).toHaveLength(2);
    // Not banded: banding is for many rows of one shape, not one object.
    expect(screen.getByTestId("list").className).not.toContain("table-zebra");
    unmount();
  });
});
