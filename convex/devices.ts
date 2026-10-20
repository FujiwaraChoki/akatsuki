// devices.* (ARCHITECTURE §4.2).
import { v } from "convex/values";
import type { Id } from "./_generated/dataModel";
import type { MutationCtx } from "./_generated/server";
import { userMutation, userQuery } from "./lib/auth";
import { appError } from "./lib/errors";
import { consume } from "./lib/rateLimits";
import { project, vDeviceView } from "./lib/rows";

const vKind = v.union(v.literal("web"), v.literal("desktop"), v.literal("mobile"));

/** Devices kept per user; notify reads at most this many (D-403). */
export const MAX_DEVICES = 50;

export const register = userMutation({
  args: {
    deviceId: v.string(),
    kind: vKind,
    name: v.string(),
    appVersion: v.string(),
    expoPushToken: v.optional(v.string()),
  },
  returns: v.null(),
  handler: async (ctx, args) => {
    if (args.deviceId.length < 8 || args.deviceId.length > 64)
      throw appError("invalid_argument", "Bad device id.");
    if (args.name.length > 120) throw appError("invalid_argument", "Device name is too long.");
    if (args.appVersion.length > 40) throw appError("invalid_argument", "Bad app version.");
    if (
      args.expoPushToken !== undefined &&
      (args.expoPushToken.length === 0 || args.expoPushToken.length > 256)
    )
      throw appError("invalid_argument", "Bad push token.");
    const existing = await ctx.db
      .query("devices")
      .withIndex("by_user_device", (q) => q.eq("userId", ctx.userId).eq("deviceId", args.deviceId))
      .unique();
    const now = Date.now();
    let id: Id<"devices">;
    if (existing) {
      await ctx.db.patch(existing._id, { ...args, lastSeenAt: now });
      id = existing._id;
    } else {
      await consume(ctx, "deviceRegister", ctx.userId);
      const mine = await ctx.db
        .query("devices")
        .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
        .collect();
      // At the cap the least recently seen device makes room (a reinstalled browser must still register).
      const stale = mine
        .sort((a, b) => a.lastSeenAt - b.lastSeenAt)
        .slice(0, Math.max(0, mine.length - MAX_DEVICES + 1));
      for (const d of stale) await ctx.db.delete(d._id);
      id = await ctx.db.insert("devices", { userId: ctx.userId, ...args, lastSeenAt: now });
    }
    // A push token belongs to one installation: when a phone changes hands (sign-out, then someone else signs
    // in), the previous user's row must stop receiving its pushes (D-403).
    if (args.expoPushToken) await releasePushToken(ctx, args.expoPushToken, id);
    return null;
  },
});

/** Clears `token` from every devices row except `keep`, whoever owns it. */
export async function releasePushToken(ctx: MutationCtx, token: string, keep?: Id<"devices">) {
  const holders = await ctx.db
    .query("devices")
    .withIndex("by_push_token", (q) => q.eq("expoPushToken", token))
    .take(100);
  for (const d of holders) if (d._id !== keep) await ctx.db.patch(d._id, { expoPushToken: undefined });
}

export const list = userQuery({
  args: {},
  returns: v.array(vDeviceView),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("devices")
      .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
      .take(100);
    return rows.map((d) => project(vDeviceView, d));
  },
});

export const remove = userMutation({
  args: { deviceId: v.string() },
  returns: v.null(),
  handler: async (ctx, { deviceId }) => {
    const row = await ctx.db
      .query("devices")
      .withIndex("by_user_device", (q) => q.eq("userId", ctx.userId).eq("deviceId", deviceId))
      .unique();
    if (!row) throw appError("not_found");
    await ctx.db.delete(row._id);
    return null;
  },
});
