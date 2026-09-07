'use client';

import { Drawer as BaseDrawer } from '@base-ui/react/drawer';

import { cn } from '../cn.js';

export interface DrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: React.ReactNode;
  className?: string;
}

/**
 * Bottom sheet for payer-facing detail on small screens.
 *
 * Replaces the `vaul` + `framer-motion` `PayerSheet` (plan §0.1, §5 Lane A
 * step 5): `@base-ui/react/drawer` exists in 1.8.0 and owns the swipe
 * gesture, focus trap and open/close transition itself, so both
 * dependencies are gone rather than pinned. Used by the dashboard for
 * charge detail, where operators are often on a phone. Content is passed
 * in; this component owns presentation only.
 */
export function Drawer({ open, onOpenChange, title, children, className }: DrawerProps) {
  return (
    <BaseDrawer.Root open={open} onOpenChange={onOpenChange}>
      <BaseDrawer.Portal>
        <BaseDrawer.Backdrop className="fixed inset-0 bg-black/40" />
        <BaseDrawer.Viewport className="fixed inset-x-0 bottom-0 mt-24 flex max-h-[90vh] flex-col">
          <BaseDrawer.Popup className={cn('bg-base-100 flex flex-col rounded-t-2xl', className)}>
            <div className="bg-base-300 mx-auto my-3 h-1.5 w-12 rounded-full" />
            <div className="overflow-y-auto px-4 pb-8">
              <BaseDrawer.Title className="mb-2 text-lg font-semibold">{title}</BaseDrawer.Title>
              {children}
            </div>
          </BaseDrawer.Popup>
        </BaseDrawer.Viewport>
      </BaseDrawer.Portal>
    </BaseDrawer.Root>
  );
}
