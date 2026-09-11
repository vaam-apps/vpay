import type { Meta, StoryObj } from "@storybook/react-vite";

import { List } from "./list";

const meta = {
  title: "Primitives/List",
  component: List,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof List>;

export default meta;
type Story = StoryObj<typeof meta>;

export const UnsupportedRails: Story = {
  render: () => (
    <List>
      <li>MTN Mobile Money is not available for this amount.</li>
      <li>Orange Money is not available in this country.</li>
    </List>
  ),
};
