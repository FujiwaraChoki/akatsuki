// me.* (ARCHITECTURE §4.2).
import { v } from "convex/values";
import { components, internal } from "./_generated/api";
import type { MutationCtx } from "./_generated/server";
import { loadEntitlement, userMutation, userQuery } from "./lib/auth";
import { appError } from "./lib/errors";
import { NO_BILLING_VIEW, project, vBillingView, vProfile } from "./lib/rows";
import { createUserRows, isValidTimezone } from "./lib/userRows";
import { vEntitlement, vPrefs } from "./lib/validators";

async function profileOf(ctx: { db: import("./_generated/server").QueryCtx["db"] }, userId: string) {
  return await ctx.db
    .query("profiles")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
}

export const get = userQuery({
  args: {},
  returns: v.union(
    v.null(),
    v.object({
      userId: v.string(),
      email: v.string(),
      name: v.optional(v.string()),
      profile: vProfile,
      entitlement: vEntitlement,
      billing: vBillingView,
    }),
  ),
  handler: async (ctx) => {
    const profile = await profileOf(ctx, ctx.userId);
    // null until me.bootstrap (or the user.onCreate trigger) created the rows.
    if (!profile) return null;
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
      .unique();
    return {
      userId: ctx.userId,
      email: profile.email,
      name: profile.name,
      profile: project(vProfile, profile),
      entitlement: await loadEntitlement(ctx, ctx.userId),
      billing: billing ? project(vBillingView, billing) : NO_BILLING_VIEW,
    };
  },
});

export const bootstrap = userMutation({
  args: { timezone: v.string() },
  returns: v.null(),
  handler: async (ctx, { timezone }) => {
    const identity = await ctx.auth.getUserIdentity();
    const email = typeof identity?.email === "string" ? identity.email : "";
    const name = typeof identity?.name === "string" ? identity.name : undefined;
    await createUserRows(ctx, { userId: ctx.userId, email, name, timezone });
    return null;
  },
});

export const updateProfile = userMutation({
  args: {
    name: v.optional(v.string()),
    timezone: v.optional(v.string()),
    onboarding: v.optional(v.union(v.literal("checkout"), v.literal("connect"), v.literal("done"))),
  },
  returns: v.null(),
  handler: async (ctx, args) => {
    const profile = await profileOf(ctx, ctx.userId);
    if (!profile) throw appError("not_found");
    if (args.timezone !== undefined && !isValidTimezone(args.timezone)) {
      throw appError("invalid_argument", "Unknown time zone.");
    }
    if (args.name !== undefined && args.name.length > 200)
      throw appError("invalid_argument", "Name is too long.");
    await ctx.db.patch(profile._id, { ...args, updatedAt: Date.now() });
    return null;
  },
});

const UNDO_SECONDS = new Set([0, 5, 10, 20, 30]);

export const updatePrefs = userMutation({
  args: { patch: vPrefs.partial() },
  returns: v.null(),
  handler: async (ctx, { patch }) => {
    const profile = await profileOf(ctx, ctx.userId);
    if (!profile) throw appError("not_found");
    if (patch.undoSendSeconds !== undefined && !UNDO_SECONDS.has(patch.undoSendSeconds)) {
      throw appError("invalid_argument", "Undo send must be 0, 5, 10, 20 or 30 seconds.");
    }
    await ctx.db.patch(profile._id, { prefs: { ...profile.prefs, ...patch }, updatedAt: Date.now() });
    return null;
  },
});

/**
 * Deletes the Better Auth side of a user: sessions, linked login accounts and the user row (D-393). Runs
 * inside `me.deleteAccount`, so the login identity goes with the app rows even when the session is too
 * old for Better Auth's own `deleteUser` (which demands a fresh session for passwordless users).
 */
export async function deleteAuthUser(ctx: MutationCtx, userId: string): Promise<void> {
  const page = { cursor: null, numItems: 1000 };
  for (const model of ["session", "account"] as const) {
    await ctx.runMutation(components.betterAuth.adapter.deleteMany, {
      input: { model, where: [{ field: "userId", value: userId }] },
      paginationOpts: page,
    });
  }
  await ctx.runMutation(components.betterAuth.adapter.deleteOne, {
    input: { model: "user", where: [{ field: "_id", value: userId }] },
  });
}

export const deleteAccount = userMutation({
  args: { confirm: v.literal("DELETE") },
  returns: v.null(),
  handler: async (ctx) => {
    const accounts = await ctx.db
      .query("mailAccounts")
      .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
      .collect();
    for (const a of accounts) await ctx.db.patch(a._id, { status: "removing" });
    await ctx.scheduler.runAfter(0, internal.accounts.purgeUser, { userId: ctx.userId });
    await deleteAuthUser(ctx, ctx.userId);
    return null;
  },
});
