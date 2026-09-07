import type { Meta, StoryObj } from '@storybook/react-vite';

import { Button } from './button';
import { Dialog } from './dialog';

const meta = {
  title: 'Primitives/Dialog',
  parameters: { layout: 'centered' },
  tags: ['autodocs'],
} satisfies Meta;

export default meta;
type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => (
    <Dialog.Root>
      <Dialog.Trigger render={<Button variant="outline" />}>Show sign-in error</Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Popup>
          <Dialog.Title>Sign-in failed</Dialog.Title>
          <Dialog.Description>Check the code and try again.</Dialog.Description>
          <div className="mt-4 flex justify-end">
            <Dialog.Close render={<Button variant="ghost" />}>Dismiss</Dialog.Close>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  ),
};
