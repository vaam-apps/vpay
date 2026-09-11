import type { Meta, StoryObj } from "@storybook/react-vite";

import { Badge } from "../badge";
import { Stack } from "./stack";

const meta = {
  title: "Primitives/Stack",
  component: Stack,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Stack>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Row: Story = {
  args: { gap: "sm" },
  render: (args) => (
    <Stack {...args}>
      <Badge>row</Badge>
      <Badge>gap-sm</Badge>
    </Stack>
  ),
};

export const Column: Story = {
  args: { direction: "column", gap: "md", align: "start" },
  render: (args) => (
    <Stack {...args}>
      <Badge>column</Badge>
      <Badge>gap-md</Badge>
    </Stack>
  ),
};
