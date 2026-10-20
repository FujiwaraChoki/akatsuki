// Body cache policy (ARCHITECTURE §16.1): keyed by (messageId, sanitizerVersion), 300 MB LRU on mobile, memory
// only when `prefs.cacheBodiesOnDevice` is false, wiped on sign-out. The storage lives in lib/bodyCache.ts.

export const MOBILE_BODY_CACHE_BYTES = 300 * 1024 * 1024;

export interface CacheEntryMeta {
  key: string;
  size: number;
  lastAccess: number;
}

/** Keys to evict (least recently used first) so the total fits under `capBytes`. */
export function planEviction(entries: readonly CacheEntryMeta[], capBytes: number): string[] {
  let total = entries.reduce((s, e) => s + e.size, 0);
  if (total <= capBytes) return [];
  const out: string[] = [];
  for (const e of [...entries].sort((a, b) => a.lastAccess - b.lastAccess)) {
    if (total <= capBytes) break;
    out.push(e.key);
    total -= e.size;
  }
  return out;
}

/** A cached body is usable when it was produced by the newest sanitizer the device has seen. */
export function isFresh(entryVersion: number, latestKnownVersion: number): boolean {
  return entryVersion >= latestKnownVersion;
}

/** Approximate stored size of a body (UTF-16 code units × 2 is what SQLite TEXT costs at most). */
export function bodySize(body: { html: string | null; text: string | null }): number {
  return ((body.html?.length ?? 0) + (body.text?.length ?? 0)) * 2 + 256;
}

/** Small in-memory LRU for bodies when on-device caching is off (and as a hot layer in front of SQLite). */
export class MemoryLru<V> {
  private readonly map = new Map<string, { value: V; size: number }>();
  private total = 0;
  constructor(private readonly capBytes: number) {}
  get(key: string): V | undefined {
    const hit = this.map.get(key);
    if (!hit) return undefined;
    this.map.delete(key);
    this.map.set(key, hit);
    return hit.value;
  }
  set(key: string, value: V, size: number): void {
    const old = this.map.get(key);
    if (old) {
      this.total -= old.size;
      this.map.delete(key);
    }
    this.map.set(key, { value, size });
    this.total += size;
    for (const [k, v] of this.map) {
      if (this.total <= this.capBytes) break;
      this.map.delete(k);
      this.total -= v.size;
    }
  }
  clear(): void {
    this.map.clear();
    this.total = 0;
  }
  get bytes(): number {
    return this.total;
  }
  get size(): number {
    return this.map.size;
  }
}
