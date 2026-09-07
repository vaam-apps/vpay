import type { Meta, StoryObj } from '@storybook/react-vite';
import { useState } from 'react';

import { Button } from './button.js';
import { Drawer } from './drawer.js';

const meta = {
  title: 'Primitives/Drawer',
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

function ChargeDetailDrawer() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button onClick={() => setOpen(true)}>Open charge detail</Button>
      <Drawer open={open} onOpenChange={setOpen} title="Charge detail">
        <p className="text-sm opacity-70">10,000 XAF · MTN Mobile Money · succeeded</p>
      </Drawer>
    </>
  );
}

export const Default: Story = { render: () => <ChargeDetailDrawer /> };
