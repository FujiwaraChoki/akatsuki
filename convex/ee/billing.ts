// ee.billing.entitlement (ARCHITECTURE §4.2, §13). Pro-only code; same licence, separate directory.
import { v } from "convex/values";
import { loadEntitlement, userQuery } from "../lib/auth";
import { NO_BILLING_VIEW, project, vBillingView } from "../lib/rows";
import { vEntitlement } from "../lib/validators";
import { aiBudgetMicros } from "./entitlement";

export const entitlement = userQuery({
  args: {},
  returns: v.object({
    entitlement: vEntitlement,
    billing: vBillingView,
    aiUsage: v.object({ period: v.string(), usedMicros: v.number(), limitMicros: v.number() }),
  }),
  handler: async (ctx) => {
    const ent = await loadEntitlement(ctx, ctx.userId);
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
      .unique();
    // Queries cannot read the clock, so this reports the newest usage period on record; a client
    // whose current UTC month differs treats usage as 0.
    const usage = await ctx.db
      .query("aiUsage")
      .withIndex("by_user_period", (q) => q.eq("userId", ctx.userId))
      .order("desc")
      .first();
    return {
      entitlement: ent,
      billing: billing ? project(vBillingView, billing) : NO_BILLING_VIEW,
      aiUsage: {
        period: usage?.period ?? "",
        usedMicros: usage?.costMicros ?? 0,
        limitMicros: aiBudgetMicros(ent),
      },
    };
  },
});
