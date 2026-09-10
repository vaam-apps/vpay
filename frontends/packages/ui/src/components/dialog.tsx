"use client";

import { Dialog as BaseDialog } from "@base-ui/react/dialog";

import { cn } from "../cn";

/**
 * A modal dialog.
 *
 * Thin styled wrappers around `@base-ui/react/dialog`'s own parts —
 * `Root`/`Trigger` are unstyled (behaviour only) and pass through
 * unchanged; `Backdrop`/`Popup`/`Title`/`Description`/`Close` take
 * daisyUI's `modal` classes. Composed the same way Base UI's own docs
 * compose it, so a consumer reads Base UI's documentation for anything
 * this file does not add.
 */
export const DialogRoot = BaseDialog.Root;
export const DialogTrigger = BaseDialog.Trigger;

export type DialogPortalProps = React.ComponentPropsWithoutRef<
  typeof BaseDialog.Portal
>;

export function DialogPortal({ children, ...rest }: DialogPortalProps) {
  return (
    <BaseDialog.Portal {...rest}>
      <BaseDialog.Backdrop className="modal-backdrop" />
      {children}
    </BaseDialog.Portal>
  );
}

export type DialogPopupProps = React.ComponentPropsWithoutRef<
  typeof BaseDialog.Popup
>;

export function DialogPopup({ className, ...rest }: DialogPopupProps) {
  return <BaseDialog.Popup className={cn("modal-box", className)} {...rest} />;
}

export type DialogTitleProps = React.ComponentPropsWithoutRef<
  typeof BaseDialog.Title
>;

export function DialogTitle({ className, ...rest }: DialogTitleProps) {
  return (
    <BaseDialog.Title
      className={cn("text-lg font-semibold", className)}
      {...rest}
    />
  );
}

export type DialogDescriptionProps = React.ComponentPropsWithoutRef<
  typeof BaseDialog.Description
>;

export function DialogDescription({
  className,
  ...rest
}: DialogDescriptionProps) {
  return (
    <BaseDialog.Description
      className={cn("text-sm opacity-70", className)}
      {...rest}
    />
  );
}

export const DialogClose = BaseDialog.Close;

/**
 * The full set, under one import — `import { Dialog } from '@vpay/ui'` then
 * `<Dialog.Root>`, `<Dialog.Trigger>`, `<Dialog.Portal>`, `<Dialog.Popup>`,
 * `<Dialog.Title>`, `<Dialog.Description>`, `<Dialog.Close>`.
 */
export const Dialog = {
  Root: DialogRoot,
  Trigger: DialogTrigger,
  Portal: DialogPortal,
  Popup: DialogPopup,
  Title: DialogTitle,
  Description: DialogDescription,
  Close: DialogClose,
};
