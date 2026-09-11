import type { Meta, StoryObj } from "@storybook/react-vite";

import { Code } from "./code";

const meta = {
  title: "Primitives/Code",
  component: Code,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Code>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Identifier: Story = {
  args: { children: "pi_test_000000000000000001" },
};

export const LongAndBreakable: Story = {
  args: {
    wrap: "anywhere",
    children: "cs_test_000000000000000001_secret_0000000000000000000000",
  },
};
