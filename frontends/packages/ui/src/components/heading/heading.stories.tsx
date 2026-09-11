import type { Meta, StoryObj } from "@storybook/react-vite";

import { Heading } from "./heading";

const meta = {
  title: "Primitives/Heading",
  component: Heading,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Heading>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Levels: Story = {
  render: () => (
    <>
      <Heading level={1}>Confirm your payment</Heading>
      <Heading level={2}>Payment summary</Heading>
      <Heading level={3}>Support</Heading>
    </>
  ),
};
