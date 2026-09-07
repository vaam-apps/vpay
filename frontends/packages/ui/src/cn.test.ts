import { describe, expect, it } from 'vitest';

import { cn } from './cn.js';

/**
 * One case per `classGroups` entry in `cn.ts`. Deleting any one entry from
 * that file must fail exactly the case named after it — that is the
 * decisive mutation docs/plans/2026-09-07-ui-revamp.md §3 names for this
 * file, and the reason this is a unit test rather than a review habit.
 */
describe('cn', () => {
  it('plain Tailwind utilities still merge (tailwind-merge unchanged)', () => {
    expect(cn('p-2 text-sm', 'p-4')).toBe('text-sm p-4');
  });

  it('daisy-btn-variant: a later btn colour replaces an earlier one', () => {
    expect(cn('btn btn-primary', 'btn-ghost')).toBe('btn btn-ghost');
  });

  it('daisy-btn-size: a later btn size replaces an earlier one', () => {
    expect(cn('btn btn-lg', 'btn-xs')).toBe('btn btn-xs');
  });

  it('daisy-badge-variant: a later badge colour replaces an earlier one', () => {
    expect(cn('badge badge-neutral', 'badge-success')).toBe('badge badge-success');
  });

  it('daisy-badge-size: a later badge size replaces an earlier one', () => {
    expect(cn('badge badge-lg', 'badge-sm')).toBe('badge badge-sm');
  });

  it('daisy-alert-variant: a later alert tone replaces an earlier one', () => {
    expect(cn('alert alert-warning', 'alert-error')).toBe('alert alert-error');
  });

  it('daisy-input-variant: a later input modifier replaces an earlier one', () => {
    expect(cn('input input-ghost', 'input-error')).toBe('input input-error');
  });

  it('daisy-select-variant: a later select modifier replaces an earlier one', () => {
    expect(cn('select select-ghost', 'select-error')).toBe('select select-error');
  });

  it('daisy-select-size: a later select size replaces an earlier one', () => {
    expect(cn('select select-lg', 'select-sm')).toBe('select select-sm');
  });

  it('daisy-checkbox-variant: a later checkbox colour replaces an earlier one', () => {
    expect(cn('checkbox checkbox-primary', 'checkbox-error')).toBe('checkbox checkbox-error');
  });

  it('daisy-checkbox-size: a later checkbox size replaces an earlier one', () => {
    expect(cn('checkbox checkbox-lg', 'checkbox-sm')).toBe('checkbox checkbox-sm');
  });

  it('daisy-radio-variant: a later radio colour replaces an earlier one', () => {
    expect(cn('radio radio-primary', 'radio-error')).toBe('radio radio-error');
  });

  it('daisy-radio-size: a later radio size replaces an earlier one', () => {
    expect(cn('radio radio-lg', 'radio-sm')).toBe('radio radio-sm');
  });

  it('daisy-card-size: a later card size replaces an earlier one', () => {
    expect(cn('card card-lg', 'card-sm')).toBe('card card-sm');
  });

  it('daisy-table-size: a later table size replaces an earlier one', () => {
    expect(cn('table table-lg', 'table-sm')).toBe('table table-sm');
  });

  it('daisy-loading-type: a later loading type replaces an earlier one', () => {
    expect(cn('loading loading-spinner', 'loading-dots')).toBe('loading loading-dots');
  });

  it('daisy-loading-size: a later loading size replaces an earlier one', () => {
    expect(cn('loading loading-lg', 'loading-md')).toBe('loading loading-md');
  });

  it('falsy and conditional inputs are dropped, like clsx alone', () => {
    const disabled = false;
    const count = 0;
    expect(cn('btn', disabled && 'btn-ghost', null, undefined, count && 'x')).toBe('btn');
  });
});
