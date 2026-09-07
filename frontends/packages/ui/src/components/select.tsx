'use client';

import { Select as BaseSelect } from '@base-ui/react/select';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn';

const trigger = cva('select w-full', {
  variants: {
    size: { sm: 'select-sm', md: '' },
  },
  defaultVariants: { size: 'md' },
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
const POPUP_CLASS = 'menu w-(--anchor-width) rounded-box border border-base-300 bg-base-100 p-1 shadow-lg';
const ITEM_CLASS = 'cursor-pointer rounded-box px-3 py-2 outline-none data-[highlighted]:bg-base-200';

export interface SelectItem {
  value: string;
  label: string;
  disabled?: boolean;
}

export interface SelectProps extends VariantProps<typeof trigger> {
  items: SelectItem[];
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  name?: string;
  id?: string;
  className?: string;
  'aria-label'?: string;
  /**
   * The id of a VISIBLE element naming this control. Preferred over
   * `aria-label`: a name only a screen reader can read leaves a sighted
   * payer looking at an unlabelled combobox.
   */
  'aria-labelledby'?: string;
}

/**
 * A single-value picker.
 *
 * `@base-ui/react/select` renders a button trigger plus a portalled popup
 * listbox — not a native `<select>` — so it composes with `Field` the same
 * way `Input` does, and gets full keyboard/ARIA listbox behaviour for free.
 * `items` is a plain array rather than `children`: every call site in this
 * product (the checkout locale switch) is a flat list with no groups, and a
 * plain prop keeps the call site a one-liner.
 */
export function Select({
  items,
  value,
  defaultValue,
  onValueChange,
  placeholder,
  disabled,
  size,
  name,
  id,
  className,
  'aria-label': ariaLabel,
  'aria-labelledby': ariaLabelledBy,
}: SelectProps) {
  return (
    <BaseSelect.Root
      items={items}
      value={value}
      defaultValue={defaultValue}
      onValueChange={(next) => {
        // This component is always single-value (`multiple` is not exposed),
        // so `next` is only ever `null` before a value has ever been chosen
        // — which `value`/`defaultValue` already rule out for a controlled
        // or defaulted call site.
        if (next !== null) onValueChange?.(next);
      }}
      disabled={disabled}
      name={name}
    >
      <BaseSelect.Trigger
        id={id}
        aria-label={ariaLabel}
        aria-labelledby={ariaLabelledBy}
        className={cn(trigger({ size }), className)}
      >
        <BaseSelect.Value placeholder={placeholder} />
      </BaseSelect.Trigger>
      <BaseSelect.Portal>
        <BaseSelect.Positioner className="z-50" sideOffset={4}>
          <BaseSelect.Popup className={POPUP_CLASS}>
            <BaseSelect.List>
              {items.map((item) => (
                <BaseSelect.Item
                  key={item.value}
                  value={item.value}
                  disabled={item.disabled}
                  className={ITEM_CLASS}
                >
                  <BaseSelect.ItemText>{item.label}</BaseSelect.ItemText>
                </BaseSelect.Item>
              ))}
            </BaseSelect.List>
          </BaseSelect.Popup>
        </BaseSelect.Positioner>
      </BaseSelect.Portal>
    </BaseSelect.Root>
  );
}
