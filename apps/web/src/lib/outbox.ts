// Durable outbox (ARCHITECTURE §16.1): before a thread mutation is sent, `{clientOpId, fn, args}` is written
// to IndexedDB and removed once the mutation resolves (or fails permanently). On start the entries are
// replayed in order; `clientOpId` dedupe on the server makes a replay of an already-applied gesture a no-op.
import type { Mutations, OutboxFn } from "../backend/types.ts";
import { defaultStore, type KvStore } from "./kv.ts";

export interface OutboxEntry {
  clientOpId: string;
  fn: OutboxFn;
  args: unknown;
  at: number;
}

/** Errors that will never succeed on replay; everything else stays queued. */
const PERMANENT = new Set([
  "not_found",
  "invalid_argument",
  "invalid_action",
  "unsupported",
  "not_entitled",
  "unauthenticated",
]);

export function errorCodeOf(err: unknown): string | undefined {
  const data = (err as { data?: unknown })?.data;
  if (data && typeof data === "object" && typeof (data as { code?: unknown }).code === "string")
    return (data as { code: string }).code;
  const code = (err as { code?: unknown })?.code;
  return typeof code === "string" ? code : undefined;
}

export class Outbox {
  private store: KvStore<OutboxEntry>;
  private replaying = false;

  constructor(store: KvStore<OutboxEntry> = defaultStore("outbox")) {
    this.store = store;
  }

  /** Keys sort by time, then clientOpId (UUIDv7 already sorts by time). */
  private key(e: Pick<OutboxEntry, "at" | "clientOpId">) {
    return `${String(e.at).padStart(15, "0")}:${e.clientOpId}`;
  }

  /** Records the call, runs it, and forgets it when it settles (unless it failed transiently). */
  async run<R>(fn: OutboxFn, clientOpId: string, args: unknown, call: () => Promise<R>): Promise<R> {
    const entry: OutboxEntry = { clientOpId, fn, args, at: Date.now() };
    const key = this.key(entry);
    // Never block the gesture on IndexedDB: the optimistic update has already rendered.
    const written = this.store.put(key, entry).catch(() => undefined);
    try {
      const r = await call();
      await written;
      await this.store.delete(key).catch(() => undefined);
      return r;
    } catch (err) {
      const code = errorCodeOf(err);
      if (code === undefined || PERMANENT.has(code)) {
        await written;
        await this.store.delete(key).catch(() => undefined);
      }
      throw err;
    }
  }

  async pending(): Promise<OutboxEntry[]> {
    return (await this.store.entries().catch(() => [])).map(([, v]) => v);
  }

  /** Replays every entry in order through `m`. Returns how many were sent. */
  async replay(m: Pick<Mutations, OutboxFn>): Promise<number> {
    if (this.replaying) return 0;
    this.replaying = true;
    let sent = 0;
    try {
      for (const [key, e] of await this.store.entries().catch(() => [] as [string, OutboxEntry][])) {
        try {
          // biome-ignore lint/suspicious/noExplicitAny: args were recorded for exactly this function
          await (m[e.fn] as (a: any) => Promise<unknown>)(e.args);
          await this.store.delete(key);
          sent++;
        } catch (err) {
          const code = errorCodeOf(err);
          if (code === undefined || PERMANENT.has(code)) await this.store.delete(key).catch(() => undefined);
          else break; // offline again: keep order, try next start
        }
      }
    } finally {
      this.replaying = false;
    }
    return sent;
  }

  clear(): Promise<void> {
    return this.store.clear();
  }
}
