import { type ClassValue, clsx } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

/**
 * daisyUI's component modifiers, registered as `tailwind-merge` conflict
 * groups.
 *
 * `twMerge` only knows Tailwind's own utility classes. Left alone,
 * `cn('btn btn-primary', 'btn-ghost')` keeps **both** classes — daisyUI's
 * CSS then resolves the tie however cascade order happens to fall out, which
 * is not what an override prop is for. Every group below is one daisyUI
 * component whose listed modifiers are mutually exclusive by design (a
 * button has exactly one colour, not `btn-primary btn-ghost` at once), so
 * the later class in a `cn()` call wins, the same way `bg-red-500
 * bg-blue-500` already does for a plain Tailwind utility.
 *
 * Scoped to the daisyUI classes this package's components actually emit —
 * add a group here when a component gains a new variant dimension, not
 * ahead of one.
 *
 * **Colour and style are two groups, not one, for `btn` and `badge.`** Plan
 * §3's illustrative snippet put `outline`/`soft`/`dash` in the same group as
 * the colours; daisyUI 5.7.28's own compiled CSS says they are orthogonal:
 *
 *     .btn-primary   { --btn-color: var(--color-primary); … }
 *     .btn-outline,
 *     .btn-dash      { --btn-bg:#0000; --btn-border: var(--btn-color, …); … }
 *     .badge-primary { --badge-color: var(--color-primary); … }
 *     .badge-outline { color: var(--badge-color); --badge-bg:#0000; … }
 *
 * — the style classes *read* the variable the colour classes set, so
 * `btn-primary btn-outline` is how daisyUI 5 spells "a primary outline
 * button". Grouped together, `cn('btn-primary', 'btn-outline')` returned
 * `btn-outline` and the colour vanished with nothing to report it: a class
 * that still parses, still renders and quietly stops doing anything, which
 * is the exact failure mode plan §6.3 exists to catch, arriving through the
 * merge function itself.
 *
 * `ghost` and `link` deliberately stay with the COLOURS, even though the CSS
 * would allow the same argument: `Button`'s own `cva` map offers `ghost` as
 * an alternative to `primary`, and plan §3 pins `cn('btn btn-primary',
 * 'btn-ghost') === 'btn btn-ghost'` as this file's named acceptance case.
 * Narrowing that is a product decision, not a review's, and it is recorded as
 * an open question in docs/plans/exp26-notes/lane-a-review.md rather than
 * taken here.
 */
type DaisyClassGroupId =
  | "daisy-btn-variant"
  | "daisy-btn-style"
  | "daisy-btn-size"
  | "daisy-badge-variant"
  | "daisy-badge-style"
  | "daisy-badge-size"
  | "daisy-alert-variant"
  | "daisy-input-variant"
  | "daisy-select-variant"
  | "daisy-select-size"
  | "daisy-checkbox-variant"
  | "daisy-checkbox-size"
  | "daisy-radio-variant"
  | "daisy-radio-size"
  | "daisy-card-size"
  | "daisy-table-size"
  | "daisy-loading-type"
  | "daisy-loading-size";

const twMerge = extendTailwindMerge<DaisyClassGroupId>({
  extend: {
    classGroups: {
      "daisy-btn-variant": [
        {
          btn: [
            "primary",
            "secondary",
            "accent",
            "neutral",
            "info",
            "success",
            "warning",
            "error",
            "ghost",
            "link",
          ],
        },
      ],
      "daisy-btn-style": [{ btn: ["outline", "soft", "dash"] }],
      "daisy-btn-size": [{ btn: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-badge-variant": [
        {
          badge: [
            "primary",
            "secondary",
            "accent",
            "neutral",
            "info",
            "success",
            "warning",
            "error",
            "ghost",
          ],
        },
      ],
      "daisy-badge-style": [{ badge: ["outline", "soft", "dash"] }],
      "daisy-badge-size": [{ badge: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-alert-variant": [
        { alert: ["info", "success", "warning", "error"] },
      ],
      "daisy-input-variant": [{ input: ["ghost", "error"] }],
      "daisy-select-variant": [{ select: ["ghost", "error"] }],
      "daisy-select-size": [{ select: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-checkbox-variant": [
        {
          checkbox: [
            "primary",
            "secondary",
            "accent",
            "neutral",
            "success",
            "warning",
            "error",
          ],
        },
      ],
      "daisy-checkbox-size": [{ checkbox: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-radio-variant": [
        {
          radio: [
            "primary",
            "secondary",
            "accent",
            "neutral",
            "success",
            "warning",
            "error",
          ],
        },
      ],
      "daisy-radio-size": [{ radio: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-card-size": [{ card: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-table-size": [{ table: ["xs", "sm", "md", "lg", "xl"] }],
      "daisy-loading-type": [
        { loading: ["spinner", "dots", "ring", "ball", "bars", "infinity"] },
      ],
      "daisy-loading-size": [{ loading: ["xs", "sm", "md", "lg", "xl"] }],
    },
  },
});

/** Merge conditional class names, letting later Tailwind AND daisyUI classes win. */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
