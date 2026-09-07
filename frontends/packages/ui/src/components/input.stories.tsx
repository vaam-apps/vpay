import type { Meta, StoryObj } from '@storybook/react-vite';

import { Input } from './input';

const meta = {
  title: 'Primitives/Input',
  component: Input,
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
  args: { placeholder: '+237 6XX XXX XXX', 'aria-label': 'MSISDN' },
} satisfies Meta<typeof Input>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {};
export const Ghost: Story = { args: { tone: 'ghost' } };
export const Error: Story = { args: { tone: 'error', defaultValue: '123' } };
export const Disabled: Story = { args: { disabled: true } };
