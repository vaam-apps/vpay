import { Card, Skeleton } from "@vaam-apps/ui";
import { ArrowLeft, Check } from "lucide-react";
import type { ReactNode } from "react";

import type { ListScreenCopy } from "../dash/list-screens";

/**
 * The placeholders every screen's `loading.tsx` is built from.
 *
 * # Sized by the real strings, not by guessed widths
 *
 * Each placeholder holds an **invisible copy** of the text the screen will
 * render, in the same type, inside the same layout `PageHeader` and
 * `FilterChips` use. So the title placeholder is exactly as tall as the
 * `<h2>`, the description's wraps onto two lines on a phone exactly when the
 * description does, and a chip placeholder is the chip's own width. The
 * payments skeleton's history (`payments-skeleton.tsx`) is why: every fixed
 * width this app tried was off by something at some window size, and the
 * screen jumped when the rows arrived.
 *
 * What is *not* claimed to line up is everything below the filter row: how
 * many rows a read returns is not known until it returns.
 *
 * No `<h2>` is rendered — `dashboard.cy.ts` waits on `cy.contains("h2", …)`
 * as the sign a screen has loaded — and everything but the "Loading…" text
 * is `aria-hidden`, through `Skeleton`.
 */
export function PageHeaderSkeleton({
  title,
  description,
  back,
}: {
  readonly title: string;
  readonly description?: ReactNode;
  readonly back?: string;
}) {
  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-end">
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        {back === undefined ? null : (
          <Skeleton className="-ml-3 self-start rounded-full">
            <span className="invisible flex h-10 items-center gap-2 px-3">
              <ArrowLeft size={18} aria-hidden="true" />
              <span className="text-label-large">{back}</span>
            </span>
          </Skeleton>
        )}
        <Skeleton className="w-fit font-display text-headline-medium">
          <span className="invisible">{title}</span>
        </Skeleton>
        {description === undefined ? null : (
          <Skeleton className="w-fit max-w-3xl text-body-large">
            <span className="invisible">{description}</span>
          </Skeleton>
        )}
      </div>
    </div>
  );
}

/** `FilterChips`' row, chip for chip, with "All" first and selected. */
export function FilterChipsSkeleton({
  labels,
}: {
  readonly labels: readonly string[];
}) {
  return (
    <div className="flex flex-wrap gap-2">
      <Skeleton className="flex h-8 rounded-lg">
        <span className="invisible flex items-center gap-1 pr-3 pl-2">
          <Check size={16} aria-hidden="true" />
          <span className="text-label-large">All</span>
        </span>
      </Skeleton>
      {labels.map((label) => (
        <Skeleton
          key={label}
          className="flex h-8 rounded-lg border border-transparent"
        >
          <span className="invisible flex items-center px-3">
            <span className="text-label-large">{label}</span>
          </span>
        </Skeleton>
      ))}
    </div>
  );
}

/** A table card's placeholder: a header band and `rows` row bars. */
export function TableCardSkeleton({ rows = 8 }: { readonly rows?: number }) {
  return (
    <Card className="overflow-hidden">
      <div className="flex h-8 items-center border-b border-edge px-3">
        <Skeleton className="h-2.5 w-24" />
      </div>
      <div className="flex flex-col divide-y divide-edge-subtle">
        {Array.from({ length: rows }, (_, row) => (
          <div key={row} className="flex h-11 items-center gap-6 px-3">
            <Skeleton className="h-3.5 w-36" />
            <Skeleton className="h-3.5 w-20" />
            <Skeleton className="h-5 w-24 rounded-full" />
          </div>
        ))}
      </div>
    </Card>
  );
}

/**
 * What a procedure-backed list shows between a click and its rows.
 *
 * Rendered by each list route's `loading.tsx`, from the same
 * `ListScreenCopy` its `page.tsx` renders. Before 2026-10-21 those routes had
 * no loading boundary at all, so following a link left the previous screen
 * on display, unchanged and still clickable, until the next one had been
 * read in full on the server — nothing said the click had landed.
 */
export function ListSkeleton({ title, description, filter }: ListScreenCopy) {
  return (
    <div role="status" aria-busy="true" className="flex flex-col gap-6">
      <span className="sr-only">Loading {title}…</span>
      <PageHeaderSkeleton title={title} description={description} />
      {filter === undefined ? null : (
        <FilterChipsSkeleton
          labels={filter.options.map((option) => option.label)}
        />
      )}
      <TableCardSkeleton />
    </div>
  );
}
