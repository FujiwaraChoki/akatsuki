// senderPrefs.* (ARCHITECTURE §4.2): per-sender remote images, VIP, block.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import { entitledMutation, entitledQuery } from "./lib/auth";
import { appError } from "./lib/errors";
import { vSenderPrefView } from "./lib/rows";

const SENDER_RE = /^([^\s@]+@[^\s@]+\.[^\s@]+|@[^\s@]+\.[^\s@]+)$/;

function normSender(sender: string): string {
  const s = sender.trim().toLowerCase();
  if (!SENDER_RE.test(s) || s.length > 320) throw appError("invalid_argument", "Use an address or @domain.");
  return s;
}

export const get = entitledQuery({
  args: { sender: v.string() },
  returns: v.union(vSenderPrefView, v.null()),
  handler: async (ctx, { sender }) => {
    const s = normSender(sender);
    const row = await ctx.db
      .query("senderPrefs")
      .withIndex("by_user_sender", (q) => q.eq("userId", ctx.userId).eq("sender", s))
      .unique();
    return row
      ? { sender: row.sender, allowRemoteImages: row.allowRemoteImages, vip: row.vip, blocked: row.blocked }
      : null;
  },
});

export const set = entitledMutation({
  args: {
    sender: v.string(),
    allowRemoteImages: v.optional(v.boolean()),
    vip: v.optional(v.boolean()),
    blocked: v.optional(v.boolean()),
  },
  returns: v.null(),
  handler: async (ctx, { sender, ...patch }) => {
    const s = normSender(sender);
    const row = await ctx.db
      .query("senderPrefs")
      .withIndex("by_user_sender", (q) => q.eq("userId", ctx.userId).eq("sender", s))
      .unique();
    const vipChanged = patch.vip !== undefined && patch.vip !== (row?.vip ?? false);
    if (row) {
      await ctx.db.patch(row._id, patch);
    } else {
      await ctx.db.insert("senderPrefs", {
        userId: ctx.userId,
        sender: s,
        allowRemoteImages: patch.allowRemoteImages ?? false,
        vip: patch.vip ?? false,
        blocked: patch.blocked ?? false,
      });
    }
    if (vipChanged)
      await ctx.scheduler.runAfter(0, internal.splits.recompute, { userId: ctx.userId, cursor: null });
    return null;
  },
});
