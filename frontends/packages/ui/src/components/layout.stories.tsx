import type { Meta, StoryObj } from "@storybook/react-vite";

import { Heading, List, PageShell, Stack, Text } from "./layout";

const meta = {
  title: "Primitives/Layout",
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const Headings: Story = {
  render: () => (
    <div className="flex flex-col gap-2">
      <Heading level={1}>Confirm your payment</Heading>
      <Heading level={2}>Payment summary</Heading>
      <Heading level={3}>Support</Heading>
    </div>
  ),
};

export const TextTones: Story = {
  render: () => (
    <div className="flex flex-col gap-1">
      <Text>Default</Text>
      <Text tone="muted">Muted, for a support line</Text>
      <Text tone="error" size="sm">
        An inline validation message
      </Text>
    </div>
  ),
};

export const StackDirections: Story = {
  render: () => (
    <div className="flex flex-col gap-4">
      <Stack gap="sm">
        <span className="badge">row</span>
        <span className="badge">gap-sm</span>
      </Stack>
      <Stack direction="column" gap="md">
        <span className="badge">column</span>
        <span className="badge">gap-md</span>
      </Stack>
    </div>
  ),
};

export const ListExample: Story = {
  render: () => (
    <List>
      <li>MTN Mobile Money is not available for this amount.</li>
      <li>Orange Money is not available in this country.</li>
    </List>
  ),
};

export const PageShellExample: Story = {
  render: () => (
    <PageShell className="border border-base-300">
      <Heading level={1}>Confirm your payment</Heading>
      <Text tone="muted">10,000 XAF to Acme Store</Text>
    </PageShell>
  ),
};
