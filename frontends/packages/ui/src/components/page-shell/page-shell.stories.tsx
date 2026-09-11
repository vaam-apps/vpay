import type { Meta, StoryObj } from "@storybook/react-vite";

import { Heading } from "../heading";
import { Text } from "../text";
import { PageShell } from "./page-shell";

const meta = {
  title: "Primitives/PageShell",
  component: PageShell,
  parameters: { layout: "fullscreen" },
  tags: ["autodocs"],
} satisfies Meta<typeof PageShell>;

export default meta;
type Story = StoryObj<typeof meta>;

export const CheckoutFrame: Story = {
  render: () => (
    <PageShell>
      <Heading level={1}>Confirm your payment</Heading>
      <Text tone="muted">10,000 XAF to Acme Store</Text>
    </PageShell>
  ),
};
