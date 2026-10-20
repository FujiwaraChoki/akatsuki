// readReceipts.forMessage (ARCHITECTURE §4.2). Opt-in only (BRIEF decision 8).
import { v } from "convex/values";
import { entitledQuery, getOwned } from "./lib/auth";

export const forMessage = entitledQuery({
  args: { messageId: v.id("messages") },
  returns: v.union(
    v.object({ opens: v.array(v.object({ at: v.number(), proxied: v.boolean() })) }),
    v.null(),
  ),
  handler: async (ctx, { messageId }) => {
    await getOwned(ctx, messageId, ctx.userId);
    const row = await ctx.db
      .query("readReceipts")
      .withIndex("by_message", (q) => q.eq("messageId", messageId))
      .first();
    if (!row || row.userId !== ctx.userId) return null;
    return { opens: row.opens };
  },
});
