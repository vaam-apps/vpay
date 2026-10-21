import { Card, CardBody, CardHeader } from "@vaam-apps/ui";
import { ArrowLeft } from "lucide-react";
import NextLink from "next/link";
import type { ReactNode } from "react";

/**
 * The few layout pieces every screen shares, so a screen composes rather
 * than restates them. `@vaam-apps/ui` ships `ScreenStack` and
 * `ScreenHeader`, but `ScreenHeader` spends the page's `<h1>`, and this app's
 * `<h1>` is the brand in the shell — each screen's title is an `<h2>`, the
 * hierarchy `dashboard.cy.ts` finds every screen by.
 *
 * Every `className` here is a plain literal of 60 characters or fewer
 * (`just verify-ui`), which is why some elements nest: one element carries
 * the shape, its child the colour.
 */

/** A screen's title, its one-line description, and its actions. */
export function PageHeader({
  title,
  description,
  back,
  actions,
}: {
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly back?: { readonly href: string; readonly label: string };
  readonly actions?: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-end">
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        {back === undefined ? null : <BackLink {...back} />}
        <h2 className="font-display text-headline-medium text-foreground">
          {title}
        </h2>
        {description === undefined ? null : (
          <p className="max-w-3xl text-body-large text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      {actions === undefined ? null : (
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          {actions}
        </div>
      )}
    </div>
  );
}

/** "← Payments": an M3 text button that is a real link. */
export function BackLink({
  href,
  label,
}: {
  readonly href: string;
  readonly label: string;
}) {
  return (
    <NextLink
      href={href}
      className="-ml-3 self-start rounded-full hover:bg-primary/8"
    >
      <span className="flex h-10 items-center gap-2 px-3 text-primary">
        <ArrowLeft size={18} aria-hidden="true" />
        <span className="text-label-large">{label}</span>
      </span>
    </NextLink>
  );
}

/**
 * An M3 outlined button that is a real `<a>`: paging, "View all". A link
 * rather than a `Button` because these go somewhere — middle-click, copy
 * link and `rel="prev"`/`rel="next"` all keep working, and Cypress's paging
 * legs assert on `<a>`.
 */
export function OutlinedLink({
  href,
  rel,
  children,
}: {
  readonly href: string;
  readonly rel?: string;
  readonly children: ReactNode;
}) {
  return (
    <NextLink
      href={href}
      {...(rel === undefined ? {} : { rel })}
      className="inline-flex rounded-full text-primary hover:bg-primary/8"
    >
      <span className="flex rounded-full border border-outline-variant">
        <span className="flex h-10 items-center gap-2 px-4 text-label-large">
          {children}
        </span>
      </span>
    </NextLink>
  );
}

/**
 * A titled section of a screen: a `Card` (the diagnostic register — a
 * hairline on a surface step) inside a named `<section>` landmark, so a
 * screen-reader user can jump between them.
 */
export function Section({
  title,
  meta,
  action,
  flush = false,
  children,
}: {
  readonly title: string;
  readonly meta?: ReactNode;
  readonly action?: ReactNode;
  /** `true` for a table, which brings its own edge-to-edge rows. */
  readonly flush?: boolean;
  readonly children: ReactNode;
}) {
  return (
    <section aria-label={title} className="min-w-0">
      <Card>
        <CardHeader
          title={title}
          headingLevel={3}
          {...(meta === undefined ? {} : { meta })}
          {...(action === undefined ? {} : { action })}
        />
        {flush ? children : <CardBody>{children}</CardBody>}
      </Card>
    </section>
  );
}
