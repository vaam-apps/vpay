"use client";

import { useState } from "react";

import { Button, Code, MoreDetailDrawer } from "@vaam-apps/ui";

export interface ManualKeySheetProps {
  /** The base32 secret an authenticator app takes when it cannot scan. */
  secret: string;
}

/**
 * The typed-in alternative to the QR code, in a bottom sheet.
 *
 * A choice — scan, or type the key — is a sheet and never a menu, and the
 * enrolment screen stays one question: *what code does your app show?* The key
 * is behind a button because most people scan, and a base32 string on the
 * screen they are scanning is noise that invites typing it by hand.
 *
 * `MoreDetailDrawer` is `@vaam-apps/ui`'s modal sheet: a bottom sheet on a
 * phone and a side panel from `md`, with focus moved into it and returned on
 * close. Nothing is a `<select>`, a menu or a popover (`no-dropdowns.test.ts`).
 *
 * The secret is rendered here, in the browser, because the person has to read
 * it — that is enrolment, not a leak. It reaches this component as a prop from
 * the server page, which has already shown the same value as a QR code.
 */
export function ManualKeySheet({ secret }: ManualKeySheetProps) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        type="button"
        variant="ghost"
        className="w-full"
        onClick={() => setOpen(true)}
      >
        Cannot scan? Enter the key instead
      </Button>
      <MoreDetailDrawer
        open={open}
        onOpenChange={setOpen}
        title="Enter this key in your app"
        description="Choose to enter a setup key, then type the key below. Spaces do not matter."
        footer={
          <Button type="button" onClick={() => setOpen(false)}>
            Done
          </Button>
        }
      >
        {/* `Code` takes no `data-testid` (it spreads no `...rest`), so the
            wrapper carries it; the end-to-end suite reads the wrapper's own
            text, which is exactly the secret because `Code` adds no other. */}
        <p className="text-body text-foreground">
          <span data-testid="totp-secret">
            <Code>{secret}</Code>
          </span>
        </p>
      </MoreDetailDrawer>
    </>
  );
}
