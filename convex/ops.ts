// ops.* (ARCHITECTURE §4.2, §5.3, §5.7) and the outbox crons (§4.5).
import { v } from "convex/values";
import type { Doc, Id } from "./_generated/dataModel";
import { internalMutation } from "./_generated/server";
import { entitledMutation, getOwned, userMutation, userQuery } from "./lib/auth";
import { appError, appErrorValue } from "./lib/errors";
import type { ThreadActionLike } from "./lib/model";
import { checkClientOpId, enqueueThreadAction, insertOp, opsForClientOp } from "./lib/ops";
import { vOpFailure } from "./lib/rows";
import { recomputeThread } from "./lib/threads";

export const failures = userQuery({
  args: {},
  returns: v.array(vOpFailure),
  handler: async (ctx) => {
    const rows: Doc<"ops">[] = [];
    for (const state of ["failed", "unknown"] as const) {
      rows.push(
        ...(await ctx.db
          .query("ops")
          .withIndex("by_user_state_acked", (q) =>
            q.eq("userId", ctx.userId).eq("state", state).eq("failureAcked", false),
          )
          .order("desc")
          .take(20)),
      );
    }
    rows.sort((a, b) => (b.completedAt ?? b.createdAt) - (a.completedAt ?? a.createdAt));
    const out = [];
    for (const op of rows.slice(0, 20)) {
      const account = await ctx.db.get(op.accountId);
      const thread = op.threadId ? await ctx.db.get(op.threadId) : null;
      out.push({
        _id: op._id,
        threadId: op.threadId,
        subject: thread?.subject,
        accountEmail: account?.email ?? "",
        payloadKind: op.payload.kind,
        error: op.lastError ?? appErrorValue("internal"),
        state: op.state as "failed" | "unknown",
        at: op.completedAt ?? op.createdAt,
      });
    }
    return out;
  },
});

export const ackFailure = userMutation({
  args: { opIds: v.array(v.id("ops")) },
  returns: v.null(),
  handler: async (ctx, { opIds }) => {
    if (opIds.length > 100) throw appError("invalid_argument", "At most 100 ops.");
    for (const id of opIds) {
      const op = await getOwned(ctx, id, ctx.userId);
      if (!op.failureAcked) await ctx.db.patch(id, { failureAcked: true });
    }
    return null;
  },
});

export const retry = entitledMutation({
  args: { opId: v.id("ops"), clientOpId: v.string() },
  returns: v.object({ opIds: v.array(v.id("ops")) }),
  handler: async (ctx, { opId, clientOpId }) => {
    checkClientOpId(clientOpId);
    const replay = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (replay.length > 0) return { opIds: replay.map((o) => o._id) };
    const op = await getOwned(ctx, opId, ctx.userId);
    if (op.state !== "failed") throw appError("invalid_argument", "Only failed ops can be retried.");
    if (op.payload.kind === "send") {
      throw appError("invalid_action", "Sends are resolved with sends.resolveUnknown or a new send.");
    }
    const account = await ctx.db.get(op.accountId);
    if (!account || account.status === "removing") throw appError("not_found");
    await ctx.db.patch(opId, { failureAcked: true });
    if (op.payload.kind === "thread") {
      const thread = op.threadId ? await ctx.db.get(op.threadId) : null;
      if (!thread) throw appError("not_found");
      const r = await enqueueThreadAction(ctx, {
        userId: ctx.userId,
        account,
        thread,
        clientOpId,
        action: op.payload.action as ThreadActionLike,
      });
      return { opIds: [r.opId] };
    }
    const newId = await insertOp(ctx, {
      userId: ctx.userId,
      account,
      clientOpId,
      payload: op.payload,
      labelId: op.labelId,
      threadId: op.threadId,
    });
    return { opIds: [newId] };
  },
});

/** No-op marker scheduled at `availableAt` so the worker subscription wakes up (§4.5). */
export const release = internalMutation({
  args: { opId: v.id("ops") },
  returns: v.null(),
  handler: async () => null,
});

const REAUTH_TIMEOUT_MS = 7 * 24 * 60 * 60 * 1000;

/** Cron (1 min): requeue expired leases (§5.3) and fail ops stuck 7 days on reauth. */
export const reapLeases = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const now = Date.now();
    const expired = await ctx.db
      .query("ops")
      .withIndex("by_state_lease", (q) =>
        q.eq("state", "inflight").gte("leaseExpiresAt", 0).lt("leaseExpiresAt", now),
      )
      .take(200);
    for (const op of expired) {
      const verifyOnly = op.payload.kind === "send" && op.sendStartedAt !== undefined ? true : op.verifyOnly;
      await ctx.db.patch(op._id, {
        state: "queued",
        leaseOwner: undefined,
        leaseExpiresAt: undefined,
        availableAt: now,
        verifyOnly,
      });
    }
    // Accounts waiting for reauth are few; a bounded scan is fine for a once-a-minute cron.
    const reauth = await ctx.db
      .query("mailAccounts")
      .filter((q) => q.eq(q.field("status"), "reauth_required"))
      .take(200);
    for (const account of reauth) {
      const stale = await ctx.db
        .query("ops")
        .withIndex("by_account_state", (q) => q.eq("accountId", account._id).eq("state", "queued"))
        .filter((q) => q.lt(q.field("createdAt"), now - REAUTH_TIMEOUT_MS))
        .take(200);
      const touched = new Set<Id<"threads">>();
      for (const op of stale) {
        await ctx.db.patch(op._id, {
          state: "failed",
          completedAt: now,
          lastError: appErrorValue("reauth_timeout"),
        });
        if (op.payload.kind === "send") {
          await ctx.db.patch(op.payload.scheduledSendId, {
            state: "failed",
            error: appErrorValue("reauth_timeout"),
          });
        }
        if (op.threadId) touched.add(op.threadId);
      }
      for (const t of touched) await recomputeThread(ctx, t);
    }
    return null;
  },
});

const DAY = 24 * 60 * 60 * 1000;

/** Cron (hourly): delete done/cancelled ops after 24 h and failed/unknown after 30 d. */
export const purge = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const now = Date.now();
    const plan = [
      ["done", now - DAY],
      ["cancelled", now - DAY],
      ["failed", now - 30 * DAY],
      ["unknown", now - 30 * DAY],
    ] as const;
    for (const [state, before] of plan) {
      const rows = await ctx.db
        .query("ops")
        .withIndex("by_state_completed", (q) =>
          q.eq("state", state).gte("completedAt", 0).lt("completedAt", before),
        )
        .take(500);
      for (const r of rows) await ctx.db.delete(r._id);
    }
    return null;
  },
});
