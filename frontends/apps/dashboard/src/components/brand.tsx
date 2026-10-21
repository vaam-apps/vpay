/**
 * The product mark in the top app bar: a primary-filled tile and the
 * wordmark. Decorative chrome in the seed's `primary` family — the one hue
 * `@vaam-apps/ui` reserves for chrome — so it can never read as a status.
 *
 * Not a heading. The document's one `<h1>` is the `sr-only` brand in
 * `AppShell`; this is what the eye sees.
 */
export function Brand() {
  return (
    <span className="flex min-w-0 items-center gap-3">
      <span
        aria-hidden="true"
        className="grid size-9 place-items-center rounded-xl bg-primary"
      >
        <span className="font-display text-title-large text-primary-content">
          v
        </span>
      </span>
      <span className="flex min-w-0 flex-col leading-none">
        <span className="font-display text-title-medium text-foreground">
          vpay
        </span>
        <span className="hidden text-label-small text-muted-foreground sm:block">
          Operator console
        </span>
      </span>
    </span>
  );
}
