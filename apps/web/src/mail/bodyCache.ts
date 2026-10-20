// Message body cache (ARCHITECTURE §16.1): memory first, then IndexedDB, keyed by messageId and checked
// against the newest `sanitizerVersion` seen, LRU-capped at 250 MB. With `prefs.cacheBodiesOnDevice` off,
// bodies stay in memory only. Sign-out wipes the database (lib/kv.ts `wipeLocalData`).
//
// Fetching goes through `BodyLoader`: one in-flight thread stream per thread, so opening a thread that is
// already being prefetched never starts a second request.
import type { MessageBody, MessageId, ThreadId, WorkerApi } from "../backend/types.ts";
import { defaultStore, type KvStore } from "../lib/kv.ts";

export const WEB_BODY_CACHE_BYTES = 250 * 1024 * 1024;
const VERSION_KEY = "ak:sanitizer-version";

export interface CachedBody {
  body: MessageBody;
  size: number;
  lastAccess: number;
}

function sizeOf(b: MessageBody): number {
  // UTF-16 code units × 2 is close enough for an LRU budget.
  return ((b.html?.length ?? 0) + (b.text?.length ?? 0)) * 2 + 512;
}

function readVersion(): number {
  try {
    return Number(localStorage.getItem(VERSION_KEY)) || 0;
  } catch {
    return 0;
  }
}

export class BodyCache {
  private mem = new Map<string, MessageBody>();
  private store: KvStore<CachedBody>;
  private persist: boolean;
  private capBytes: number;
  private minVersion = readVersion();
  private writes = 0;

  constructor(opts: { store?: KvStore<CachedBody>; persist?: boolean; capBytes?: number } = {}) {
    this.store = opts.store ?? defaultStore<CachedBody>("bodies");
    this.persist = opts.persist ?? true;
    this.capBytes = opts.capBytes ?? WEB_BODY_CACHE_BYTES;
  }

  /** `prefs.cacheBodiesOnDevice`. Turning it off clears what is on disk. */
  setPersist(persist: boolean) {
    if (this.persist && !persist) void this.store.clear().catch(() => undefined);
    this.persist = persist;
  }

  peek(id: MessageId): MessageBody | undefined {
    return this.mem.get(id);
  }

  async get(id: MessageId): Promise<MessageBody | undefined> {
    const hit = this.mem.get(id);
    if (hit) return hit;
    if (!this.persist) return undefined;
    const row = await this.store.get(id).catch(() => undefined);
    if (!row || row.body.sanitizerVersion < this.minVersion) return undefined;
    this.mem.set(id, row.body);
    void this.store.put(id, { ...row, lastAccess: Date.now() }).catch(() => undefined);
    return row.body;
  }

  async put(body: MessageBody): Promise<void> {
    if (body.sanitizerVersion > this.minVersion) {
      // A newer sanitizer invalidates every older cached body.
      this.minVersion = body.sanitizerVersion;
      try {
        localStorage.setItem(VERSION_KEY, String(body.sanitizerVersion));
      } catch {
        /* ignore */
      }
      for (const [k, b] of this.mem) if (b.sanitizerVersion < this.minVersion) this.mem.delete(k);
    }
    this.mem.set(body.messageId, body);
    if (this.mem.size > 400) this.mem.delete(this.mem.keys().next().value as string);
    if (!this.persist) return;
    await this.store
      .put(body.messageId, { body, size: sizeOf(body), lastAccess: Date.now() })
      .catch(() => undefined);
    if (++this.writes % 25 === 0) await this.evict();
  }

  /** Drops least-recently used entries over the cap, and entries from an older sanitizer. */
  async evict(): Promise<void> {
    const all = await this.store.entries().catch(() => [] as [string, CachedBody][]);
    let total = 0;
    const live: [string, CachedBody][] = [];
    for (const e of all) {
      if (e[1].body.sanitizerVersion < this.minVersion) await this.store.delete(e[0]).catch(() => undefined);
      else {
        live.push(e);
        total += e[1].size;
      }
    }
    live.sort((a, b) => a[1].lastAccess - b[1].lastAccess);
    for (const [k, v] of live) {
      if (total <= this.capBytes) break;
      await this.store.delete(k).catch(() => undefined);
      this.mem.delete(k);
      total -= v.size;
    }
  }

  clearMemory() {
    this.mem.clear();
  }
}

export type BodyState =
  | { status: "loading" }
  | { status: "ready"; body: MessageBody }
  | { status: "error"; error: unknown };

/** Loads and prefetches thread bodies through the worker, deduping concurrent requests. */
export class BodyLoader {
  private inflight = new Map<string, Promise<void>>();
  private failedAt = new Map<string, { at: number; error: unknown }>();
  private completed = new Set<string>();
  private listeners = new Set<() => void>();
  readonly cache: BodyCache;
  private worker: WorkerApi;

  constructor(worker: WorkerApi, cache: BodyCache = new BodyCache()) {
    this.worker = worker;
    this.cache = cache;
  }

  subscribe(cb: () => void) {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }
  private emit() {
    for (const l of this.listeners) l();
  }

  /** The last error for a thread, if its fetch failed in the last minute (shown instead of retrying in a loop). */
  errorFor(threadId: ThreadId): unknown {
    const f = this.failedAt.get(threadId);
    return f && Date.now() - f.at < 60_000 ? f.error : undefined;
  }

  /** Ensures every message in `messageIds` is cached, streaming the thread once if any is missing. */
  async loadThread(
    threadId: ThreadId,
    messageIds: readonly MessageId[],
    opts: { force?: boolean } = {},
  ): Promise<void> {
    if (opts.force) this.failedAt.delete(threadId);
    else if (this.errorFor(threadId) !== undefined) return;
    const missing: MessageId[] = [];
    for (const id of messageIds) if (!(await this.cache.get(id))) missing.push(id);
    if (missing.length === 0) {
      this.emit();
      return;
    }
    const running = this.inflight.get(threadId);
    if (running) return running;
    const p = (async () => {
      try {
        for await (const body of this.worker.threadBodies(threadId)) {
          await this.cache.put(body);
          this.emit();
        }
        this.failedAt.delete(threadId);
        this.completed.add(threadId);
      } catch (error) {
        this.failedAt.set(threadId, { at: Date.now(), error });
      } finally {
        this.inflight.delete(threadId);
        this.emit();
      }
    })();
    this.inflight.set(threadId, p);
    return p;
  }

  /** The thread's stream finished without error (a message still missing then is gone at the provider). */
  finished(threadId: ThreadId): boolean {
    return this.completed.has(threadId);
  }

  isLoading(threadId: ThreadId): boolean {
    return this.inflight.has(threadId);
  }
}
