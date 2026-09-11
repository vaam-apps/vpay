import type { Meta, StoryObj } from "@storybook/react-vite";

import { Table } from "./table";

const meta = {
  title: "Primitives/Table",
  component: Table,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Table>;

export default meta;
type Story = StoryObj<typeof meta>;

function Rows() {
  return (
    <>
      <thead>
        <tr>
          <th>Item</th>
          <th>Qty</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td>Coffee</td>
          <td>2</td>
        </tr>
        <tr>
          <td>Tea</td>
          <td>1</td>
        </tr>
      </tbody>
    </>
  );
}

export const Default: Story = {
  render: () => (
    <Table>
      <Rows />
    </Table>
  ),
};
export const Zebra: Story = {
  render: () => (
    <Table zebra>
      <Rows />
    </Table>
  ),
};
export const Small: Story = {
  render: () => (
    <Table size="sm">
      <Rows />
    </Table>
  ),
};
