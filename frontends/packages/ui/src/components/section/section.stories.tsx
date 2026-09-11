import type { Meta, StoryObj } from "@storybook/react-vite";

import { Text } from "../text";
import { Section } from "./section";

const meta = {
  title: "Primitives/Section",
  component: Section,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Section>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Titled: Story = {
  args: {
    title: "Charge",
    children: (
      <Text tone="muted">
        No charge. Nobody has confirmed this payment intent.
      </Text>
    ),
  },
};
