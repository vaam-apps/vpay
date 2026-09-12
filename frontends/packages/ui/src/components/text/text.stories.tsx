import type { Meta, StoryObj } from "@storybook/react-vite";

import { Text } from "./text";

const meta = {
  title: "Primitives/Text",
  component: Text,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Text>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Tones: Story = {
  render: () => (
    <>
      <Text>Default</Text>
      <Text tone="muted">Muted, for a support line</Text>
      <Text tone="error" size="sm">
        An inline validation message
      </Text>
    </>
  ),
};

export const Amount: Story = {
  args: { size: "3xl", weight: "semibold", numeric: true, children: "10,000" },
};
