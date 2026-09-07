'use client';

import { Select as BaseSelect } from '@base-ui/react/select';
import { cva, type VariantProps } from 'class-variance-authority';

import { cn } from '../cn.js';

const trigger = cva('select w-full', {
  variants: {
    size: { sm: 'select-sm', md: '' },
  },
  defaultVariants: { size: 'md' },
});

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
        className={cn(trigger({ size }), className)}
      >
        <BaseSelect.Value placeholder={placeholder} />
      </BaseSelect.Trigger>
      <BaseSelect.Portal>
        <BaseSelect.Positioner className="z-50" sideOffset={4}>
          {/*
           * The width uses Tailwind 4's PARENTHESISED bare-variable syntax.
           * Tailwind 4 dropped the square-bracket shorthand v3 accepted:
           * written that way the utility compiles to the literal declaration
           * `width: --anchor-width`, invalid CSS the browser drops, and the
           * popup sizes to its content instead of matching the trigger —
           * with no error anywhere. The daisyUI-4-class failure mode (plan
           * §6.3) in Tailwind's own syntax. `enforce-consistent-variable-
           * syntax` now gates it, and the class name is deliberately not
           * spelled the wrong way here: Tailwind's scanner reads comments
           * too, and would emit the dead rule from this very sentence.
           * `--anchor-width` is set by Base UI on `Select.Positioner`.
           */}
          <BaseSelect.Popup className="menu rounded-box border-base-300 bg-base-100 w-(--anchor-width) border p-1 shadow-lg">
            <BaseSelect.List>
              {items.map((item) => (
                <BaseSelect.Item
                  key={item.value}
                  value={item.value}
                  disabled={item.disabled}
                  className="rounded-box data-[highlighted]:bg-base-200 cursor-pointer px-3 py-2 outline-none"
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
