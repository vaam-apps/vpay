import type { Meta, StoryObj } from '@storybook/react-vite';

import { Radio, RadioGroup } from './radio';

const meta = {
  title: 'Primitives/Radio',
  component: RadioGroup,
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
} satisfies Meta<typeof RadioGroup>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <RadioGroup aria-label="Surface" defaultValue="hosted">
      <label className="flex items-center gap-2">
        <Radio value="hosted" /> Hosted
      </label>
      <label className="flex items-center gap-2">
        <Radio value="embedded" /> Embedded
      </label>
      <label className="flex items-center gap-2">
        <Radio value="popup" /> Popup
      </label>
    </RadioGroup>
  ),
};

export const Small: Story = {
  render: () => (
    <RadioGroup aria-label="Surface" defaultValue="hosted">
      <label className="flex items-center gap-2">
        <Radio value="hosted" size="sm" /> Hosted
      </label>
      <label className="flex items-center gap-2">
        <Radio value="embedded" size="sm" /> Embedded
      </label>
    </RadioGroup>
  ),
};
