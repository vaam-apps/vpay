import type { Meta, StoryObj } from "@storybook/react-vite";

import { Code } from "../code";
import { StatusBadge } from "../status-badge";
import { DataList, DataListRow } from "./data-list";

const meta = {
  title: "Primitives/DataList",
  component: DataList,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof DataList>;

export default meta;
type Story = StoryObj<typeof meta>;

export const PaymentSummary: Story = {
  args: { children: null },
  render: () => (
    <DataList>
      <DataListRow label="Id">
        <Code wrap="anywhere">pi_test_000000000000000001</Code>
      </DataListRow>
      <DataListRow label="Status">
        <StatusBadge status="succeeded" />
      </DataListRow>
      <DataListRow label="Amount">10,000 XAF</DataListRow>
      <DataListRow label="Livemode">no</DataListRow>
    </DataList>
  ),
};
