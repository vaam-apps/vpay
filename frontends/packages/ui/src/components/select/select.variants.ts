import { cva } from "class-variance-authority";

export const trigger = cva("select w-full", {
  variants: {
    size: { sm: "select-sm", md: "" },
  },
  defaultVariants: { size: "md" },
});

/**
 * The popup and its items carry static classes, not variants, so they are
 * plain constants rather than a second `cva` map — and they sit here rather
 * than inline because at ten levels of JSX indentation an 80-character class
 * string no longer fits the one-line rule (@vpay/config's
 * `enforce-consistent-line-wrapping`), and wrapping it would be the thing
 * that rule exists to prevent.
 *
 * The popup's width uses Tailwind 4's PARENTHESISED bare-variable syntax.
 * Tailwind 4 dropped the square-bracket shorthand Tailwind 3 accepted, and
 * does not reject it: written that way the utility compiles to the literal
 * declaration `width: --anchor-width`, invalid CSS the browser drops, so the
 * popup sizes to its own content instead of matching the trigger — with no
 * error anywhere. That is plan §6.3's failure mode in Tailwind's own syntax;
 * `enforce-consistent-variable-syntax` gates it now. The wrong form is
 * deliberately not spelled out here: Tailwind's scanner reads comments too,
 * and would emit the dead rule from this very sentence. `--anchor-width` is
 * set by Base UI on `Select.Positioner`.
 */
export const POPUP_CLASS =
  "menu w-(--anchor-width) rounded-box border border-base-300 bg-base-100 p-1 shadow-lg";
export const ITEM_CLASS =
  "cursor-pointer rounded-box px-3 py-2 outline-none data-[highlighted]:bg-base-200";
