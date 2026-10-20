// accounts.* (ARCHITECTURE §4.2) and the purge jobs (§4.5).
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Id, TableNames } from "./_generated/dataModel";
import { internalMutation, type MutationCtx } from "./_generated/server";
import { billingDisabled } from "./ee/entitlement";
import { entitledMutation, getOwned, userMutation, userQuery } from "./lib/auth";
import { appError } from "./lib/errors";
import { toMailAccountView, vMailAccountView } from "./lib/rows";

export const list = userQuery({
  args: {},
  returns: v.array(vMailAccountView),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("mailAccounts")
      .withIndex("by_user", (q) => q.eq("userId", ctx.userId))
      .take(100);
    return rows.map(toMailAccountView);
  },
});

const COLOR_RE = /^#[0-9a-fA-F]{6}$/;

export const update = entitledMutation({
  args: {
    accountId: v.id("mailAccounts"),
    displayName: v.optional(v.string()),
    color: v.optional(v.string()),
    sortOrder: v.optional(v.number()),
  },
  returns: v.null(),
  handler: async (ctx, { accountId, ...patch }) => {
    await getOwned(ctx, accountId, ctx.userId);
    if (patch.color !== undefined && !COLOR_RE.test(patch.color))
      throw appError("invalid_argument", "Bad colour.");
    if (patch.displayName !== undefined && patch.displayName.length > 120) {
      throw appError("invalid_argument", "Name is too long.");
    }
    await ctx.db.patch(accountId, patch);
    return null;
  },
});

export const setBackfillHorizon = entitledMutation({
  args: { accountId: v.id("mailAccounts"), horizonAt: v.number() },
  returns: v.null(),
  handler: async (ctx, { accountId, horizonAt }) => {
    const account = await getOwned(ctx, accountId, ctx.userId);
    if (horizonAt < 0 || (horizonAt !== 0 && horizonAt > account.windowStartAt)) {
      throw appError("invalid_argument", "The horizon must be 0 (everything) or before the sync window.");
    }
    await ctx.db.patch(accountId, { backfillHorizonAt: horizonAt });
    return null;
  },
});

export const remove = userMutation({
  args: { accountId: v.id("mailAccounts") },
  returns: v.null(),
  handler: async (ctx, { accountId }) => {
    await getOwned(ctx, accountId, ctx.userId);
    // The worker stops the watch, revokes the grant and calls worker.ackAccountRemoved, which purges.
    await ctx.db.patch(accountId, { status: "removing" });
    return null;
  },
});

// ───────────── purge ─────────────

const BATCH = 1000;

export const PURGE_TABLES = [
  "threads", // with their threadLabels, threadSearch and timers
  "messages",
  "labels",
  "ops",
  "identities",
  "folderCursors",
  "accountSecrets",
  "syncState",
  "tombstones",
  "accountRevs",
] as const;
type PurgeTable = (typeof PURGE_TABLES)[number];

async function batchFor(ctx: MutationCtx, table: PurgeTable, accountId: Id<"mailAccounts">, n: number) {
  switch (table) {
    case "threads":
      return await ctx.db
        .query("threads")
        .withIndex("by_account_time", (q) => q.eq("accountId", accountId))
        .take(n);
    case "messages":
      return await ctx.db
        .query("messages")
        .withIndex("by_account_provider", (q) => q.eq("accountId", accountId))
        .take(n);
    case "labels":
      return await ctx.db
        .query("labels")
        .withIndex("by_account", (q) => q.eq("accountId", accountId))
        .take(n);
    case "ops":
      return await ctx.db
        .query("ops")
        .withIndex("by_account_state", (q) => q.eq("accountId", accountId))
        .take(n);
    case "identities":
      return await ctx.db
        .query("identities")
        .withIndex("by_account", (q) => q.eq("accountId", accountId))
        .take(n);
    case "folderCursors":
      return await ctx.db
        .query("folderCursors")
        .withIndex("by_account_folder", (q) => q.eq("accountId", accountId))
        .take(n);
    case "accountSecrets":
      return await ctx.db
        .query("accountSecrets")
        .withIndex("by_account_kind", (q) => q.eq("accountId", accountId))
        .take(n);
    case "syncState":
      return await ctx.db
        .query("syncState")
        .withIndex("by_account", (q) => q.eq("accountId", accountId))
        .take(n);
    case "tombstones":
      return await ctx.db
        .query("tombstones")
        .withIndex("by_account_rev", (q) => q.eq("accountId", accountId))
        .take(n);
    case "accountRevs":
      return await ctx.db
        .query("accountRevs")
        .withIndex("by_account", (q) => q.eq("accountId", accountId))
        .take(n);
  }
}

async function deleteThreadDependents(ctx: MutationCtx, threadId: Id<"threads">) {
  for (const row of await ctx.db
    .query("threadLabels")
    .withIndex("by_thread", (q) => q.eq("threadId", threadId))
    .collect())
    await ctx.db.delete(row._id);
  for (const row of await ctx.db
    .query("threadSearch")
    .withIndex("by_thread", (q) => q.eq("threadId", threadId))
    .collect())
    await ctx.db.delete(row._id);
  for (const row of await ctx.db
    .query("timers")
    .withIndex("by_thread_kind_state", (q) => q.eq("threadId", threadId))
    .collect()) {
    if (row.scheduledFnId) await ctx.scheduler.cancel(row.scheduledFnId);
    await ctx.db.delete(row._id);
  }
}

/** Deletes up to 1,000 documents per run, reschedules itself, deletes mailAccounts last. */
export const purge = internalMutation({
  args: { accountId: v.id("mailAccounts"), table: v.string(), cursor: v.union(v.string(), v.null()) },
  returns: v.null(),
  handler: async (ctx, { accountId, table }) => {
    let index = PURGE_TABLES.indexOf(table as PurgeTable);
    if (index < 0) index = 0;
    let budget = BATCH;
    while (index < PURGE_TABLES.length && budget > 0) {
      const t = PURGE_TABLES[index]!;
      // Threads drag their dependents along, so take fewer of them per run.
      const take = t === "threads" ? Math.min(budget, 100) : budget;
      const docs = await batchFor(ctx, t, accountId, take);
      for (const d of docs) {
        if (t === "threads") await deleteThreadDependents(ctx, d._id as Id<"threads">);
        await ctx.db.delete(d._id);
      }
      budget -= docs.length;
      if (docs.length < take) index++;
      else break;
    }
    if (index < PURGE_TABLES.length) {
      await ctx.scheduler.runAfter(0, internal.accounts.purge, {
        accountId,
        table: PURGE_TABLES[index]!,
        cursor: null,
      });
      return null;
    }
    const account = await ctx.db.get(accountId);
    if (account) {
      // Drafts and scheduled sends reference the account but are indexed by user.
      for (const d of await ctx.db
        .query("drafts")
        .withIndex("by_user_updated", (q) => q.eq("userId", account.userId))
        .collect()) {
        if (d.accountId === accountId) await ctx.db.delete(d._id);
      }
      for (const s of await ctx.db
        .query("scheduledSends")
        .withIndex("by_user_client", (q) => q.eq("userId", account.userId))
        .collect()) {
        if (s.accountId === accountId) {
          if (s.scheduledFnId) await ctx.scheduler.cancel(s.scheduledFnId);
          await ctx.db.delete(s._id);
        }
      }
      await ctx.db.delete(accountId);
    }
    return null;
  },
});

const USER_TABLES = [
  "devices",
  "splits",
  "rules",
  "snippets",
  "senderPrefs",
  "readReceipts",
  "contacts",
  "drafts",
  "scheduledSends",
  "aiSettings",
  "aiUsage",
  "profiles",
  "billing",
] as const satisfies readonly TableNames[];

async function userBatch(ctx: MutationCtx, table: (typeof USER_TABLES)[number], userId: string, n: number) {
  switch (table) {
    case "devices":
      return await ctx.db
        .query("devices")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .take(n);
    case "splits":
      return await ctx.db
        .query("splits")
        .withIndex("by_user_order", (q) => q.eq("userId", userId))
        .take(n);
    case "rules":
      return await ctx.db
        .query("rules")
        .withIndex("by_user_order", (q) => q.eq("userId", userId))
        .take(n);
    case "snippets":
      return await ctx.db
        .query("snippets")
        .withIndex("by_user_name", (q) => q.eq("userId", userId))
        .take(n);
    case "senderPrefs":
      return await ctx.db
        .query("senderPrefs")
        .withIndex("by_user_sender", (q) => q.eq("userId", userId))
        .take(n);
    case "readReceipts":
      return await ctx.db
        .query("readReceipts")
        .withIndex("by_user_rfc", (q) => q.eq("userId", userId))
        .take(n);
    case "contacts":
      return await ctx.db
        .query("contacts")
        .withIndex("by_user_email", (q) => q.eq("userId", userId))
        .take(n);
    case "drafts":
      return await ctx.db
        .query("drafts")
        .withIndex("by_user_updated", (q) => q.eq("userId", userId))
        .take(n);
    case "scheduledSends":
      return await ctx.db
        .query("scheduledSends")
        .withIndex("by_user_client", (q) => q.eq("userId", userId))
        .take(n);
    case "aiSettings":
      return await ctx.db
        .query("aiSettings")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .take(n);
    case "aiUsage":
      return await ctx.db
        .query("aiUsage")
        .withIndex("by_user_period", (q) => q.eq("userId", userId))
        .take(n);
    case "profiles":
      return await ctx.db
        .query("profiles")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .take(n);
    case "billing":
      return await ctx.db
        .query("billing")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .take(n);
  }
}

/** Deletes every row of a user: accounts through `purge`, then user-level tables. */
export const purgeUser = internalMutation({
  args: { userId: v.string() },
  returns: v.null(),
  handler: async (ctx, { userId }) => {
    const accounts = await ctx.db
      .query("mailAccounts")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .collect();
    for (const a of accounts) {
      if (a.status !== "removing") await ctx.db.patch(a._id, { status: "removing" });
      await ctx.scheduler.runAfter(0, internal.accounts.purge, {
        accountId: a._id,
        table: PURGE_TABLES[0],
        cursor: null,
      });
    }
    let budget = BATCH;
    for (const table of USER_TABLES) {
      const docs = await userBatch(ctx, table, userId, budget);
      for (const d of docs) {
        const fn = (d as { scheduledFnId?: Id<"_scheduled_functions"> }).scheduledFnId;
        if (fn) await ctx.scheduler.cancel(fn);
        await ctx.db.delete(d._id);
      }
      budget -= docs.length;
      if (budget <= 0) {
        await ctx.scheduler.runAfter(0, internal.accounts.purgeUser, { userId });
        return null;
      }
    }
    return null;
  },
});

/** Daily safety net for §13.3: pause accounts of users whose entitlement lapsed. */
export const pauseLapsed = internalMutation({
  args: { cursor: v.optional(v.union(v.string(), v.null())) },
  returns: v.null(),
  handler: async (ctx, { cursor }) => {
    if (billingDisabled()) return null;
    const page = await ctx.db.query("billing").paginate({ numItems: 200, cursor: cursor ?? null });
    for (const b of page.page) {
      if (b.entitlement.hostedSync) continue;
      const accounts = await ctx.db
        .query("mailAccounts")
        .withIndex("by_user", (q) => q.eq("userId", b.userId))
        .collect();
      for (const a of accounts) {
        if (a.status !== "paused" && a.status !== "removing") await ctx.db.patch(a._id, { status: "paused" });
      }
    }
    if (!page.isDone)
      await ctx.scheduler.runAfter(0, internal.accounts.pauseLapsed, { cursor: page.continueCursor });
    return null;
  },
});

export const assertNotRemoving = (status: string) => {
  if (status === "removing") throw appError("not_found");
};

// ───────────── reauth reminder (§9.7) ─────────────

/** How long an account sits in `reauth_required` before the owner gets an email. */
export const REAUTH_EMAIL_AFTER_MS = 24 * 60 * 60 * 1000;

/**
 * Called on every transition into `reauth_required` (worker status reports and op auth failures): one email,
 * 24 h later, if the account still needs reconnecting then (§9.7, D-391). An earlier pending reminder for the
 * same account is replaced, so a flapping account gets one email.
 */
export async function scheduleReauthReminder(ctx: MutationCtx, accountId: Id<"mailAccounts">): Promise<void> {
  const pending = await ctx.db.system
    .query("_scheduled_functions")
    .filter((q) => q.eq(q.field("state.kind"), "pending"))
    .collect();
  for (const job of pending) {
    if (!/^accounts(\.js)?:reauthReminder$/.test(job.name)) continue;
    if ((job.args[0] as { accountId?: string } | undefined)?.accountId === accountId)
      await ctx.scheduler.cancel(job._id);
  }
  await ctx.scheduler.runAfter(REAUTH_EMAIL_AFTER_MS, internal.accounts.reauthReminder, { accountId });
}

export const reauthReminder = internalMutation({
  args: { accountId: v.id("mailAccounts") },
  returns: v.null(),
  handler: async (ctx, { accountId }) => {
    const account = await ctx.db.get(accountId);
    if (account?.status !== "reauth_required") return null;
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", account.userId))
      .unique();
    if (profile?.email)
      await ctx.scheduler.runAfter(0, internal.email.send, {
        template: "reauth_needed",
        to: profile.email,
        vars: { account: account.email },
      });
    return null;
  },
});
