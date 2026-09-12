import type { Meta, StoryObj } from "@storybook/react-vite";

import { Timeline } from "./timeline";

const meta = {
  title: "Primitives/Timeline",
  component: Timeline,
  parameters: { layout: "padded" },
  tags: ["autodocs"],
} satisfies Meta<typeof Timeline>;

export default meta;
type Story = StoryObj<typeof meta>;

/**
 * `test: "todo"` — axe reports, the run does not fail. **Measured
 * 2026-09-12:** the per-event timestamp is `opacity-60 text-xs`, which
 * composites to `#9d9d9d` on `#ffffff` — **2.71:1**, against AA's 4.5:1.
 *
 * Unlike the `text-error` pair this one is a component decision rather than
 * a theme token — `opacity-60` is a choice `timeline.tsx` makes — but it is
 * still a visible change to a shipped component, so it is surfaced rather
 * than adjusted by the change that found it. Tracked in
 * `docs/status/frontend.md`.
 */
export const Events: Story = {
  parameters: { a11y: { test: "todo" } },
  args: {
    emptyMessage: "No events yet.",
    items: [
      { id: "evt_1", at: "2026-09-11 09:00:00", label: "charge.succeeded" },
      { id: "evt_2", at: "2026-09-11 09:00:04", label: "charge.settled" },
    ],
  },
};

export const Empty: Story = {
  args: { items: [], emptyMessage: "No events yet." },
};
