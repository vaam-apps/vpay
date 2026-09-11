import type { Meta, StoryObj } from "@storybook/react-vite";

import { Logo } from "./logo";

const meta = {
  title: "Primitives/Logo",
  component: Logo,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Logo>;

export default meta;
type Story = StoryObj<typeof meta>;

export const OperatorMark: Story = {
  args: {
    src:
      "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' " +
      "viewBox='0 0 120 32'%3E%3Ctext y='24' font-size='24'%3EAcme%3C/text%3E%3C/svg%3E",
    alt: "Acme Store",
  },
};
