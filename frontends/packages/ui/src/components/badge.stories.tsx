import type { Meta, StoryObj } from '@storybook/react-vite';

import { Badge } from './badge';

const meta = {
  title: 'Primitives/Badge',
  component: Badge,
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
  args: { children: 'Paid' },
} satisfies Meta<typeof Badge>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Neutral: Story = { args: { tone: 'neutral' } };
export const Info: Story = { args: { tone: 'info' } };
export const Success: Story = { args: { tone: 'success' } };
export const Warning: Story = { args: { tone: 'warning' } };
export const Error: Story = { args: { tone: 'error' } };
export const Ghost: Story = { args: { tone: 'ghost' } };

export const AllTonesAndSizes: Story = {
  render: () => (
    <div className="flex flex-col gap-2">
      {(['neutral', 'info', 'success', 'warning', 'error', 'ghost'] as const).map((tone) => (
        <div key={tone} className="flex items-center gap-2">
          {(['xs', 'sm', 'md', 'lg'] as const).map((size) => (
            <Badge key={size} tone={tone} size={size}>
              {tone}
            </Badge>
          ))}
        </div>
      ))}
    </div>
  ),
};
