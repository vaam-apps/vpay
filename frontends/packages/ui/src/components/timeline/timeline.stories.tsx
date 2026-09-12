import type { Meta, StoryObj } from "@storybook/react-vite";

import { Timeline } from "./timeline";

const meta = {
  title: "Primitives/Timeline",
  component: Timeline,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Timeline>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Events: Story = {
  args: {
    emptyMessage: "No events yet.",
    items: [
      { id: "evt_1", at: "2026-09-11 09:00:00", label: "charge.succeeded" },
      { id: "evt_2", at: "2026-09-11 09:00:04", label: "charge.settled" },
    ],
  },
};

export const Empty: Story = {
  args: { items: [], emptyMessage: "No events yet." },
};
