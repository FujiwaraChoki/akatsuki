// rules.* (ARCHITECTURE §4.2, D-046): filters applied to newly arrived inbound mail in worker.upsertThreads.
import { v } from "convex/values";
import { entitledMutation, entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { project, vRuleView } from "./lib/rows";
import { vRule, vThreadAction } from "./lib/validators";
import { validateRule } from "./splits";

const MAX_RULES = 100;

export const list = entitledQuery({
  args: {},
  returns: v.array(vRuleView),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("rules")
      .withIndex("by_user_order", (q) => q.eq("userId", ctx.userId))
      .take(MAX_RULES);
    return rows.map((r) => project(vRuleView, r));
  },
});

export const save = entitledMutation({
  args: {
    ruleId: v.optional(v.id("rules")),
    accountId: v.optional(v.id("mailAccounts")),
    name: v.string(),
    enabled: v.boolean(),
    order: v.number(),
    rule: vRule,
    actions: v.array(vThreadAction),
  },
  returns: v.object({ ruleId: v.id("rules") }),
  handler: async (ctx, { ruleId, accountId, name, enabled, order, rule, actions }) => {
    const n = name.trim();
    if (n.length === 0 || n.length > 80)
      throw appError("invalid_argument", "Rule names are 1 to 80 characters.");
    if (accountId) await getOwned(ctx, accountId, ctx.userId);
    await validateRule(ctx, ctx.userId, rule);
    if (actions.length === 0 || actions.length > 10)
      throw appError("invalid_argument", "A rule needs 1 to 10 actions.");
    for (const a of actions) {
      if (a.kind === "unsubscribe" || a.kind === "delete_forever") {
        throw appError("invalid_action", "Rules cannot unsubscribe or delete permanently.");
      }
    }
    const doc = { userId: ctx.userId, accountId, name: n, enabled, order, rule, actions };
    if (ruleId) {
      await getOwned(ctx, ruleId, ctx.userId);
      await ctx.db.replace(ruleId, doc);
      return { ruleId };
    }
    const count = (
      await ctx.db
        .query("rules")
        .withIndex("by_user_order", (q) => q.eq("userId", ctx.userId))
        .take(MAX_RULES + 1)
    ).length;
    if (count >= MAX_RULES) throw appError("invalid_argument", `At most ${MAX_RULES} rules.`);
    return { ruleId: await ctx.db.insert("rules", doc) };
  },
});

export const remove = entitledMutation({
  args: { ruleId: v.id("rules") },
  returns: v.null(),
  handler: async (ctx, { ruleId }) => {
    await getOwned(ctx, ruleId, ctx.userId);
    await ctx.db.delete(ruleId);
    return null;
  },
});
