import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn';

/**
 * Layout and typography primitives with no Base UI primitive and no daisyUI
 * component class behind them — plain flex/spacing/type utilities. §3 of
 * docs/plans/2026-09-07-ui-revamp.md permits exactly this set of raw
 * utilities, and only inside this package: box model, spacing, non-colour
 * typography, position, `sr-only`, opacity. Every screen that used to write
 * `flex items-center gap-3` or `text-xs opacity-60` inline imports one of
 * these instead, so the utility literal exists in one file rather than once
 * per screen.
 */

const stack = cva('flex', {
  variants: {
    direction: { row: 'flex-row', column: 'flex-col' },
    align: {
      start: 'items-start',
      center: 'items-center',
      end: 'items-end',
      stretch: 'items-stretch',
    },
    justify: {
      start: 'justify-start',
      center: 'justify-center',
      end: 'justify-end',
      between: 'justify-between',
    },
    gap: { xs: 'gap-1', sm: 'gap-2', md: 'gap-4', lg: 'gap-6' },
    wrap: { true: 'flex-wrap', false: '' },
  },
  defaultVariants: { direction: 'row', align: 'center', justify: 'start', gap: 'sm', wrap: false },
});

export interface StackProps extends React.ComponentPropsWithoutRef<'div'>, VariantProps<typeof stack> {
  /**
   * The element to render. A `<div>` by default, because most stacks are
   * grouping and nothing else — but a stack that IS the page header or a
   * section is a landmark, and replacing it with a `<div>` silently removes
   * that landmark from the accessibility tree. `checkout-view.tsx` and
   * `return-view.tsx` each lost their `<header>` that way.
   */
  as?: 'div' | 'header' | 'footer' | 'section' | 'nav' | 'aside';
}

/** A flex row or column. Replaces `flex items-center gap-*` written inline per screen. */
export function Stack({
  as: Tag = 'div',
  direction,
  align,
  justify,
  gap,
  wrap,
  className,
  ...rest
}: StackProps) {
  return <Tag className={cn(stack({ direction, align, justify, gap, wrap }), className)} {...rest} />;
}

const text = cva('', {
  variants: {
    tone: { default: '', muted: 'opacity-60', error: 'text-error' },
    size: { xs: 'text-xs', sm: 'text-sm', md: 'text-base', lg: 'text-lg' },
    weight: { normal: '', medium: 'font-medium', semibold: 'font-semibold' },
  },
  defaultVariants: { tone: 'default', size: 'md', weight: 'normal' },
});

export interface TextProps extends React.ComponentPropsWithoutRef<'p'>, VariantProps<typeof text> {
  as?: 'p' | 'span';
}

/** Inline or block copy. `tone="muted"` / `tone="error"` replace ad hoc `opacity-*` / `text-error`. */
export function Text({ as: Tag = 'p', tone, size, weight, className, ...rest }: TextProps) {
  return <Tag className={cn(text({ tone, size, weight }), className)} {...rest} />;
}

export interface HeadingProps extends React.ComponentPropsWithoutRef<'h1'> {
  level?: 1 | 2 | 3;
}

/**
 * A screen or section heading.
 *
 * Styling only — `data-screen`, `tabIndex` and the focus-on-mount behaviour
 * a screen transition needs stay in the app (plan §4.1, `ScreenHeading`),
 * because they are screen navigation logic, not a look.
 */
const HEADING_TAGS = { 1: 'h1', 2: 'h2', 3: 'h3' } as const;

export function Heading({ level = 1, className, ...rest }: HeadingProps) {
  const Tag = HEADING_TAGS[level];
  return <Tag className={cn('text-xl font-semibold outline-none', className)} {...rest} />;
}

export type ListProps = React.ComponentPropsWithoutRef<'ul'>;

/** A plain vertical list. Replaces `mt-3 space-y-1 text-sm opacity-70` written inline. */
export function List({ className, ...rest }: ListProps) {
  return <ul className={cn('mt-3 space-y-1 text-sm opacity-70', className)} {...rest} />;
}

export type PageShellProps = React.ComponentPropsWithoutRef<'div'>;

/**
 * The one-column page frame every checkout and dashboard screen sits in.
 *
 * Replaces `mx-auto flex w-full max-w-md flex-col gap-6 p-6`, which plan
 * §4.1 found duplicated verbatim across `checkout-view.tsx` and
 * `return-view.tsx`.
 */
export function PageShell({ className, ...rest }: PageShellProps) {
  return <div className={cn('mx-auto flex w-full max-w-md flex-col gap-6 p-6', className)} {...rest} />;
}
