import type { Meta, StoryObj } from '@storybook/react';
import { PAYMENT_STATUS } from '@vpay/tokens';

import { StatusBadge } from './status-badge.js';

const meta = {
  title: 'Payments/StatusBadge',
  component: StatusBadge,
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
} satisfies Meta<typeof StatusBadge>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Succeeded: Story = { args: { status: 'succeeded' } };
export const Processing: Story = { args: { status: 'processing' } };
export const RequiresAction: Story = { args: { status: 'requires_action' } };

/** Every status side by side, so a missing or duplicated tone is obvious. */
export const AllStatuses: Story = {
  args: { status: 'succeeded' },
  render: () => (
    <div className="flex flex-col items-start gap-2">
      {PAYMENT_STATUS.map((s) => (
        <StatusBadge key={s} status={s} />
      ))}
    </div>
  ),
};
