/**
 * The IndexedDB adapter, against the stub factory in `src/testing/idb-stub.ts`.
 *
 * What these cases can prove: the wiring — that a write lands under the key
 * a read looks for, that the object store is created before it is used, that
 * every failure path resolves rather than rejecting, and that exactly three
 * fields reach the store. What they cannot prove is that a real browser's
 * IndexedDB behaves like the stub; `docs/status.md` says so rather than this
 * suite implying otherwise.
 */
import { describe, expect, it } from 'vitest';

import { MEMORY_KEY, MEMORY_STORE, indexedDbPageMemory } from './memory-idb';
import { stubIndexedDb } from '../testing/idb-stub';

const NOW = 1_757_000_000_000;
const GOOD = '237671234567';
const RECORD = { msisdn: GOOD, rail: 'mtn_momo', savedAt: NOW };

function memoryOn(db = stubIndexedDb()) {
  return { db, memory: indexedDbPageMemory(db.factory, () => NOW) };
}

describe('indexedDbPageMemory', () => {
  it('reads back what it wrote', async () => {
    const { memory } = memoryOn();
    await memory.write(RECORD);
    expect(await memory.read()).toEqual(RECORD);
  });

  it('creates the object store on first use', async () => {
    const { db, memory } = memoryOn();
    expect(db.stores.has(MEMORY_STORE)).toBe(false);
    await memory.write(RECORD);
    expect(db.stores.get(MEMORY_STORE)?.has(MEMORY_KEY)).toBe(true);
  });

  it('does not wipe the record when the database is opened again', async () => {
    // The stub fires `onupgradeneeded` on every open, which a real factory
    // does only on a version rise — so an upgrade handler that recreated the
    // store unconditionally would lose the record here, on the second read.
    const { memory } = memoryOn();
    await memory.write(RECORD);
    expect(await memory.read()).toEqual(RECORD);
    expect(await memory.read()).toEqual(RECORD);
  });

  it('answers null before anything has been written', async () => {
    const { memory } = memoryOn();
    expect(await memory.read()).toBeNull();
  });

  it('forgets on clear', async () => {
    const { db, memory } = memoryOn();
    await memory.write(RECORD);
    await memory.clear();
    expect(await memory.read()).toBeNull();
    expect(db.stores.get(MEMORY_STORE)?.size).toBe(0);
  });

  it('stores exactly three fields, whatever the caller hands over', async () => {
    const { db, memory } = memoryOn();
    await memory.write({ ...RECORD, pin: '1234' } as never);
    expect(Object.keys(db.stores.get(MEMORY_STORE)?.get(MEMORY_KEY) as object).sort()).toEqual([
      'msisdn',
      'rail',
      'savedAt',
    ]);
  });

  it('applies the horizon on read, so an old record is never handed to the form', async () => {
    const db = stubIndexedDb();
    // Written by a page ninety-one days ago, read by one now.
    const past = indexedDbPageMemory(db.factory, () => NOW - 91 * 24 * 60 * 60 * 1000);
    await past.write({ ...RECORD, savedAt: NOW - 91 * 24 * 60 * 60 * 1000 });
    const present = indexedDbPageMemory(db.factory, () => NOW);
    expect(await present.read()).toBeNull();
    // And it is still on disk — the horizon is a read rule, not a sweep.
    expect(db.stores.get(MEMORY_STORE)?.has(MEMORY_KEY)).toBe(true);
  });

  it('resolves rather than rejecting when the browser refuses to open a database', async () => {
    const { db, memory } = memoryOn();
    db.refuseToOpen();
    // A private window, storage disabled, an origin with no quota. A payer
    // whose browser will not store a number must still be able to pay, so
    // none of these is an error a screen has to render.
    await expect(memory.read()).resolves.toBeNull();
    await expect(memory.write(RECORD)).resolves.toBeUndefined();
    await expect(memory.clear()).resolves.toBeUndefined();
  });

  it('resolves rather than rejecting when a request fails', async () => {
    const { db, memory } = memoryOn();
    db.failEveryRequest();
    await expect(memory.read()).resolves.toBeNull();
    await expect(memory.write(RECORD)).resolves.toBeUndefined();
    await expect(memory.clear()).resolves.toBeUndefined();
  });

  it('resolves rather than rejecting when the store is not there', async () => {
    const { db, memory } = memoryOn();
    await memory.write(RECORD);
    // A database at this name created by something else, with no store of
    // ours in it: `transaction()` throws synchronously.
    db.dropStore(MEMORY_STORE);
    await expect(memory.read()).resolves.toBeNull();
    await expect(memory.write(RECORD)).resolves.toBeUndefined();
  });

  it('drops a stored value that is not a record this page would act on', async () => {
    const { db, memory } = memoryOn();
    await memory.write(RECORD);
    db.stores.get(MEMORY_STORE)?.set(MEMORY_KEY, { msisdn: 'not-a-number', savedAt: NOW });
    expect(await memory.read()).toBeNull();
  });
});
