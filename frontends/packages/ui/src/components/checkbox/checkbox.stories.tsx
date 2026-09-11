import type { Meta, StoryObj } from "@storybook/react-vite";

import { Checkbox } from "./checkbox";

const meta = {
  title: "Primitives/Checkbox",
  component: Checkbox,
  parameters: { layout: "centered" },
  tags: ["autodocs"],
  args: { "aria-label": "Remember this number for next time" },
} satisfies Meta<typeof Checkbox>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Checked: Story = { args: { defaultChecked: true } };
export const Primary: Story = {
  args: { tone: "primary", defaultChecked: true },
};
export const Small: Story = { args: { size: "sm" } };
export const Disabled: Story = { args: { disabled: true } };
