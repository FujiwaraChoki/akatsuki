// identities.* (ARCHITECTURE §4.2). Send-as addresses; signatures are sanitized on write.
import { v } from "convex/values";
import { entitledMutation, entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { project, vIdentityView } from "./lib/rows";
import { sanitizeUserHtml } from "./lib/sanitizeUserHtml";

export const list = entitledQuery({
  args: { accountId: v.optional(v.id("mailAccounts")) },
  returns: v.array(vIdentityView),
  handler: async (ctx, { accountId }) => {
    if (accountId) {
      await getOwned(ctx, accountId, ctx.userId);
      const rows = await ctx.db
        .query("identities")
        .withIndex("by_account", (q) => q.eq("accountId", accountId))
        .take(200);
      return rows.map((i) => project(vIdentityView, i));
    }
    const rows = await ctx.db
      .query("identities")
      .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
      .take(500);
    return rows.map((i) => project(vIdentityView, i));
  },
});

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export const update = entitledMutation({
  args: {
    identityId: v.id("identities"),
    name: v.optional(v.string()),
    replyTo: v.optional(v.string()),
    signatureHtml: v.optional(v.string()),
    isDefault: v.optional(v.boolean()),
  },
  returns: v.null(),
  handler: async (ctx, { identityId, name, replyTo, signatureHtml, isDefault }) => {
    const identity = await getOwned(ctx, identityId, ctx.userId);
    const patch: Record<string, unknown> = {};
    if (name !== undefined) {
      if (name.length > 120) throw appError("invalid_argument", "Name is too long.");
      patch.name = name;
    }
    if (replyTo !== undefined) {
      if (replyTo !== "" && !EMAIL_RE.test(replyTo))
        throw appError("invalid_argument", "Bad reply-to address.");
      patch.replyTo = replyTo === "" ? undefined : replyTo.trim().toLowerCase();
    }
    if (signatureHtml !== undefined) {
      const clean = sanitizeUserHtml(signatureHtml);
      if (clean.length > 10 * 1024) throw appError("payload_too_large", "Signatures are limited to 10 KB.");
      patch.signatureHtml = clean;
    }
    if (isDefault === true) {
      const siblings = await ctx.db
        .query("identities")
        .withIndex("by_account", (q) => q.eq("accountId", identity.accountId))
        .collect();
      for (const s of siblings)
        if (s._id !== identityId && s.isDefault) await ctx.db.patch(s._id, { isDefault: false });
      patch.isDefault = true;
    } else if (isDefault === false) {
      patch.isDefault = false;
    }
    await ctx.db.patch(identityId, patch);
    return null;
  },
});
