// Better Auth rate-limit storage (D-394, D-401). Better Auth's own "database" storage keeps its counters in the
// component's `rateLimit` table, whose only index is `key`; its expiry sweep (`lastRequest < cutoff`) then scans
// the whole table on every window reset. This app table has a `lastRequest` index, `consume` decides and writes
// in one serializable mutation (atomic, unlike the adapter's read-then-increment), and a cron removes rows past
// the longest window.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import { internalMutation } from "./_generated/server";

/** Rows older than this are expired for every rule (the longest Better Auth window is 60 s). */
export const AUTH_LIMIT_RETENTION_MS = 10 * 60 * 1000;
const GC_BATCH = 500;

/**
 * One fixed-window step, the same decision as Better Auth's `decideConsume`: a request after the window starts a
 * new one, a request at `max` is refused with the seconds until the window ends.
 */
export const consume = internalMutation({
  args: { key: v.string(), window: v.number(), max: v.number() },
  returns: v.object({ allowed: v.boolean(), retryAfter: v.union(v.number(), v.null()) }),
  handler: async (ctx, { key, window, max }) => {
    const now = Date.now();
    const windowMs = window * 1000;
    const row = await ctx.db
      .query("authRateLimits")
      .withIndex("by_key", (q) => q.eq("key", key))
      .first();
    if (!row) {
      await ctx.db.insert("authRateLimits", { key, count: 1, lastRequest: now });
      return { allowed: true, retryAfter: null };
    }
    if (now - row.lastRequest > windowMs) {
      await ctx.db.patch(row._id, { count: 1, lastRequest: now });
      return { allowed: true, retryAfter: null };
    }
    if (row.count >= max) {
      return {
        allowed: false,
        retryAfter: Math.max(1, Math.ceil((row.lastRequest + windowMs - now) / 1000)),
      };
    }
    await ctx.db.patch(row._id, { count: row.count + 1, lastRequest: now });
    return { allowed: true, retryAfter: null };
  },
});

/** Deletes expired counters through the `lastRequest` index, rescheduling itself while batches are full. */
export const gc = internalMutation({
  args: {},
  returns: v.number(),
  handler: async (ctx) => {
    const cutoff = Date.now() - AUTH_LIMIT_RETENTION_MS;
    const rows = await ctx.db
      .query("authRateLimits")
      .withIndex("by_last_request", (q) => q.lt("lastRequest", cutoff))
      .take(GC_BATCH);
    for (const r of rows) await ctx.db.delete(r._id);
    if (rows.length === GC_BATCH) await ctx.scheduler.runAfter(0, internal.authLimits.gc, {});
    return rows.length;
  },
});
