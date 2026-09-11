import type { Meta, StoryObj } from "@storybook/react-vite";

import { Link } from "./link";

const meta = {
  title: "Primitives/Link",
  component: Link,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Link>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  args: { href: "#", children: "Back to payments" },
};

export const UnderlineOnHover: Story = {
  args: { href: "#", tone: "hover", children: "Next" },
};
