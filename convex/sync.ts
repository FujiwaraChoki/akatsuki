// sync.changes (ARCHITECTURE §4.2, §16.3): per-account rev feed for the desktop metadata mirror.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Doc, Id } from "./_generated/dataModel";
import { internalMutation, type QueryCtx } from "./_generated/server";
import { entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { revState } from "./lib/revs";
import { project, toLabelView, toMessageMeta, vLabelView, vMessageMeta, vThreadMirror } from "./lib/rows";
import { shardCount } from "./lib/shards";

type Revved = { rev: number };

export const changes = entitledQuery({
  args: { accountId: v.id("mailAccounts"), sinceRev: v.number(), limit: v.number() },
  returns: v.object({
    threads: v.array(vThreadMirror),
    messages: v.array(vMessageMeta),
    labels: v.array(vLabelView),
    tombstones: v.array(
      v.object({
        table: v.union(v.literal("threads"), v.literal("messages"), v.literal("labels")),
        docId: v.string(),
      }),
    ),
    upToRev: v.number(),
    hasMore: v.boolean(),
    resetRequired: v.boolean(),
  }),
  handler: async (ctx, { accountId, sinceRev, limit }) => {
    await getOwned(ctx, accountId, ctx.userId);
    if (limit < 1 || limit > 500) throw appError("invalid_argument", "limit is 1 to 500.");
    const { head, gcRev } = await revState(ctx, accountId);
    const empty = { threads: [], messages: [], labels: [], tombstones: [] };
    if (sinceRev > head) return { ...empty, upToRev: head, hasMore: false, resetRequired: true };
    // Tombstones above sinceRev were garbage-collected: deletions may be missing, so re-pull (§16.3,
    // D-391). A pull from 0 never needs tombstones.
    if (sinceRev > 0 && sinceRev < gcRev)
      return { ...empty, upToRev: head, hasMore: false, resetRequired: true };
    if (sinceRev === head) return { ...empty, upToRev: head, hasMore: false, resetRequired: false };

    const readers = makeReaders(ctx, accountId);
    const fetchRange = async (lo: number, hi: number | null, n: number) => ({
      threads: await readers.threads(lo, hi, n),
      messages: await readers.messages(lo, hi, n),
      labels: await readers.labels(lo, hi, n),
      tombstones: await readers.tombstones(lo, hi, n),
    });

    // Take `limit + 1` per source; a source that overflows caps the window just below its last rev,
    // so a rev is never split across two pages.
    let batch = await fetchRange(sinceRev, null, limit + 1);
    let upToRev = head;
    let hasMore = false;
    for (const rows of Object.values(batch) as Revved[][]) {
      if (rows.length > limit) {
        hasMore = true;
        upToRev = Math.min(upToRev, rows[rows.length - 1]!.rev - 1);
      }
    }
    if (upToRev <= sinceRev) {
      // One mutation stamped more than `limit` docs with the same rev: return that rev whole.
      upToRev = sinceRev + 1;
      batch = await fetchRange(sinceRev, upToRev, 2000);
    }
    const within = <T extends Revved>(rows: T[]) => rows.filter((r) => r.rev > sinceRev && r.rev <= upToRev);
    return {
      threads: within(batch.threads).map((t) => project(vThreadMirror, t)),
      messages: within(batch.messages).map(toMessageMeta),
      labels: within(batch.labels).map(toLabelView),
      tombstones: within(batch.tombstones).map((t) => ({ table: t.table, docId: t.docId })),
      upToRev,
      hasMore: hasMore && upToRev < head,
      resetRequired: false,
    };
  },
});

function makeReaders(ctx: Pick<QueryCtx, "db">, accountId: Id<"mailAccounts">) {
  return {
    threads: (since: number, hi: number | null, n: number): Promise<Doc<"threads">[]> =>
      ctx.db
        .query("threads")
        .withIndex("by_account_rev", (q) =>
          hi === null
            ? q.eq("accountId", accountId).gt("rev", since)
            : q.eq("accountId", accountId).gt("rev", since).lte("rev", hi),
        )
        .take(n),
    messages: (since: number, hi: number | null, n: number): Promise<Doc<"messages">[]> =>
      ctx.db
        .query("messages")
        .withIndex("by_account_rev", (q) =>
          hi === null
            ? q.eq("accountId", accountId).gt("rev", since)
            : q.eq("accountId", accountId).gt("rev", since).lte("rev", hi),
        )
        .take(n),
    labels: (since: number, hi: number | null, n: number): Promise<Doc<"labels">[]> =>
      ctx.db
        .query("labels")
        .withIndex("by_account_rev", (q) =>
          hi === null
            ? q.eq("accountId", accountId).gt("rev", since)
            : q.eq("accountId", accountId).gt("rev", since).lte("rev", hi),
        )
        .take(n),
    tombstones: (since: number, hi: number | null, n: number): Promise<Doc<"tombstones">[]> =>
      ctx.db
        .query("tombstones")
        .withIndex("by_account_rev", (q) =>
          hi === null
            ? q.eq("accountId", accountId).gt("rev", since)
            : q.eq("accountId", accountId).gt("rev", since).lte("rev", hi),
        )
        .take(n),
  };
}

const DAY = 24 * 60 * 60 * 1000;

const GC_BATCH = 1000;

/**
 * Cron (daily): delete tombstones older than 30 days, and raise each account's GC watermark
 * (`accountRevs.gcRev`) to the highest rev deleted, so `sync.changes` can tell a client that is
 * further behind to re-pull (D-391). Reschedules itself while a full batch was deleted.
 */
export const gcTombstones = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("tombstones")
      .withIndex("by_at", (q) => q.lt("at", Date.now() - 30 * DAY))
      .take(GC_BATCH);
    const highest = new Map<Id<"mailAccounts">, number>();
    for (const r of rows) {
      highest.set(r.accountId, Math.max(highest.get(r.accountId) ?? 0, r.rev));
      await ctx.db.delete(r._id);
    }
    for (const [accountId, rev] of highest) {
      const row = await ctx.db
        .query("accountRevs")
        .withIndex("by_account", (q) => q.eq("accountId", accountId))
        .unique();
      if (row && (row.gcRev ?? 0) < rev) await ctx.db.patch(row._id, { gcRev: rev });
    }
    if (rows.length === GC_BATCH) await ctx.scheduler.runAfter(0, internal.sync.gcTombstones, {});
    return null;
  },
});

/**
 * Cron (5 min): mark accounts degraded when their worker lease expired > 2 min ago while live. The detail is a
 * §15 registry code like every other `statusDetail`: `internal` (our sync service is down, retryable), not the
 * ad-hoc `worker_unavailable` it used to write (D-404).
 */
export const watchdog = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const cutoff = Date.now() - 2 * 60 * 1000;
    const shards = shardCount();
    for (let shard = 0; shard < shards; shard++) {
      const stale = await ctx.db
        .query("syncState")
        .withIndex("by_shard_lease", (q) =>
          q.eq("shard", shard).gte("leaseExpiresAt", 0).lt("leaseExpiresAt", cutoff),
        )
        .take(500);
      for (const s of stale) {
        const account = await ctx.db.get(s.accountId);
        if (account && (account.status === "live" || account.status === "initial_sync")) {
          await ctx.db.patch(account._id, { status: "degraded", statusDetail: "internal" });
        }
      }
    }
    return null;
  },
});
