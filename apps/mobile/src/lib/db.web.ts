// Web preview: the same API as db.ts, held in memory (no SQLite in the browser build).
import { MemoryLru, MOBILE_BODY_CACHE_BYTES } from "../vm/lru";
import { memoryOutboxStorage } from "../vm/outbox";

const kv = new Map<string, string>();
const bodies = new MemoryLru<{ v: number; json: string }>(MOBILE_BODY_CACHE_BYTES / 6);

export async function kvGet<T>(key: string): Promise<T | null> {
  const v = kv.get(key);
  return v === undefined ? null : (JSON.parse(v) as T);
}
export async function kvSet(key: string, value: unknown): Promise<void> {
  kv.set(key, JSON.stringify(value));
}
export async function kvDelete(key: string): Promise<void> {
  kv.delete(key);
}
export async function bodyGet<T>(messageId: string, minVersion: number): Promise<T | null> {
  const hit = bodies.get(messageId);
  return hit && hit.v >= minVersion ? (JSON.parse(hit.json) as T) : null;
}
export async function bodyPut(
  messageId: string,
  sanitizerVersion: number,
  body: unknown,
  size: number,
): Promise<void> {
  bodies.set(messageId, { v: sanitizerVersion, json: JSON.stringify(body) }, size);
}
export async function bodyEvict(): Promise<number> {
  return 0;
}
export async function bodyIds(ids: readonly string[], minVersion: number): Promise<Set<string>> {
  return new Set(ids.filter((id) => (bodies.get(id)?.v ?? -1) >= minVersion));
}
export const sqliteOutboxStorage = memoryOutboxStorage();
export async function wipeLocalData(): Promise<void> {
  kv.clear();
  bodies.clear();
}
