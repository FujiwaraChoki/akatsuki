// Wi-Fi prefetch (ARCHITECTURE §16.2): up to 100 inbox bodies through `POST /v1/prefetch`, so the first open is
// instant. Runs at most every 15 minutes, only on Wi-Fi, and only when bodies may be cached on the device.

import type { Id } from "@akatsuki/convex";
import { api } from "@akatsuki/convex";
import type { ConvexReactClient } from "convex/react";
import * as Network from "expo-network";
import { cachedIds, rememberBody } from "./bodies";
import { requireWorker } from "./clients";
import { kvGet, kvSet } from "./db";

const KEY = "prefetch.at";
const EVERY_MS = 15 * 60_000;
const MAX_THREADS = 25;
const MAX_BODIES = 100;
let running = false;

export async function prefetchInbox(
  convex: ConvexReactClient,
  threadIds: readonly string[],
  persist: boolean,
): Promise<number> {
  if (!persist || running || threadIds.length === 0) return 0;
  running = true;
  try {
    const net = await Network.getNetworkStateAsync();
    if (net.type !== Network.NetworkStateType.WIFI || net.isInternetReachable === false) return 0;
    const last = (await kvGet<number>(KEY)) ?? 0;
    if (Date.now() - last < EVERY_MS) return 0;
    await kvSet(KEY, Date.now());
    const ids: string[] = [];
    for (const threadId of threadIds.slice(0, MAX_THREADS)) {
      const t = await convex.query(api.mail.getThread, { threadId: threadId as Id<"threads"> });
      const newest = t.messages
        .filter((m) => !m.isDraft)
        .slice(-4)
        .map((m) => m._id as string);
      ids.push(...newest);
      if (ids.length >= MAX_BODIES) break;
    }
    const have = await cachedIds(ids);
    const missing = ids.filter((id) => !have.has(id)).slice(0, MAX_BODIES);
    if (missing.length === 0) return 0;
    let n = 0;
    for await (const body of requireWorker().prefetch(missing as Id<"messages">[])) {
      await rememberBody(body, true);
      n++;
    }
    return n;
  } catch {
    return 0;
  } finally {
    running = false;
  }
}
