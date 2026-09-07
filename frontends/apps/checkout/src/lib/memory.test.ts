/**
 * What the page will and will not remember.
 *
 * The case that matters most is the first one in `parseMemoryRecord`: a
 * value out of the store that is not a Cameroon MSISDN must never reach the
 * form. A payer who does not re-read a prefilled field would send a push to
 * whatever was there.
 */
import { describe, expect, it } from 'vitest';

import {
  MEMORY_MAX_AGE_MS,
  NO_PAGE_MEMORY,
  memoryRecordFor,
  pageMemoryFor,
  parseMemoryRecord,
} from './memory';

const NOW = 1_757_000_000_000;
const GOOD = '237671234567';

describe('parseMemoryRecord', () => {
  it('reads a record this page wrote', () => {
    expect(parseMemoryRecord({ msisdn: GOOD, rail: 'mtn_momo', savedAt: NOW - 1000 }, NOW)).toEqual({
      msisdn: GOOD,
      rail: 'mtn_momo',
      savedAt: NOW - 1000,
    });
  });

  it('answers null for anything that is not an object', () => {
    for (const value of [undefined, null, 'x', 7, true, [], () => undefined]) {
      expect(parseMemoryRecord(value, NOW)).toBeNull();
    }
  });

  it('drops a number that is not a canonical Cameroon MSISDN', () => {
    for (const msisdn of [
      '671234567', // not normalised
      '+237671234567', // still not normalised
      '23767123456', // one digit short
      '2376712345678', // one digit long
      '237571234567', // not a mobile prefix this page produces
      '237 671 234 567',
      '',
      42,
      { toString: () => GOOD },
    ]) {
      const record = parseMemoryRecord({ msisdn, rail: 'mtn_momo', savedAt: NOW }, NOW);
      expect(record?.msisdn ?? null, String(msisdn)).toBeNull();
    }
  });

  it('drops a rail code that is not one', () => {
    for (const rail of ['MTN MoMo', 'mtn momo', '<script>', 'a'.repeat(65), 5]) {
      const record = parseMemoryRecord({ msisdn: GOOD, rail, savedAt: NOW }, NOW);
      expect(record?.rail ?? null, String(rail)).toBeNull();
    }
  });

  it('keeps the good half of a half-bad record', () => {
    expect(parseMemoryRecord({ msisdn: 'nonsense', rail: 'mtn_momo', savedAt: NOW }, NOW)).toEqual({
      msisdn: null,
      rail: 'mtn_momo',
      savedAt: NOW,
    });
  });

  it('treats a record that remembers nothing as no record', () => {
    expect(parseMemoryRecord({ msisdn: null, rail: null, savedAt: NOW }, NOW)).toBeNull();
    expect(parseMemoryRecord({ savedAt: NOW }, NOW)).toBeNull();
  });

  it('forgets a record older than the ninety-day horizon', () => {
    const justInside = NOW - MEMORY_MAX_AGE_MS;
    expect(parseMemoryRecord({ msisdn: GOOD, rail: null, savedAt: justInside }, NOW)).not.toBeNull();
    expect(parseMemoryRecord({ msisdn: GOOD, rail: null, savedAt: justInside - 1 }, NOW)).toBeNull();
  });

  it('refuses a record from the future — a clock moved, and the age is meaningless', () => {
    expect(parseMemoryRecord({ msisdn: GOOD, rail: null, savedAt: NOW + 1 }, NOW)).toBeNull();
  });

  it('refuses a savedAt that is not a finite number', () => {
    for (const savedAt of ['yesterday', null, undefined, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(parseMemoryRecord({ msisdn: GOOD, rail: null, savedAt }, NOW)).toBeNull();
    }
  });

  it('never invents a field the store did not hold', () => {
    const record = parseMemoryRecord(
      { msisdn: GOOD, rail: 'mtn_momo', savedAt: NOW, pin: '1234', token: 'x' },
      NOW,
    );
    // There is no PIN on this page and no credential in this store. A field
    // an older or forged record carried is not carried forward.
    expect(Object.keys(record ?? {}).sort()).toEqual(['msisdn', 'rail', 'savedAt']);
  });
});

describe('memoryRecordFor', () => {
  it('builds the record a submitted MTN form writes', () => {
    expect(memoryRecordFor({ msisdn: GOOD, rail: 'mtn_momo' }, NOW)).toEqual({
      msisdn: GOOD,
      rail: 'mtn_momo',
      savedAt: NOW,
    });
  });

  it('builds a rail-only record for a redirect rail, which collects no number here', () => {
    expect(memoryRecordFor({ msisdn: null, rail: 'orange_money' }, NOW)).toEqual({
      msisdn: null,
      rail: 'orange_money',
      savedAt: NOW,
    });
  });

  it('writes nothing when there is nothing worth writing', () => {
    expect(memoryRecordFor({ msisdn: null, rail: null }, NOW)).toBeNull();
  });

  it('refuses a number the page would not have sent', () => {
    // The client normalises before calling this, so a value that fails here
    // is one the confirm would have refused too — and keeping it would
    // prefill a number that cannot be paid with.
    expect(memoryRecordFor({ msisdn: '+237 671 234 567', rail: null }, NOW)).toBeNull();
  });
});

describe('NO_PAGE_MEMORY', () => {
  it('remembers nothing, in both directions', async () => {
    await NO_PAGE_MEMORY.write({ msisdn: GOOD, rail: 'mtn_momo', savedAt: NOW });
    expect(await NO_PAGE_MEMORY.read()).toBeNull();
    await NO_PAGE_MEMORY.clear();
    expect(await NO_PAGE_MEMORY.read()).toBeNull();
  });
});

describe('pageMemoryFor', () => {
  const store = { ...NO_PAGE_MEMORY };

  it('offers the store when the deployment allows it and the browser has one', () => {
    expect(pageMemoryFor(true, store)).toEqual({ memory: store, offered: true });
  });

  it('offers nothing, and forgets, when config.yaml turned the feature off', () => {
    // Both halves matter: a checkbox that was hidden while the store still
    // wrote would make `page_memory: false` a lie.
    expect(pageMemoryFor(false, store)).toEqual({ memory: NO_PAGE_MEMORY, offered: false });
  });

  it('offers nothing when the browser has no IndexedDB', () => {
    // A private window, or storage walled off. A checkbox that silently
    // forgets is worse than no checkbox.
    expect(pageMemoryFor(true, null)).toEqual({ memory: NO_PAGE_MEMORY, offered: false });
    expect(pageMemoryFor(false, null)).toEqual({ memory: NO_PAGE_MEMORY, offered: false });
  });
});
