// splits.* (ARCHITECTURE §3.4, §4.2) and internal.splits.recompute.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Id } from "./_generated/dataModel";
import { internalMutation, type MutationCtx } from "./_generated/server";
import { entitledMutation, entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { isLabelKey, isSplitKey } from "./lib/model";
import { toSplitView, vSplitView } from "./lib/rows";
import { recomputeThread } from "./lib/threads";
import { type Rule, vRule } from "./lib/validators";

export const MAX_SPLITS = 20;
export const MAX_CONDS = 20;

/** Shared by splits and rules: 1..20 conditions, valid label keys, owned account ids. */
export async function validateRule(ctx: MutationCtx, userId: string, rule: Rule): Promise<void> {
  if (rule.conds.length === 0 || rule.conds.length > MAX_CONDS) {
    throw appError("invalid_argument", `A rule needs 1 to ${MAX_CONDS} conditions.`);
  }
  for (const c of rule.conds) {
    if (c.f === "label" && (!isLabelKey(c.key) || isSplitKey(c.key))) {
      throw appError("invalid_argument", "Rules can only test sys: and l: label keys.");
    }
    if (c.f === "account") await getOwned(ctx, c.accountId, userId);
    if ("eq" in c && typeof c.eq === "string" && (c.eq.length === 0 || c.eq.length > 320)) {
      throw appError("invalid_argument", "Empty or overlong condition value.");
    }
    if (c.f === "subject_contains" && (c.text.length === 0 || c.text.length > 200)) {
      throw appError("invalid_argument", "Empty or overlong subject text.");
    }
  }
}

async function scheduleRecompute(ctx: MutationCtx, userId: string) {
  await ctx.scheduler.runAfter(0, internal.splits.recompute, { userId, cursor: null });
}

export const list = entitledQuery({
  args: {},
  returns: v.array(vSplitView),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("splits")
      .withIndex("by_user_order", (q) => q.eq("userId", ctx.userId))
      .take(MAX_SPLITS);
    return rows.map(toSplitView);
  },
});

export const save = entitledMutation({
  args: { splitId: v.optional(v.id("splits")), name: v.string(), rule: vRule, showCount: v.boolean() },
  returns: v.object({ splitId: v.id("splits") }),
  handler: async (ctx, { splitId, name, rule, showCount }) => {
    const n = name.trim();
    if (n.length === 0 || n.length > 60)
      throw appError("invalid_argument", "Split names are 1 to 60 characters.");
    await validateRule(ctx, ctx.userId, rule);
    let id: Id<"splits">;
    if (splitId) {
      await getOwned(ctx, splitId, ctx.userId);
      await ctx.db.patch(splitId, { name: n, rule, showCount });
      id = splitId;
    } else {
      const existing = await ctx.db
        .query("splits")
        .withIndex("by_user_order", (q) => q.eq("userId", ctx.userId))
        .collect();
      if (existing.length >= MAX_SPLITS) throw appError("invalid_argument", `At most ${MAX_SPLITS} splits.`);
      const order = existing.reduce((m, s) => Math.max(m, s.order + 1), 0);
      id = await ctx.db.insert("splits", { userId: ctx.userId, name: n, order, rule, showCount });
    }
    await scheduleRecompute(ctx, ctx.userId);
    return { splitId: id };
  },
});

export const remove = entitledMutation({
  args: { splitId: v.id("splits") },
  returns: v.null(),
  handler: async (ctx, { splitId }) => {
    await getOwned(ctx, splitId, ctx.userId);
    await ctx.db.delete(splitId);
    await scheduleRecompute(ctx, ctx.userId);
    return null;
  },
});

export const reorder = entitledMutation({
  args: { order: v.array(v.id("splits")) },
  returns: v.null(),
  handler: async (ctx, { order }) => {
    if (new Set(order).size !== order.length) throw appError("invalid_argument", "Duplicate split ids.");
    const existing = await ctx.db
      .query("splits")
      .withIndex("by_user_order", (q) => q.eq("userId", ctx.userId))
      .collect();
    if (existing.length !== order.length || !existing.every((s) => order.includes(s._id))) {
      throw appError("invalid_argument", "The order must list every split exactly once.");
    }
    for (const [i, id] of order.entries()) await ctx.db.patch(id, { order: i });
    await scheduleRecompute(ctx, ctx.userId);
    return null;
  },
});

/** Re-evaluates the split of every inbox thread, 200 per run (§3.4). */
export const recompute = internalMutation({
  args: { userId: v.string(), cursor: v.union(v.string(), v.null()) },
  returns: v.null(),
  handler: async (ctx, { userId, cursor }) => {
    const page = await ctx.db
      .query("threadLabels")
      .withIndex("by_user_label_sort", (q) => q.eq("userId", userId).eq("labelKey", "sys:inbox"))
      .paginate({ numItems: 200, cursor });
    for (const row of page.page) await recomputeThread(ctx, row.threadId);
    if (!page.isDone) {
      await ctx.scheduler.runAfter(0, internal.splits.recompute, { userId, cursor: page.continueCursor });
    }
    return null;
  },
});
