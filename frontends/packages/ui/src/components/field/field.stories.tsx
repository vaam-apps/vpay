import type { Meta, StoryObj } from "@storybook/react-vite";

import { Field, FieldDescription, FieldError, FieldLabel } from "./field";
import { Input } from "../input";

const meta = {
  title: "Primitives/Field",
  component: Field,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Field>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <Field>
      <FieldLabel>Phone number</FieldLabel>
      <Input placeholder="+237 6XX XXX XXX" />
      <FieldDescription>Include the country code.</FieldDescription>
    </Field>
  ),
};

export const Invalid: Story = {
  render: () => (
    <Field invalid>
      <FieldLabel>Phone number</FieldLabel>
      <Input tone="error" defaultValue="123" />
      <FieldError match>Enter a valid phone number.</FieldError>
    </Field>
  ),
};
