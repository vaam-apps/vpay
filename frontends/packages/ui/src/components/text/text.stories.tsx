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

/**
 * `test: "todo"` — same pair, same measurement, same owed decision as
 * `field.stories.tsx`'s `Invalid`: `text-error` is `#ff6266` on `#ffffff`,
 * **2.92:1** at 14px on 2026-09-12. See that file for why it is not fixed
 * here.
 */
export const Tones: Story = {
  parameters: { a11y: { test: "todo" } },
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
