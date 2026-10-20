// A tiny async key-value store over IndexedDB, with an in-memory fallback (private windows, tests). Used by the
// body cache and the durable outbox (§16.1). One database, one object store per use.

export interface KvStore<T> {
  get(key: string): Promise<T | undefined>;
  put(key: string, value: T): Promise<void>;
  delete(key: string): Promise<void>;
  /** All entries, ordered by key. */
  entries(): Promise<[string, T][]>;
  clear(): Promise<void>;
}

const DB_NAME = "akatsuki";
// 2: part 2 adds the device's compose store (§10.1 "the device's local compose store") and the AI output
// cache (§16.2, keyed by feature, input hash, prompt version and model).
const DB_VERSION = 2;
export const STORES = ["bodies", "outbox", "drafts", "ai"] as const;
export type StoreName = (typeof STORES)[number];

let dbPromise: Promise<IDBDatabase> | null = null;

function openDb(): Promise<IDBDatabase> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      for (const name of STORES)
        if (!req.result.objectStoreNames.contains(name)) req.result.createObjectStore(name);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
    req.onblocked = () => reject(new Error("IndexedDB blocked"));
  });
  dbPromise.catch(() => {
    dbPromise = null;
  });
  return dbPromise;
}

function wrap<R>(r: IDBRequest<R>): Promise<R> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

export function idbStore<T>(name: StoreName): KvStore<T> {
  const tx = async (mode: IDBTransactionMode) => (await openDb()).transaction(name, mode).objectStore(name);
  return {
    async get(key) {
      return (await wrap((await tx("readonly")).get(key))) as T | undefined;
    },
    async put(key, value) {
      await wrap((await tx("readwrite")).put(value, key));
    },
    async delete(key) {
      await wrap((await tx("readwrite")).delete(key));
    },
    async entries() {
      const store = await tx("readonly");
      const [keys, values] = await Promise.all([wrap(store.getAllKeys()), wrap(store.getAll())]);
      return keys.map((k, i) => [String(k), values[i] as T]);
    },
    async clear() {
      await wrap((await tx("readwrite")).clear());
    },
  };
}

export function memoryStore<T>(): KvStore<T> {
  const m = new Map<string, T>();
  return {
    get: async (k) => m.get(k),
    put: async (k, v) => {
      m.set(k, v);
    },
    delete: async (k) => {
      m.delete(k);
    },
    entries: async () => [...m.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
    clear: async () => m.clear(),
  };
}

/** IndexedDB when available, else memory. */
export function defaultStore<T>(name: StoreName): KvStore<T> {
  return typeof indexedDB !== "undefined" ? idbStore<T>(name) : memoryStore<T>();
}

/** The separate database holding BYO AI keys (`src/ai/keyStore.ts`, §12.2). */
export const KEYS_DB_NAME = "akatsuki-keys";

/**
 * Deletes the whole database (sign-out wipes bodies, the outbox, local drafts and the AI cache, §16.1) and the
 * BYO key database (D-285).
 */
export async function wipeLocalData(): Promise<void> {
  try {
    localStorage.removeItem("ak:voice-profile");
  } catch {
    /* ignore */
  }
  if (typeof indexedDB === "undefined") return;
  await new Promise<void>((resolve) => {
    const req = indexedDB.deleteDatabase(KEYS_DB_NAME);
    req.onsuccess = req.onerror = req.onblocked = () => resolve();
  });
  const db = await dbPromise?.catch(() => null);
  db?.close();
  dbPromise = null;
  await new Promise<void>((resolve) => {
    const req = indexedDB.deleteDatabase(DB_NAME);
    req.onsuccess = req.onerror = req.onblocked = () => resolve();
  });
}
