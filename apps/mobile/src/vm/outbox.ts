// Durable mutation outbox (ARCHITECTURE §16.1): before a thread mutation is called, `{clientOpId, fn, args}` is
// written to device storage (expo-sqlite on mobile) and removed when the mutation resolves. On start the app
// replays what is left, in order. `clientOpId` deduplication on the server makes a replay safe.

export type OutboxFn = "mail.apply" | "mail.snooze" | "mail.unsnooze" | "mail.undo" | "mail.setReminder";

export interface OutboxEntry {
  clientOpId: string;
  fn: OutboxFn;
  args: Record<string, unknown>;
  createdAt: number;
  attempts: number;
}

/** Storage behind the outbox (SQLite on device, a Map in tests). Keys are clientOpIds. */
export interface OutboxStorage {
  put(entry: OutboxEntry): Promise<void>;
  remove(clientOpId: string): Promise<void>;
  all(): Promise<OutboxEntry[]>;
}

/** Entries older than this are dropped instead of replayed (undo windows and snoozes are long past). */
export const OUTBOX_MAX_AGE_MS = 7 * 86_400_000;
export const OUTBOX_MAX_ATTEMPTS = 5;

export interface Outbox {
  /** Records the call, runs it, and forgets it once the server answered (success or a permanent error). */
  run<T>(fn: OutboxFn, clientOpId: string, args: Record<string, unknown>, call: () => Promise<T>): Promise<T>;
  /** Replays leftovers in creation order. `call` returns false for a transient failure (kept for later). */
  replay(
    call: (e: OutboxEntry) => Promise<"done" | "retry">,
    now: number,
  ): Promise<{ replayed: number; dropped: number; kept: number }>;
  pending(): Promise<OutboxEntry[]>;
}

/** Errors worth replaying later: no connection, or a server that said "retry". */
export function isTransient(err: unknown): boolean {
  if (!err || typeof err !== "object") return false;
  const e = err as { retryable?: unknown; data?: { retryable?: unknown }; message?: unknown };
  if (e.retryable === true || e.data?.retryable === true) return true;
  const msg = typeof e.message === "string" ? e.message : "";
  return /network|connection|timed? ?out|offline|fetch failed/i.test(msg);
}

export function createOutbox(storage: OutboxStorage, now: () => number = Date.now): Outbox {
  return {
    async run(fn, clientOpId, args, call) {
      const entry: OutboxEntry = { clientOpId, fn, args, createdAt: now(), attempts: 1 };
      await storage.put(entry);
      try {
        const result = await call();
        await storage.remove(clientOpId);
        return result;
      } catch (err) {
        if (!isTransient(err)) await storage.remove(clientOpId);
        throw err;
      }
    },
    async replay(call, at) {
      const entries = (await storage.all()).sort((a, b) => a.createdAt - b.createdAt);
      let replayed = 0;
      let dropped = 0;
      let kept = 0;
      for (const e of entries) {
        if (at - e.createdAt > OUTBOX_MAX_AGE_MS || e.attempts >= OUTBOX_MAX_ATTEMPTS) {
          await storage.remove(e.clientOpId);
          dropped++;
          continue;
        }
        let outcome: "done" | "retry";
        try {
          outcome = await call(e);
        } catch (err) {
          outcome = isTransient(err) ? "retry" : "done";
        }
        if (outcome === "done") {
          await storage.remove(e.clientOpId);
          replayed++;
        } else {
          await storage.put({ ...e, attempts: e.attempts + 1 });
          kept++;
        }
      }
      return { replayed, dropped, kept };
    },
    pending: () => storage.all(),
  };
}

/** In-memory storage (tests, web preview). */
export function memoryOutboxStorage(): OutboxStorage {
  const map = new Map<string, OutboxEntry>();
  return {
    async put(e) {
      map.set(e.clientOpId, e);
    },
    async remove(id) {
      map.delete(id);
    },
    async all() {
      return [...map.values()];
    },
  };
}
