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

/**
 * `test: "todo"` — axe reports, the run does not fail. **Measured
 * 2026-09-12**, the first run of `pnpm --filter @vpay/ui test-storybook`:
 * `FieldError` paints `text-error`, which is bumblebee's `--color-error`
 * (`oklch(70% .191 22.216)` = `#ff6266`) on `--color-base-100` (`#ffffff`)
 * at 12px — **2.92:1**, against AA's 4.5:1.
 *
 * Not fixed here, and not hidden: `--color-error` is also the *background*
 * `.alert-error` paints, which `theme-contrast.test.ts` and the checkout's
 * `outcome-contrast.test.ts` both pin. Moving it moves the payment-failure
 * screen, so it is a theme decision of the same kind as the
 * `--color-error-content` change in `src/styles.css`, which that file
 * records as the maintainer's delegate's call on 2026-09-07. It is owed a
 * decision, not a quiet edit from the change that happened to find it.
 * Tracked in `docs/status/frontend.md`.
 */
export const Invalid: Story = {
  parameters: { a11y: { test: "todo" } },
  render: () => (
    <Field invalid>
      <FieldLabel>Phone number</FieldLabel>
      <Input tone="error" defaultValue="123" />
      <FieldError match>Enter a valid phone number.</FieldError>
    </Field>
  ),
};
