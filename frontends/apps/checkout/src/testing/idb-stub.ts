/**
 * A minimal in-memory `IDBFactory`, for the tests only.
 *
 * jsdom implements no IndexedDB at all, so without this `memory-idb.ts`
 * would be a shipping file nothing had ever executed. It is a **test
 * double, and it lives under `src/testing/` for that reason**:
 * `no-runtime-imports.test.ts` and the shared ESLint config both fail if a
 * shipping file imports anything from this directory.
 *
 * What it models is the object graph the adapter actually walks — `open` →
 * `onupgradeneeded` → `createObjectStore` → `transaction` →
 * `objectStore(name)` → `get`/`put`/`delete`, each request settling
 * asynchronously — so the wiring bugs it can catch are real ones: an
 * upgrade that never creates the store, a store name that disagrees between
 * write and read, a request whose `onerror` is never attached, a
 * transaction opened `readonly` for a write.
 *
 * What it does **not** model: versions above 1 and their upgrade paths,
 * `onblocked`, indexes, cursors, key ranges, structured cloning, quotas, and
 * transaction lifetime (a real transaction becomes inactive at the end of
 * the microtask that created it). A test that needs any of those is a test
 * this stub cannot honestly answer, and `docs/status.md` says so rather than
 * this file pretending otherwise.
 */

type Listener = ((event: unknown) => void) | null;

class StubRequest<T> {
  onsuccess: Listener = null;
  onerror: Listener = null;
  result!: T;

  settle(produce: () => T, fail: boolean): void {
    // A real request fires on a later task, and code that assumes otherwise
    // (reading `.result` straight after the call) is the bug this delay
    // exposes.
    queueMicrotask(() => {
      if (fail) {
        this.onerror?.({ target: this });
        return;
      }
      this.result = produce();
      this.onsuccess?.({ target: this });
    });
  }
}

class StubObjectStore {
  constructor(
    private readonly rows: Map<string, unknown>,
    private readonly mode: IDBTransactionMode,
    private readonly failing: boolean,
  ) {}

  get(key: string): StubRequest<unknown> {
    const request = new StubRequest<unknown>();
    request.settle(() => this.rows.get(key), this.failing);
    return request;
  }

  put(value: unknown, key: string): StubRequest<string> {
    if (this.mode === "readonly") {
      throw new Error("ReadOnlyError: a write on a readonly transaction");
    }
    const request = new StubRequest<string>();
    request.settle(() => {
      // Structured-clone-ish: the stored value must not alias the caller's,
      // or a test could pass because both sides hold one object.
      this.rows.set(key, JSON.parse(JSON.stringify(value)) as unknown);
      return key;
    }, this.failing);
    return request;
  }

  delete(key: string): StubRequest<undefined> {
    if (this.mode === "readonly") {
      throw new Error("ReadOnlyError: a delete on a readonly transaction");
    }
    const request = new StubRequest<undefined>();
    request.settle(() => {
      this.rows.delete(key);
      return undefined;
    }, this.failing);
    return request;
  }
}

/** What the stub factory hands back, plus the hooks a test drives it with. */
export interface StubIndexedDb {
  factory: IDBFactory;
  /** The rows, by store name, so a test can assert on what was written. */
  stores: Map<string, Map<string, unknown>>;
  /** Every `open` fails from here on — a private window, or storage disabled. */
  refuseToOpen(): void;
  /** Every request fails from here on — a quota, or an aborted transaction. */
  failEveryRequest(): void;
  /**
   * Removes a store and refuses to recreate it, so `transaction` throws —
   * the shape a database created at this name by something else has.
   */
  dropStore(name: string): void;
}

/**
 * A fresh in-memory IndexedDB.
 *
 * `stores` starts empty: the adapter's own `onupgradeneeded` is what creates
 * the object store, so a test that never opens the database sees nothing,
 * and an adapter that forgot the upgrade fails rather than silently getting
 * an empty store.
 */
export function stubIndexedDb(): StubIndexedDb {
  const stores = new Map<string, Map<string, unknown>>();
  let openRefused = false;
  let requestsFail = false;
  const blocked = new Set<string>();

  const factory = {
    open(_name: string, _version?: number) {
      const request: Record<string, unknown> = {
        onupgradeneeded: null,
        onsuccess: null,
        onerror: null,
        onblocked: null,
      };
      const db = {
        objectStoreNames: {
          contains: (name: string) => stores.has(name),
        },
        createObjectStore(name: string) {
          if (blocked.has(name)) {
            return;
          }
          stores.set(name, new Map<string, unknown>());
        },
        transaction(name: string, mode: IDBTransactionMode = "readonly") {
          const rows = stores.get(name);
          if (rows === undefined) {
            throw new Error(`NotFoundError: no object store named ${name}`);
          }
          return {
            objectStore: () => new StubObjectStore(rows, mode, requestsFail),
          };
        },
        close() {
          /* nothing to release */
        },
      };
      (request as { result?: unknown }).result = db;
      queueMicrotask(() => {
        if (openRefused) {
          (request["onerror"] as Listener)?.({ target: request });
          return;
        }
        // `onupgradeneeded` fires on EVERY open here, where a real factory
        // fires it only when the version rises. That is deliberate: it makes
        // the adapter's `objectStoreNames.contains` guard load-bearing, so an
        // upgrade handler that recreated the store on every open — wiping the
        // record a payer asked to keep — fails a test instead of shipping.
        (request["onupgradeneeded"] as Listener)?.({ target: request });
        (request["onsuccess"] as Listener)?.({ target: request });
      });
      return request;
    },
  };

  return {
    factory: factory as unknown as IDBFactory,
    stores,
    refuseToOpen() {
      openRefused = true;
    },
    failEveryRequest() {
      requestsFail = true;
    },
    dropStore(name: string) {
      stores.delete(name);
      blocked.add(name);
    },
  };
}
