// internal.billing.* (ARCHITECTURE §4.5, §13.3): entitlement edges, lapse warning and purge.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import { internalMutation } from "./_generated/server";
import { PURGE_AFTER_MS, storeEntitlement } from "./lib/billing";

export const reevaluate = internalMutation({
  args: { userId: v.string() },
  returns: v.null(),
  handler: async (ctx, { userId }) => {
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (billing) await storeEntitlement(ctx, { ...billing, reevaluateFnId: undefined });
    return null;
  },
});

/** +23 days after lapse: warn that synced data will be deleted (only if still lapsed). */
export const purgeWarning = internalMutation({
  args: { userId: v.string() },
  returns: v.null(),
  handler: async (ctx, { userId }) => {
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (!billing || billing.lapsedAt === undefined || billing.entitlement.hostedSync) return null;
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (profile?.email) {
      await ctx.scheduler.runAfter(0, internal.email.send, {
        template: "purge_warning",
        to: profile.email,
        vars: {},
      });
    }
    return null;
  },
});

/** +30 days after lapse: delete all mail metadata and secrets, keep profile and billing (§13.3). */
export const purgeLapsed = internalMutation({
  args: { userId: v.string() },
  returns: v.null(),
  handler: async (ctx, { userId }) => {
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (!billing || billing.entitlement.hostedSync || billing.lapsedAt === undefined) return null;
    if (Date.now() - billing.lapsedAt < PURGE_AFTER_MS - 60_000) return null;
    const accounts = await ctx.db
      .query("mailAccounts")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .collect();
    for (const a of accounts) {
      await ctx.db.patch(a._id, { status: "removing" });
      await ctx.scheduler.runAfter(0, internal.accounts.purge, {
        accountId: a._id,
        table: "threads",
        cursor: null,
      });
    }
    await ctx.db.patch(billing._id, { purgeFnId: undefined });
    return null;
  },
});
