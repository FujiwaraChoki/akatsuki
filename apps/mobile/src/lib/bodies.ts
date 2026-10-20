// Message bodies (ARCHITECTURE §2, §7.2, §16.1): fetched from the sync worker with the user's JWT, never from Convex.
// Cached by (messageId, sanitizerVersion): an in-memory LRU always, and SQLite (300 MB LRU) when
// `prefs.cacheBodiesOnDevice` is on. A newer `sanitizerVersion` seen from the worker invalidates older entries.
import type { MessageBody } from "@akatsuki/wire";
import { useEffect, useRef, useState } from "react";
import { bodySize, MemoryLru } from "../vm/lru";
import { requireWorker } from "./clients";
import { bodyEvict, bodyGet, bodyIds, bodyPut, kvGet, kvSet } from "./db";

export type BodyState = { state: "loading" | "ready" | "error"; body: MessageBody | null };

const SANITIZER_KEY = "sanitizerVersion";
const memory = new MemoryLru<MessageBody>(40 * 1024 * 1024);
let latestVersion: number | null = null;
let evictTimer: ReturnType<typeof setTimeout> | null = null;

async function latest(): Promise<number> {
  if (latestVersion === null) latestVersion = (await kvGet<number>(SANITIZER_KEY)) ?? 0;
  return latestVersion;
}

export async function rememberBody(body: MessageBody, persist: boolean): Promise<void> {
  const v = await latest();
  if (body.sanitizerVersion > v) {
    latestVersion = body.sanitizerVersion;
    await kvSet(SANITIZER_KEY, body.sanitizerVersion);
  }
  memory.set(body.messageId, body, bodySize(body));
  if (persist) {
    await bodyPut(body.messageId, body.sanitizerVersion, body, bodySize(body));
    if (!evictTimer)
      evictTimer = setTimeout(() => {
        evictTimer = null;
        bodyEvict().catch(() => {});
      }, 5000);
  }
}

export async function cachedBody(messageId: string, persist: boolean): Promise<MessageBody | null> {
  const v = await latest();
  const hot = memory.get(messageId);
  if (hot && hot.sanitizerVersion >= v) return hot;
  if (!persist) return null;
  const stored = await bodyGet<MessageBody>(messageId, v);
  if (stored) memory.set(messageId, stored, bodySize(stored));
  return stored;
}

/** Ids among `ids` that already have a fresh cached body (for prefetch). */
export async function cachedIds(ids: readonly string[]): Promise<Set<string>> {
  return bodyIds(ids, await latest());
}

export function clearMemoryBodies(): void {
  memory.clear();
  latestVersion = null;
}

/**
 * Bodies for the messages of one thread. Cached bodies render at once; the rest stream from
 * `GET /v1/threads/{id}/bodies` (NDJSON, newest first), with one request per message as the fallback.
 */
export function useThreadBodies(threadId: string, messageIds: readonly string[], persist: boolean) {
  const [bodies, setBodies] = useState<Record<string, BodyState>>({});
  const [attempt, setAttempt] = useState(0);
  const key = messageIds.join(",");
  const idsRef = useRef(messageIds);
  idsRef.current = messageIds;

  // `key` (the message ids) and `attempt` (retry) are the triggers; the ids themselves are read from the ref.
  // biome-ignore lint/correctness/useExhaustiveDependencies: see above
  useEffect(() => {
    let cancelled = false;
    const controller = new AbortController();
    const ids = idsRef.current;
    const set = (id: string, s: BodyState) => {
      if (!cancelled) setBodies((prev) => ({ ...prev, [id]: s }));
    };
    (async () => {
      const missing = new Set<string>();
      for (const id of ids) {
        const hit = await cachedBody(id, persist);
        if (hit) set(id, { state: "ready", body: hit });
        else {
          missing.add(id);
          set(id, { state: "loading", body: null });
        }
      }
      if (missing.size === 0 || cancelled) return;
      const worker = requireWorker();
      try {
        for await (const body of worker.threadBodies(threadId as never, { signal: controller.signal })) {
          if (cancelled) return;
          await rememberBody(body, persist);
          if (missing.delete(body.messageId)) set(body.messageId, { state: "ready", body });
          if (missing.size === 0) break;
        }
      } catch {
        // fall through to per-message requests
      }
      for (const id of [...missing]) {
        if (cancelled) return;
        try {
          const body = await worker.getMessage(id as never, { signal: controller.signal });
          await rememberBody(body, persist);
          missing.delete(id);
          set(id, { state: "ready", body });
        } catch {
          set(id, { state: "error", body: null });
        }
      }
    })();
    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [threadId, key, persist, attempt]);

  return { bodies, retry: () => setAttempt((n) => n + 1) };
}

/** base64 of bytes, without relying on Buffer (Hermes has btoa). */
export function bytesToBase64(bytes: Uint8Array): string {
  let bin = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) bin += String.fromCharCode(...bytes.subarray(i, i + chunk));
  return btoa(bin);
}

const INLINE_TYPES = /^image\/(png|gif|jpeg|webp)$/;
const INLINE_MAX = 1024 * 1024;

/** Fetches inline `cid:` images (≤ 1 MB, raster only) for messages that are ready, as data URLs. */
export function useInlineImages(bodies: Record<string, BodyState>): Record<string, Record<string, string>> {
  const [images, setImages] = useState<Record<string, Record<string, string>>>({});
  const requested = useRef(new Set<string>());
  useEffect(() => {
    const worker = requireWorker();
    for (const [messageId, s] of Object.entries(bodies)) {
      if (s.state !== "ready" || !s.body?.html) continue;
      for (const a of s.body.attachments) {
        const k = `${messageId}/${a.partId}`;
        if (!a.inline || !INLINE_TYPES.test(a.mimeType) || a.size > INLINE_MAX || requested.current.has(k))
          continue;
        if (!s.body.html.includes(`data-akatsuki-cid="${a.partId}"`)) continue;
        requested.current.add(k);
        worker
          .getPart(messageId as never, a.partId)
          .then((res) => res.arrayBuffer())
          .then((buf) => {
            if (buf.byteLength > INLINE_MAX) return;
            const url = `data:${a.mimeType};base64,${bytesToBase64(new Uint8Array(buf))}`;
            setImages((prev) => ({ ...prev, [messageId]: { ...prev[messageId], [a.partId]: url } }));
          })
          .catch(() => {});
      }
    }
  }, [bodies]);
  return images;
}
