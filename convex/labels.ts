// labels.* (ARCHITECTURE §4.2). Writes enqueue label ops; the worker applies them at the provider.
import { v } from "convex/values";
import type { Doc } from "./_generated/dataModel";
import { entitledMutation, entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { checkClientOpId, insertOp, opsForClientOp } from "./lib/ops";
import { consume } from "./lib/rateLimits";
import { bumpRev } from "./lib/revs";
import { toLabelView, vLabelView } from "./lib/rows";
import { labelKeyOf } from "./lib/threads";

const COLOR_RE = /^#[0-9a-fA-F]{6}$/;

function checkName(name: string): string {
  const n = name.trim();
  if (n.length === 0 || n.length > 225)
    throw appError("invalid_argument", "Label names are 1 to 225 characters.");
  if (n.toLowerCase().startsWith("akatsuki/")) throw appError("invalid_argument", "That name is reserved.");
  return n;
}

export const list = entitledQuery({
  args: { accountId: v.optional(v.id("mailAccounts")) },
  returns: v.array(vLabelView),
  handler: async (ctx, { accountId }) => {
    const accounts: Doc<"mailAccounts">[] = accountId
      ? [await getOwned(ctx, accountId, ctx.userId)]
      : await ctx.db
          .query("mailAccounts")
          .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
          .take(100);
    const out = [];
    for (const a of accounts) {
      const labels = await ctx.db
        .query("labels")
        .withIndex("by_account", (q) => q.eq("accountId", a._id))
        .take(2000);
      out.push(...labels.filter((l) => !l.pendingDelete).map(toLabelView));
    }
    return out;
  },
});

export const create = entitledMutation({
  args: {
    clientOpId: v.string(),
    accountId: v.id("mailAccounts"),
    name: v.string(),
    color: v.optional(v.string()),
  },
  returns: v.object({ labelId: v.id("labels"), labelKey: v.string(), opId: v.id("ops") }),
  handler: async (ctx, { clientOpId, accountId, name, color }) => {
    checkClientOpId(clientOpId);
    const replay = await opsForClientOp(ctx, ctx.userId, clientOpId);
    const first = replay[0];
    if (first && first.payload.kind === "label_create") {
      const label = await ctx.db.get(first.payload.labelId);
      if (label) return { labelId: label._id, labelKey: labelKeyOf(label), opId: first._id };
    }
    await consume(ctx, "labels", ctx.userId);
    const account = await getOwned(ctx, accountId, ctx.userId);
    if (account.status === "removing") throw appError("not_found");
    if (color !== undefined && !COLOR_RE.test(color)) throw appError("invalid_argument", "Bad colour.");
    const n = checkName(name);
    const siblings = await ctx.db
      .query("labels")
      .withIndex("by_account", (q) => q.eq("accountId", accountId))
      .take(5000);
    if (siblings.some((l) => !l.pendingDelete && l.name.toLowerCase() === n.toLowerCase())) {
      throw appError("invalid_argument", "A label with that name already exists.");
    }
    const rev = await bumpRev(ctx, accountId);
    const labelId = await ctx.db.insert("labels", {
      userId: ctx.userId,
      accountId,
      name: n,
      // Gmail labels, Graph categories and IMAP keywords are non-exclusive; folders come from sync.
      kind: account.provider === "gmail" ? "user" : account.provider === "graph" ? "category" : "keyword",
      exclusive: false,
      color,
      hidden: false,
      pendingDelete: false,
      rev,
    });
    const opId = await insertOp(ctx, {
      userId: ctx.userId,
      account,
      clientOpId,
      payload: { kind: "label_create", labelId },
      labelId,
    });
    return { labelId, labelKey: `l:${labelId}`, opId };
  },
});

export const rename = entitledMutation({
  args: { clientOpId: v.string(), labelId: v.id("labels"), name: v.string() },
  returns: v.object({ opId: v.id("ops") }),
  handler: async (ctx, { clientOpId, labelId, name }) => {
    checkClientOpId(clientOpId);
    const replay = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (replay[0]) return { opId: replay[0]._id };
    await consume(ctx, "labels", ctx.userId);
    const label = await getOwned(ctx, labelId, ctx.userId);
    if (label.kind === "system" || label.role)
      throw appError("invalid_action", "System labels cannot be renamed.");
    const account = await ctx.db.get(label.accountId);
    if (!account) throw appError("not_found");
    const n = checkName(name);
    await ctx.db.patch(labelId, { name: n, rev: await bumpRev(ctx, label.accountId) });
    const opId = await insertOp(ctx, {
      userId: ctx.userId,
      account,
      clientOpId,
      payload: { kind: "label_rename", labelId, name: n },
      labelId,
    });
    return { opId };
  },
});

export const remove = entitledMutation({
  args: { clientOpId: v.string(), labelId: v.id("labels") },
  returns: v.object({ opId: v.id("ops") }),
  handler: async (ctx, { clientOpId, labelId }) => {
    checkClientOpId(clientOpId);
    const replay = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (replay[0]) return { opId: replay[0]._id };
    await consume(ctx, "labels", ctx.userId);
    const label = await getOwned(ctx, labelId, ctx.userId);
    if (label.kind === "system" || label.role)
      throw appError("invalid_action", "System labels cannot be deleted.");
    const account = await ctx.db.get(label.accountId);
    if (!account) throw appError("not_found");
    await ctx.db.patch(labelId, { pendingDelete: true, rev: await bumpRev(ctx, label.accountId) });
    const opId = await insertOp(ctx, {
      userId: ctx.userId,
      account,
      clientOpId,
      payload: { kind: "label_delete", labelId },
      labelId,
    });
    return { opId };
  },
});
