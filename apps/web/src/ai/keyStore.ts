// BYO AI keys on the web (ARCHITECTURE §12.2, D-009): keys never touch Convex, the worker or apps/api.
//
// - By default a key lives in memory for this tab's session only.
// - "Remember on this browser" stores it in IndexedDB (`akatsuki-keys`) as AES-GCM ciphertext under a
//   NON-EXTRACTABLE WebCrypto key generated once per browser. The CryptoKey object itself is stored (structured
//   clone keeps it non-extractable), so script can use it to decrypt on this origin but can never export the raw
//   key bytes; a copied database file holds ciphertext and an opaque key handle only.
// - Sign-out deletes the database (`wipeLocalData`, D-285).
//
// `get()` is synchronous (the AI SDK registry asks per call), so remembered keys are decrypted into memory by
// `load()` once per session.
import { KEYS_DB_NAME } from "../lib/kv.ts";

export interface SealedKey {
  iv: Uint8Array;
  ciphertext: ArrayBuffer;
  savedAt: number;
}

/** Storage for the wrapping key and the sealed entries. IndexedDB in browsers, memory in tests. */
export interface KeyVault {
  getMaster(): Promise<CryptoKey | undefined>;
  putMaster(key: CryptoKey): Promise<void>;
  get(profileId: string): Promise<SealedKey | undefined>;
  put(profileId: string, value: SealedKey): Promise<void>;
  delete(profileId: string): Promise<void>;
  keys(): Promise<string[]>;
}

const STORE = "keys";
const MASTER = "__master__";

function openKeysDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(KEYS_DB_NAME, 1);
    req.onupgradeneeded = () => {
      if (!req.result.objectStoreNames.contains(STORE)) req.result.createObjectStore(STORE);
    };
    req.onsuccess = () => {
      const db = req.result;
      // Let sign-out delete the database while this tab holds it open.
      db.onversionchange = () => db.close();
      resolve(db);
    };
    req.onerror = () => reject(req.error);
    req.onblocked = () => reject(new Error("IndexedDB blocked"));
  });
}

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

export function indexedDbVault(): KeyVault {
  const store = async (mode: IDBTransactionMode) => {
    const db = await openKeysDb();
    const s = db.transaction(STORE, mode).objectStore(STORE);
    return { s, done: () => db.close() };
  };
  const run = async <T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> => {
    const { s, done } = await store(mode);
    try {
      return await req(f(s));
    } finally {
      done();
    }
  };
  return {
    getMaster: () => run("readonly", (s) => s.get(MASTER)) as Promise<CryptoKey | undefined>,
    putMaster: async (key) => {
      await run("readwrite", (s) => s.put(key, MASTER));
    },
    get: (id) => run("readonly", (s) => s.get(`k:${id}`)) as Promise<SealedKey | undefined>,
    put: async (id, value) => {
      await run("readwrite", (s) => s.put(value, `k:${id}`));
    },
    delete: async (id) => {
      await run("readwrite", (s) => s.delete(`k:${id}`));
    },
    keys: async () =>
      ((await run("readonly", (s) => s.getAllKeys())) as IDBValidKey[])
        .map(String)
        .filter((k) => k.startsWith("k:"))
        .map((k) => k.slice(2)),
  };
}

export function memoryVault(): KeyVault & { raw: Map<string, unknown> } {
  const raw = new Map<string, unknown>();
  return {
    raw,
    getMaster: async () => raw.get(MASTER) as CryptoKey | undefined,
    putMaster: async (k) => {
      raw.set(MASTER, k);
    },
    get: async (id) => raw.get(`k:${id}`) as SealedKey | undefined,
    put: async (id, v) => {
      raw.set(`k:${id}`, v);
    },
    delete: async (id) => {
      raw.delete(`k:${id}`);
    },
    keys: async () => [...raw.keys()].filter((k) => k.startsWith("k:")).map((k) => k.slice(2)),
  };
}

const enc = new TextEncoder();
const dec = new TextDecoder();

export class KeyStore {
  private memory = new Map<string, string>();
  private remembered = new Set<string>();
  private listeners = new Set<() => void>();
  private loaded: Promise<void> | null = null;
  version = 0;

  constructor(
    private vault: KeyVault | null,
    private subtle: SubtleCrypto | null = globalThis.crypto?.subtle ?? null,
  ) {}

  subscribe = (cb: () => void) => {
    this.listeners.add(cb);
    return () => {
      this.listeners.delete(cb);
    };
  };
  getVersion = () => this.version;
  private emit() {
    this.version++;
    for (const l of [...this.listeners]) l();
  }

  /** True when this browser can remember keys (IndexedDB and WebCrypto available). */
  get canRemember(): boolean {
    return this.vault !== null && this.subtle !== null;
  }

  private async master(): Promise<CryptoKey> {
    if (!this.vault || !this.subtle) throw new Error("no key storage on this browser");
    const existing = await this.vault.getMaster();
    if (existing) return existing;
    // extractable: false — the raw key can never be read back, only used for encrypt/decrypt here.
    const key = await this.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, [
      "encrypt",
      "decrypt",
    ]);
    await this.vault.putMaster(key);
    return key;
  }

  /** Decrypts every remembered key into memory (once per session). */
  load(): Promise<void> {
    if (!this.loaded)
      this.loaded = (async () => {
        if (!this.vault || !this.subtle) return;
        try {
          const ids = await this.vault.keys();
          if (ids.length === 0) return;
          const master = await this.master();
          for (const id of ids) {
            const sealed = await this.vault.get(id);
            if (!sealed) continue;
            try {
              const plain = await this.subtle.decrypt(
                { name: "AES-GCM", iv: sealed.iv as BufferSource },
                master,
                sealed.ciphertext,
              );
              this.memory.set(id, dec.decode(plain));
              this.remembered.add(id);
            } catch {
              await this.vault.delete(id).catch(() => undefined);
            }
          }
        } catch {
          /* storage unavailable: session-only keys still work */
        } finally {
          this.emit();
        }
      })();
    return this.loaded;
  }

  get(profileId: string): string | undefined {
    return this.memory.get(profileId);
  }

  has(profileId: string): boolean {
    return this.memory.has(profileId);
  }

  isRemembered(profileId: string): boolean {
    return this.remembered.has(profileId);
  }

  /** Holds `key` for this session, and seals it into IndexedDB when `remember` is set. */
  async set(profileId: string, key: string, remember: boolean): Promise<void> {
    const value = key.trim();
    if (!value) return this.remove(profileId);
    this.memory.set(profileId, value);
    if (remember && this.vault && this.subtle) {
      const master = await this.master();
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const ciphertext = await this.subtle.encrypt(
        { name: "AES-GCM", iv: iv as BufferSource },
        master,
        enc.encode(value),
      );
      await this.vault.put(profileId, { iv, ciphertext, savedAt: Date.now() });
      this.remembered.add(profileId);
    } else {
      await this.vault?.delete(profileId).catch(() => undefined);
      this.remembered.delete(profileId);
    }
    this.emit();
  }

  async remove(profileId: string): Promise<void> {
    this.memory.delete(profileId);
    this.remembered.delete(profileId);
    await this.vault?.delete(profileId).catch(() => undefined);
    this.emit();
  }

  /** Forgets everything held in memory (sign-out also deletes the database). */
  clearMemory(): void {
    this.memory.clear();
    this.remembered.clear();
    this.loaded = null;
    this.emit();
  }
}

let shared: KeyStore | null = null;

/** The browser's key store (IndexedDB when available). */
export function keyStore(): KeyStore {
  if (!shared) shared = new KeyStore(typeof indexedDB !== "undefined" ? indexedDbVault() : null);
  return shared;
}

/** Test seam. */
export function setKeyStore(store: KeyStore | null): void {
  shared = store;
}
