import type { Meta, StoryObj } from '@storybook/react-vite';

import { Button } from './button';

const meta = {
  title: 'Primitives/Button',
  component: Button,
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
  args: { children: 'Continue' },
} satisfies Meta<typeof Button>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Primary: Story = { args: { variant: 'primary' } };
export const Ghost: Story = { args: { variant: 'ghost' } };
export const Outline: Story = { args: { variant: 'outline' } };
export const Danger: Story = { args: { variant: 'danger' } };
export const Block: Story = { args: { block: true } };
export const Disabled: Story = { args: { disabled: true } };

/** Every variant and size side by side. */
export const AllVariants: Story = {
  render: () => (
    <div className="flex flex-col items-start gap-2">
      {(['primary', 'ghost', 'outline', 'danger'] as const).map((variant) => (
        <div key={variant} className="flex items-center gap-2">
          {(['xs', 'sm', 'md', 'lg'] as const).map((size) => (
            <Button key={size} variant={variant} size={size}>
              {variant} {size}
            </Button>
          ))}
        </div>
      ))}
    </div>
  ),
};
