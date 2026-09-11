import type { Meta, StoryObj } from "@storybook/react-vite";

import { Pagination } from "./pagination";

const meta = {
  title: "Primitives/Pagination",
  component: Pagination,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Pagination>;

export default meta;
type Story = StoryObj<typeof meta>;

export const BothWays: Story = {
  args: {
    label: "Payments paging",
    previous: <a href="#previous">Previous</a>,
    next: <a href="#next">Next</a>,
  },
};

export const FirstPage: Story = {
  args: {
    label: "Payments paging",
    previous: null,
    next: <a href="#next">Next</a>,
  },
};
