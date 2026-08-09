import { motion } from 'framer-motion';
import { Drawer } from 'vaul';

import { cn } from '../cn';

export interface PayerSheetProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children: React.ReactNode;
  className?: string;
}

/**
 * Bottom sheet for payer-facing detail on small screens, built on vaul.
 *
 * Used by the dashboard for charge detail, where operators are often on a
 * phone. Content is passed in; this component owns presentation only.
 */
export function PayerSheet({
  open,
  onOpenChange,
  title,
  children,
  className,
}: PayerSheetProps) {
  return (
    <Drawer.Root open={open} onOpenChange={onOpenChange}>
      <Drawer.Portal>
        <Drawer.Overlay className="fixed inset-0 bg-black/40" />
        <Drawer.Content
          className={cn(
            'fixed inset-x-0 bottom-0 mt-24 flex max-h-[90vh] flex-col rounded-t-2xl bg-base-100',
            className,
          )}
        >
          <div className="mx-auto my-3 h-1.5 w-12 rounded-full bg-base-300" />
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.18, ease: 'easeOut' }}
            className="overflow-y-auto px-4 pb-8"
          >
            <Drawer.Title className="mb-2 text-lg font-semibold">{title}</Drawer.Title>
            {children}
          </motion.div>
        </Drawer.Content>
      </Drawer.Portal>
    </Drawer.Root>
  );
}
