import type { Meta, StoryObj } from '@storybook/react-vite';

import { Select } from './select';

const LOCALES = [
  { value: 'en', label: 'English' },
  { value: 'fr', label: 'Français' },
];

const meta = {
  title: 'Primitives/Select',
  component: Select,
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
  args: { items: LOCALES, 'aria-label': 'Locale' },
} satisfies Meta<typeof Select>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = { args: { defaultValue: 'en' } };
export const Small: Story = { args: { defaultValue: 'en', size: 'sm' } };
export const Placeholder: Story = { args: { placeholder: 'Choose a locale' } };
