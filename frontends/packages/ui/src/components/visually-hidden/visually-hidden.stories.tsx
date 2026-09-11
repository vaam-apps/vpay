import type { Meta, StoryObj } from "@storybook/react-vite";

import { Text } from "../text";
import { VisuallyHidden } from "./visually-hidden";

const meta = {
  title: "Primitives/VisuallyHidden",
  component: VisuallyHidden,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof VisuallyHidden>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Only the number is on screen; a screen reader hears "Amount: 10,000 XAF". */
export const LabellingAnAmount: Story = {
  render: () => (
    <Text size="3xl" weight="semibold" numeric>
      <VisuallyHidden>Amount: </VisuallyHidden>10,000 XAF
    </Text>
  ),
};
