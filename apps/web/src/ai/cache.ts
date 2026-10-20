// On-device AI output cache (ARCHITECTURE §16.2): keyed by (feature, promptVersion, model, SHA-256 of the
// input vars). Kept in this browser's IndexedDB only, never synced (§12.7 rule 5); sign-out deletes it with
// the rest of the local data. At most MAX_ENTRIES, oldest dropped first.
import { defaultStore, type KvStore } from "../lib/kv.ts";

export interface CachedAi {
  text: string | null;
  object: unknown;
  at: number;
}

const MAX_ENTRIES = 300;
let store: KvStore<CachedAi> | null = null;
const db = () => {
  store ??= defaultStore<CachedAi>("ai");
  return store;
};

/** Test seam. */
export function setAiCacheStore(s: KvStore<CachedAi> | null): void {
  store = s;
}

export async function aiCacheKey(
  feature: string,
  promptVersion: number,
  model: string,
  vars: unknown,
): Promise<string> {
  const data = new TextEncoder().encode(JSON.stringify(vars));
  const digest = await crypto.subtle.digest("SHA-256", data);
  const hex = Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
  return `${feature}|${promptVersion}|${model}|${hex}`;
}

export async function aiCacheGet(
  feature: string,
  promptVersion: number,
  model: string,
  vars: unknown,
): Promise<CachedAi | undefined> {
  try {
    return await db().get(await aiCacheKey(feature, promptVersion, model, vars));
  } catch {
    return undefined;
  }
}

export async function aiCachePut(
  feature: string,
  promptVersion: number,
  model: string,
  vars: unknown,
  value: { text: string | null; object: unknown },
): Promise<void> {
  try {
    const s = db();
    await s.put(await aiCacheKey(feature, promptVersion, model, vars), { ...value, at: Date.now() });
    const all = await s.entries();
    if (all.length > MAX_ENTRIES) {
      const old = all.sort((a, b) => a[1].at - b[1].at).slice(0, all.length - MAX_ENTRIES);
      for (const [k] of old) await s.delete(k);
    }
  } catch {
    /* the cache is best effort */
  }
}
