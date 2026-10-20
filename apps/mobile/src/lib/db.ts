// On-device storage in expo-sqlite (ARCHITECTURE §16.1/§16.2): the body cache (300 MB LRU), the durable mutation
// outbox, and a small key-value table for the cold-start list cache and local compose copies. Everything here is
// wiped on sign-out (`wipeLocalData`).
import * as SQLite from "expo-sqlite";
import { type CacheEntryMeta, MOBILE_BODY_CACHE_BYTES, planEviction } from "../vm/lru";
import type { OutboxEntry, OutboxStorage } from "../vm/outbox";

let db: SQLite.SQLiteDatabase | null = null;

function open(): SQLite.SQLiteDatabase {
  if (db) return db;
  db = SQLite.openDatabaseSync("akatsuki.db");
  db.execSync(`
    PRAGMA journal_mode = WAL;
    CREATE TABLE IF NOT EXISTS bodies (
      message_id TEXT PRIMARY KEY NOT NULL,
      sanitizer_version INTEGER NOT NULL,
      json TEXT NOT NULL,
      size INTEGER NOT NULL,
      last_access INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS bodies_last_access ON bodies (last_access);
    CREATE TABLE IF NOT EXISTS outbox (
      client_op_id TEXT PRIMARY KEY NOT NULL,
      json TEXT NOT NULL,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS kv (
      key TEXT PRIMARY KEY NOT NULL,
      value TEXT NOT NULL,
      updated_at INTEGER NOT NULL
    );
  `);
  return db;
}

// ───────────── key-value ─────────────

export async function kvGet<T>(key: string): Promise<T | null> {
  const row = await open().getFirstAsync<{ value: string }>("SELECT value FROM kv WHERE key = ?", key);
  if (!row) return null;
  try {
    return JSON.parse(row.value) as T;
  } catch {
    return null;
  }
}

export async function kvSet(key: string, value: unknown): Promise<void> {
  await open().runAsync(
    "INSERT INTO kv (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    key,
    JSON.stringify(value),
    Date.now(),
  );
}

export async function kvDelete(key: string): Promise<void> {
  await open().runAsync("DELETE FROM kv WHERE key = ?", key);
}

// ───────────── bodies ─────────────

export async function bodyGet<T>(messageId: string, minVersion: number): Promise<T | null> {
  const d = open();
  const row = await d.getFirstAsync<{ json: string; sanitizer_version: number }>(
    "SELECT json, sanitizer_version FROM bodies WHERE message_id = ?",
    messageId,
  );
  if (!row || row.sanitizer_version < minVersion) return null;
  await d.runAsync("UPDATE bodies SET last_access = ? WHERE message_id = ?", Date.now(), messageId);
  try {
    return JSON.parse(row.json) as T;
  } catch {
    return null;
  }
}

export async function bodyPut(
  messageId: string,
  sanitizerVersion: number,
  body: unknown,
  size: number,
): Promise<void> {
  const d = open();
  await d.runAsync(
    "INSERT INTO bodies (message_id, sanitizer_version, json, size, last_access) VALUES (?, ?, ?, ?, ?) ON CONFLICT(message_id) DO UPDATE SET sanitizer_version = excluded.sanitizer_version, json = excluded.json, size = excluded.size, last_access = excluded.last_access",
    messageId,
    sanitizerVersion,
    JSON.stringify(body),
    size,
    Date.now(),
  );
}

let evicting = false;

/** Enforces the 300 MB LRU cap. Cheap when under the cap (one SUM). */
export async function bodyEvict(capBytes = MOBILE_BODY_CACHE_BYTES): Promise<number> {
  if (evicting) return 0;
  evicting = true;
  try {
    const d = open();
    const total = await d.getFirstAsync<{ total: number | null }>("SELECT SUM(size) AS total FROM bodies");
    if ((total?.total ?? 0) <= capBytes) return 0;
    const rows = await d.getAllAsync<{ message_id: string; size: number; last_access: number }>(
      "SELECT message_id, size, last_access FROM bodies",
    );
    const entries: CacheEntryMeta[] = rows.map((r) => ({
      key: r.message_id,
      size: r.size,
      lastAccess: r.last_access,
    }));
    const victims = planEviction(entries, capBytes);
    for (const id of victims) await d.runAsync("DELETE FROM bodies WHERE message_id = ?", id);
    return victims.length;
  } finally {
    evicting = false;
  }
}

export async function bodyIds(ids: readonly string[], minVersion: number): Promise<Set<string>> {
  if (ids.length === 0) return new Set();
  const placeholders = ids.map(() => "?").join(",");
  const rows = await open().getAllAsync<{ message_id: string }>(
    `SELECT message_id FROM bodies WHERE sanitizer_version >= ? AND message_id IN (${placeholders})`,
    minVersion,
    ...ids,
  );
  return new Set(rows.map((r) => r.message_id));
}

// ───────────── outbox ─────────────

export const sqliteOutboxStorage: OutboxStorage = {
  async put(e: OutboxEntry) {
    await open().runAsync(
      "INSERT INTO outbox (client_op_id, json, created_at) VALUES (?, ?, ?) ON CONFLICT(client_op_id) DO UPDATE SET json = excluded.json",
      e.clientOpId,
      JSON.stringify(e),
      e.createdAt,
    );
  },
  async remove(id: string) {
    await open().runAsync("DELETE FROM outbox WHERE client_op_id = ?", id);
  },
  async all() {
    const rows = await open().getAllAsync<{ json: string }>("SELECT json FROM outbox ORDER BY created_at");
    return rows.flatMap((r) => {
      try {
        return [JSON.parse(r.json) as OutboxEntry];
      } catch {
        return [];
      }
    });
  },
};

/** Sign-out (§16.1): bodies, outbox, list caches and local drafts. */
export async function wipeLocalData(): Promise<void> {
  const d = open();
  await d.execAsync("DELETE FROM bodies; DELETE FROM outbox; DELETE FROM kv;");
  await d.execAsync("VACUUM;");
}
