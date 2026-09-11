import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Card, CardBody } from "./card";

describe("Card / CardBody", () => {
  it("composes card and card-body", () => {
    render(
      <Card data-testid="card">
        <CardBody>Summary</CardBody>
      </Card>,
    );
    expect(screen.getByTestId("card").className).toContain("card");
    expect(screen.getByText("Summary").className).toContain("card-body");
  });
});
