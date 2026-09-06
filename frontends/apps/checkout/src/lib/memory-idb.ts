/**
 * The IndexedDB half of {@link PageMemory}.
 *
 * IndexedDB rather than `localStorage` for one reason that matters and one
 * that will: it is the store a browser treats as evictable data rather than
 * as a synchronous main-thread API, and it is the store this page would keep
 * a recalled credential in if the maintainer ever adds a field to recall one
 * into (`memory.ts` says why there is none today).
 *
 * The whole surface is one database, one object store and one key. There is
 * no index, no cursor and no migration path beyond `onupgradeneeded`
 * creating the store, because the record is a convenience: a version bump
 * that could not upgrade would be answered by forgetting, not by repairing.
 *
 * **Every operation resolves.** A refused `open` (a private window, storage
 * disabled, a quota), a store that is not there, a transaction that aborts —
 * each answers `null`/`undefined` rather than rejecting. A payer whose
 * browser will not store a phone number must still be able to pay, and a
 * rejected promise here would surface as a screen they cannot act on.
 *
 * The factory is a parameter rather than the global so the adapter can be
 * driven by `src/testing/idb-stub.ts` in a jsdom test, which has no
 * IndexedDB of its own.
 */
import type { PageMemory, PageMemoryRecord } from './memory';
import { parseMemoryRecord } from './memory';

export const MEMORY_DB_NAME = 'vpay-checkout';
export const MEMORY_DB_VERSION = 1;
export const MEMORY_STORE = 'page-memory';
/** One record per device. There is no per-merchant or per-session key: the number is the payer's, not the payment's. */
export const MEMORY_KEY = 'last';

/** An `IDBRequest` as a promise that resolves to `null` rather than rejecting. */
function settle<T>(request: IDBRequest<T>): Promise<T | null> {
  return new Promise((resolve) => {
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      resolve(null);
    };
  });
}

/** Opens the database, creating the store on first use. `null` if the browser refuses. */
function open(factory: IDBFactory): Promise<IDBDatabase | null> {
  return new Promise((resolve) => {
    let request: IDBOpenDBRequest;
    try {
      request = factory.open(MEMORY_DB_NAME, MEMORY_DB_VERSION);
    } catch {
      resolve(null);
      return;
    }
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(MEMORY_STORE)) {
        db.createObjectStore(MEMORY_STORE);
      }
    };
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      resolve(null);
    };
    request.onblocked = () => {
      resolve(null);
    };
  });
}

/**
 * Runs one operation against the store and closes the connection.
 *
 * A connection per operation, not one held open: this page performs at most
 * three of them in its whole life, and a held connection is the thing that
 * blocks a later version's `onupgradeneeded` in another tab.
 */
async function withStore<T>(
  factory: IDBFactory,
  mode: IDBTransactionMode,
  run: (store: IDBObjectStore) => Promise<T | null>,
): Promise<T | null> {
  const db = await open(factory);
  if (db === null) {
    return null;
  }
  try {
    const transaction = db.transaction(MEMORY_STORE, mode);
    return await run(transaction.objectStore(MEMORY_STORE));
  } catch {
    // `transaction` throws when the store is missing (a database created by
    // something else at the same name) and when the connection is closing.
    return null;
  } finally {
    db.close();
  }
}

/**
 * The adapter.
 *
 * `now` is injected for the same reason `parseMemoryRecord` takes it: the
 * ninety-day horizon is a test, not a wait.
 */
export function indexedDbPageMemory(
  factory: IDBFactory,
  now: () => number = () => Date.now(),
): PageMemory {
  return {
    async read(): Promise<PageMemoryRecord | null> {
      const raw = await withStore(factory, 'readonly', (store) =>
        settle<unknown>(store.get(MEMORY_KEY) as IDBRequest<unknown>),
      );
      return parseMemoryRecord(raw, now());
    },
    async write(record: PageMemoryRecord): Promise<void> {
      // Written as a plain object literal rather than by passing `record`
      // through: whatever the caller hands over, exactly three members are
      // stored, so a field added to the type upstream cannot reach the disk
      // without someone editing this line.
      await withStore(factory, 'readwrite', (store) =>
        settle(
          store.put({ msisdn: record.msisdn, rail: record.rail, savedAt: record.savedAt }, MEMORY_KEY),
        ),
      );
    },
    async clear(): Promise<void> {
      await withStore(factory, 'readwrite', (store) => settle(store.delete(MEMORY_KEY)));
    },
  };
}

/**
 * The page's memory for this browser: the IndexedDB adapter, or nothing.
 *
 * `undefined` in a server render and in a browser without IndexedDB. The
 * caller pairs it with `NO_PAGE_MEMORY`.
 */
export function browserPageMemory(): PageMemory | null {
  if (typeof indexedDB === 'undefined') {
    return null;
  }
  return indexedDbPageMemory(indexedDB);
}
