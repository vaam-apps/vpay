import type { Meta, StoryObj } from "@storybook/react-vite";

import { Alert } from "./alert";

const meta = {
  title: "Primitives/Alert",
  component: Alert,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
  args: { children: "Your payment could not be confirmed." },
} satisfies Meta<typeof Alert>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Neutral: Story = { args: { tone: "neutral" } };
export const Info: Story = { args: { tone: "info" } };
export const Success: Story = {
  args: { tone: "success", children: "Payment succeeded." },
};
export const Warning: Story = {
  args: { tone: "warning", children: "This payment was canceled." },
};
export const Error: Story = {
  args: { tone: "error", children: "This payment failed." },
};

export const AllTones: Story = {
  render: () => (
    <div className="flex flex-col gap-2">
      {(["neutral", "info", "success", "warning", "error"] as const).map(
        (tone) => (
          <Alert key={tone} tone={tone}>
            {tone}
          </Alert>
        ),
      )}
    </div>
  ),
};
