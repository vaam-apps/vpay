import type { Meta, StoryObj } from "@storybook/react-vite";

import { Text } from "../text";
import { LiveRegion } from "./live-region";

const meta = {
  title: "Primitives/LiveRegion",
  component: LiveRegion,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof LiveRegion>;

export default meta;
type Story = StoryObj<typeof meta>;

/** Nothing to look at: the point of this element is what it announces. */
export const AnnouncingAScreenChange: Story = {
  render: () => (
    <LiveRegion>
      <Text>Waiting for your approval on your phone.</Text>
    </LiveRegion>
  ),
};
