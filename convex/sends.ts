// sends.* (ARCHITECTURE §4.2, §5.6, §10.2–10.3) and internal.sends.fire.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Doc } from "./_generated/dataModel";
import { internalMutation, type MutationCtx } from "./_generated/server";
import { entitledMutation, entitledQuery } from "./lib/auth";
import { appError, appErrorValue } from "./lib/errors";
import { insertOp } from "./lib/ops";
import { consume } from "./lib/rateLimits";
import { project, vScheduledSendView } from "./lib/rows";

const vListState = v.union(
  v.literal("scheduled"),
  v.literal("due"),
  v.literal("sending"),
  v.literal("failed"),
  v.literal("unknown"),
);

async function byClientSendId(ctx: Pick<MutationCtx, "db">, userId: string, clientSendId: string) {
  return await ctx.db
    .query("scheduledSends")
    .withIndex("by_user_client", (q) => q.eq("userId", userId).eq("clientSendId", clientSendId))
    .unique();
}

export const list = entitledQuery({
  args: { states: v.optional(v.array(vListState)) },
  returns: v.array(vScheduledSendView),
  handler: async (ctx, { states }) => {
    const wanted = states ?? ["scheduled", "due", "sending", "failed", "unknown"];
    const out: Doc<"scheduledSends">[] = [];
    for (const state of new Set(wanted)) {
      const rows = await ctx.db
        .query("scheduledSends")
        .withIndex("by_user_state_sendAt", (q) => q.eq("userId", ctx.userId).eq("state", state))
        .take(200);
      out.push(...rows);
    }
    return out.sort((a, b) => a.sendAt - b.sendAt).map((s) => project(vScheduledSendView, s));
  },
});

/** The worker's rule for client send ids (`apps/sync` send route): 8–64 of `[A-Za-z0-9-]`. */
export const CLIENT_SEND_ID = /^[A-Za-z0-9-]{8,64}$/;

export const cancel = entitledMutation({
  args: { clientSendId: v.string() },
  returns: v.object({ result: v.union(v.literal("cancelled"), v.literal("too_late")) }),
  handler: async (ctx, { clientSendId }) => {
    if (!CLIENT_SEND_ID.test(clientSendId)) throw appError("invalid_argument", "Bad send id.");
    const row = await byClientSendId(ctx, ctx.userId, clientSendId);
    if (!row) {
      // Undo pressed before POST /v1/send finished: leave a tombstone for worker.scheduleSend (§10.3).
      // Each one is a new row, so they are rate-limited per user (D-403).
      await consume(ctx, "sendTombstone", ctx.userId);
      await ctx.db.insert("scheduledSends", {
        userId: ctx.userId,
        clientSendId,
        kind: "undo",
        sendAt: 0,
        state: "cancelled",
        preview: { subject: "", to: [] },
        createdAt: Date.now(),
      });
      return { result: "cancelled" as const };
    }
    if (row.state === "cancelled") return { result: "cancelled" as const };
    if (row.state !== "scheduled") return { result: "too_late" as const };
    if (row.scheduledFnId) await ctx.scheduler.cancel(row.scheduledFnId);
    await ctx.db.patch(row._id, { state: "cancelled" });
    if (row.draftId) {
      const draft = await ctx.db.get(row.draftId);
      if (draft && draft.state === "scheduled")
        await ctx.db.patch(draft._id, { state: "editing", updatedAt: Date.now() });
    }
    return { result: "cancelled" as const };
  },
});

export const reschedule = entitledMutation({
  args: { clientSendId: v.string(), sendAt: v.number() },
  returns: v.null(),
  handler: async (ctx, { clientSendId, sendAt }) => {
    const row = await byClientSendId(ctx, ctx.userId, clientSendId);
    if (!row) throw appError("not_found");
    if (row.state !== "scheduled")
      throw appError("invalid_argument", "Only scheduled sends can be rescheduled.");
    if (sendAt <= Date.now()) throw appError("invalid_argument", "The send time must be in the future.");
    if (row.scheduledFnId) await ctx.scheduler.cancel(row.scheduledFnId);
    const scheduledFnId = await ctx.scheduler.runAt(sendAt, internal.sends.fire, { sendId: row._id });
    await ctx.db.patch(row._id, { sendAt, scheduledFnId, kind: "later" });
    return null;
  },
});

export const resolveUnknown = entitledMutation({
  args: { clientSendId: v.string(), resolution: v.union(v.literal("was_sent"), v.literal("discard")) },
  returns: v.null(),
  handler: async (ctx, { clientSendId, resolution }) => {
    const row = await byClientSendId(ctx, ctx.userId, clientSendId);
    if (!row) throw appError("not_found");
    if (row.state !== "unknown")
      throw appError("invalid_argument", "This send is not waiting for a decision.");
    const draft = row.draftId ? await ctx.db.get(row.draftId) : null;
    if (resolution === "was_sent") {
      await ctx.db.patch(row._id, { state: "sent", error: undefined });
      if (draft) await ctx.db.patch(draft._id, { state: "sent", updatedAt: Date.now() });
      return null;
    }
    await ctx.db.patch(row._id, { state: "cancelled" });
    if (draft && row.accountId) {
      const account = await ctx.db.get(row.accountId);
      if (account) {
        await ctx.db.patch(draft._id, { state: "discarded", updatedAt: Date.now() });
        await insertOp(ctx, {
          userId: ctx.userId,
          account,
          clientOpId: `discard:${clientSendId}`,
          // No threadId: only `thread` ops queue per thread (§3.2, §5.4); a draft op never waits on one.
          payload: { kind: "delete_draft", draftId: draft._id },
        });
      }
    }
    return null;
  },
});

const STALE_LATER_MS = 24 * 60 * 60 * 1000;

/** Moves a scheduled send to `due` and inserts its `send` op (guarded by row state). */
export async function fireSend(ctx: MutationCtx, send: Doc<"scheduledSends">): Promise<void> {
  if (send.state !== "scheduled" || !send.accountId || !send.draftId) return;
  const now = Date.now();
  if (send.kind === "later" && now - send.sendAt > STALE_LATER_MS) {
    await ctx.db.patch(send._id, { state: "failed", error: appErrorValue("stale_schedule") });
    return;
  }
  const account = await ctx.db.get(send.accountId);
  const draft = await ctx.db.get(send.draftId);
  if (!account || account.status === "removing") {
    await ctx.db.patch(send._id, { state: "failed", error: appErrorValue("not_found") });
    return;
  }
  if (!draft || draft.state === "discarded") {
    await ctx.db.patch(send._id, { state: "failed", error: appErrorValue("draft_deleted") });
    return;
  }
  const opId = await insertOp(ctx, {
    userId: send.userId,
    account,
    clientOpId: `send:${send.clientSendId}`,
    // No threadId: a send never queues behind a thread op of its conversation (§3.2, §5.4, §10).
    payload: { kind: "send", scheduledSendId: send._id },
  });
  await ctx.db.patch(send._id, { state: "due", opId });
  await ctx.db.patch(draft._id, { state: "sending", updatedAt: now });
}

export const fire = internalMutation({
  args: { sendId: v.id("scheduledSends") },
  returns: v.null(),
  handler: async (ctx, { sendId }) => {
    const send = await ctx.db.get(sendId);
    if (send) await fireSend(ctx, send);
    return null;
  },
});

/** Cron: deletes cancel tombstones (no draft) older than 1 h. */
export const purgeTombstones = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const cutoff = Date.now() - 60 * 60 * 1000;
    const rows = await ctx.db
      .query("scheduledSends")
      .withIndex("by_state_sendAt", (q) => q.eq("state", "cancelled"))
      .take(500);
    for (const r of rows) if (!r.draftId && r.createdAt < cutoff) await ctx.db.delete(r._id);
    return null;
  },
});
