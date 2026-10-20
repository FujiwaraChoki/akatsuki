// internal.timers.* (ARCHITECTURE §4.5, §10, D-001).
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Id } from "./_generated/dataModel";
import { internalMutation, type MutationCtx } from "./_generated/server";
import { fireTimer } from "./lib/timers";

export const fire = internalMutation({
  args: { timerId: v.id("timers") },
  returns: v.null(),
  handler: async (ctx, { timerId }) => {
    await fireTimer(ctx, timerId);
    return null;
  },
});

const OVERDUE_MS = 60_000;

/** A scheduled function is "missing" when it no longer exists, failed or was cancelled. */
async function scheduledFnLost(
  ctx: MutationCtx,
  id: Id<"_scheduled_functions"> | undefined,
): Promise<boolean> {
  if (!id) return true;
  const fn = await ctx.db.system.get(id);
  return !fn || fn.state.kind === "failed" || fn.state.kind === "canceled";
}

/** Cron: fires active timers and scheduled sends more than 60 s overdue whose scheduled function is lost. */
export const sweep = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const cutoff = Date.now() - OVERDUE_MS;
    const timers = await ctx.db
      .query("timers")
      .withIndex("by_state_fire", (q) => q.eq("state", "active").lt("fireAt", cutoff))
      .take(100);
    for (const t of timers) {
      if (await scheduledFnLost(ctx, t.scheduledFnId)) await fireTimer(ctx, t._id);
    }
    const sends = await ctx.db
      .query("scheduledSends")
      .withIndex("by_state_sendAt", (q) => q.eq("state", "scheduled").lt("sendAt", cutoff))
      .take(100);
    for (const s of sends) {
      if (await scheduledFnLost(ctx, s.scheduledFnId)) {
        await ctx.scheduler.runAfter(0, internal.sends.fire, { sendId: s._id });
      }
    }
    return null;
  },
});
