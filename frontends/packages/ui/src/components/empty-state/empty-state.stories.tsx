import type { Meta, StoryObj } from "@storybook/react-vite";

import { Button } from "../button";
import { EmptyState } from "./empty-state";

const meta = {
  title: "Primitives/EmptyState",
  component: EmptyState,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof EmptyState>;

export default meta;
type Story = StoryObj<typeof meta>;

export const NothingYet: Story = {
  args: {
    title: "No payments",
    description: "This merchant has no payment intents yet.",
  },
};

export const NothingMatched: Story = {
  args: {
    title: "No payments",
    description:
      "No payment matched these filters. Widen the range or clear the status.",
    action: (
      <Button variant="outline" size="sm">
        Clear filters
      </Button>
    ),
  },
};
