import type { Meta, StoryObj } from '@storybook/react-vite';

import { Card, CardBody } from './card.js';

const meta = {
  title: 'Primitives/Card',
  component: Card,
  parameters: { layout: 'padded' },
  tags: ['autodocs'],
} satisfies Meta<typeof Card>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <Card>
      <CardBody>
        <p>10,000 XAF</p>
        <p className="text-sm opacity-70">MTN Mobile Money</p>
      </CardBody>
    </Card>
  ),
};

export const Small: Story = {
  render: () => (
    <Card size="sm">
      <CardBody>Compact summary</CardBody>
    </Card>
  ),
};
