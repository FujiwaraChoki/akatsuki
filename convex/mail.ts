// mail.* (ARCHITECTURE §4.2, §5.2, §10): lists, thread view, actions, undo, snooze, reminders.
import { paginationOptsValidator, paginationResultValidator } from "convex/server";
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Doc, Id } from "./_generated/dataModel";
import type { MutationCtx } from "./_generated/server";
import { entitledMutation, entitledQuery, getOwned } from "./lib/auth";
import { appError } from "./lib/errors";
import { isLabelKey } from "./lib/model";
import {
  checkClientOpId,
  enqueueThreadAction,
  inverseAction,
  MAX_THREADS_PER_CALL,
  opsForClientOp,
} from "./lib/ops";
import { consume } from "./lib/rateLimits";
import { currentRev } from "./lib/revs";
import {
  project,
  toMessageMeta,
  toOpSummary,
  toThreadRow,
  vMessageMeta,
  vOpSummary,
  vThreadRow,
  vTimerView,
} from "./lib/rows";
import { pendingOpsForThread, recomputeThread } from "./lib/threads";
import { activeTimer, cancelTimer, fireTimer, snoozeAction } from "./lib/timers";
import { vProvider, vThreadAction } from "./lib/validators";

// ───────────── queries ─────────────

export const listThreads = entitledQuery({
  args: {
    view: v.object({
      labelKey: v.string(), // a label key or "all"
      accountId: v.optional(v.id("mailAccounts")),
      unreadOnly: v.optional(v.boolean()),
    }),
    paginationOpts: paginationOptsValidator,
  },
  returns: paginationResultValidator(vThreadRow),
  handler: async (ctx, { view, paginationOpts }) => {
    const { labelKey, accountId, unreadOnly } = view;
    if (accountId) await getOwned(ctx, accountId, ctx.userId);
    if (labelKey === "all") {
      const base = accountId
        ? ctx.db.query("threads").withIndex("by_account_time", (q) => q.eq("accountId", accountId))
        : ctx.db.query("threads").withIndex("by_user_time", (q) => q.eq("userId", ctx.userId));
      const result = await base
        .order("desc")
        .filter((q) =>
          unreadOnly
            ? q.and(q.eq(q.field("deletedPending"), false), q.gt(q.field("unreadCount"), 0))
            : q.eq(q.field("deletedPending"), false),
        )
        .paginate(paginationOpts);
      return { ...result, page: result.page.map(toThreadRow) };
    }
    if (!isLabelKey(labelKey)) throw appError("invalid_argument", "Unknown label key.");
    const uid = ctx.userId;
    const q = accountId
      ? unreadOnly
        ? ctx.db
            .query("threadLabels")
            .withIndex("by_account_label_unread_sort", (q) =>
              q.eq("accountId", accountId).eq("labelKey", labelKey).eq("isUnread", true),
            )
        : ctx.db
            .query("threadLabels")
            .withIndex("by_account_label_sort", (q) => q.eq("accountId", accountId).eq("labelKey", labelKey))
      : unreadOnly
        ? ctx.db
            .query("threadLabels")
            .withIndex("by_user_label_unread_sort", (q) =>
              q.eq("userId", uid).eq("labelKey", labelKey).eq("isUnread", true),
            )
        : ctx.db
            .query("threadLabels")
            .withIndex("by_user_label_sort", (q) => q.eq("userId", uid).eq("labelKey", labelKey));
    const result = await q.order("desc").paginate(paginationOpts);
    const threads = await Promise.all(result.page.map((row) => ctx.db.get(row.threadId)));
    return {
      ...result,
      page: threads.filter((t): t is Doc<"threads"> => t !== null && !t.deletedPending).map(toThreadRow),
    };
  },
});

export const getThread = entitledQuery({
  args: { threadId: v.id("threads") },
  returns: v.object({
    thread: vThreadRow,
    account: v.object({
      _id: v.id("mailAccounts"),
      provider: vProvider,
      email: v.string(),
      caps: v.object({
        multiLabel: v.boolean(),
        categories: v.boolean(),
        serverSearch: v.boolean(),
        deleteForever: v.boolean(),
      }),
    }),
    messages: v.array(vMessageMeta),
    truncated: v.boolean(),
    pendingOps: v.array(vOpSummary),
    timers: v.array(vTimerView),
  }),
  handler: async (ctx, { threadId }) => {
    const thread = await getOwned(ctx, threadId, ctx.userId);
    const account = await ctx.db.get(thread.accountId);
    if (!account) throw appError("not_found");
    const newest = await ctx.db
      .query("messages")
      .withIndex("by_thread_date", (q) => q.eq("threadId", threadId))
      .order("desc")
      .take(501);
    const truncated = newest.length > 500;
    const messages = newest.slice(0, 500).reverse().map(toMessageMeta);
    const ops = await pendingOpsForThread(ctx, threadId);
    const timers: Doc<"timers">[] = [];
    for (const kind of ["snooze", "reminder"] as const) {
      const t = await ctx.db
        .query("timers")
        .withIndex("by_thread_kind_state", (q) =>
          q.eq("threadId", threadId).eq("kind", kind).eq("state", "active"),
        )
        .take(5);
      timers.push(...t);
    }
    return {
      thread: toThreadRow(thread),
      account: { _id: account._id, provider: account.provider, email: account.email, caps: account.caps },
      messages,
      truncated,
      pendingOps: ops.map(toOpSummary),
      timers: timers.map((t) => project(vTimerView, t)),
    };
  },
});

export const olderMessages = entitledQuery({
  args: { threadId: v.id("threads"), beforeDate: v.number(), limit: v.number() },
  returns: v.array(vMessageMeta),
  handler: async (ctx, { threadId, beforeDate, limit }) => {
    await getOwned(ctx, threadId, ctx.userId);
    const n = Math.max(1, Math.min(200, Math.floor(limit)));
    const rows = await ctx.db
      .query("messages")
      .withIndex("by_thread_date", (q) => q.eq("threadId", threadId).lt("date", beforeDate))
      .order("desc")
      .take(n);
    return rows.reverse().map(toMessageMeta);
  },
});

export const unreadCounts = entitledQuery({
  args: { labelKeys: v.array(v.string()), accountId: v.optional(v.id("mailAccounts")) },
  returns: v.record(v.string(), v.number()),
  handler: async (ctx, { labelKeys, accountId }) => {
    if (labelKeys.length > 30) throw appError("invalid_argument", "At most 30 label keys.");
    if (accountId) await getOwned(ctx, accountId, ctx.userId);
    const out: Record<string, number> = {};
    for (const key of labelKeys) {
      if (!isLabelKey(key)) throw appError("invalid_argument", "Unknown label key.");
      const rows = accountId
        ? await ctx.db
            .query("threadLabels")
            .withIndex("by_account_label_unread_sort", (q) =>
              q.eq("accountId", accountId).eq("labelKey", key).eq("isUnread", true),
            )
            .take(100)
        : await ctx.db
            .query("threadLabels")
            .withIndex("by_user_label_unread_sort", (q) =>
              q.eq("userId", ctx.userId).eq("labelKey", key).eq("isUnread", true),
            )
            .take(100);
      out[key] = rows.length; // capped at 100; the UI shows "99+"
    }
    return out;
  },
});

// ───────────── actions (§5.2) ─────────────

type Loaded = { thread: Doc<"threads">; account: Doc<"mailAccounts"> };

async function loadThreads(ctx: MutationCtx, userId: string, threadIds: Id<"threads">[]): Promise<Loaded[]> {
  if (threadIds.length === 0 || threadIds.length > MAX_THREADS_PER_CALL) {
    throw appError("invalid_argument", `Between 1 and ${MAX_THREADS_PER_CALL} threads per call.`);
  }
  const accounts = new Map<string, Doc<"mailAccounts">>();
  const out: Loaded[] = [];
  for (const id of [...new Set(threadIds)]) {
    const thread = await getOwned(ctx, id, userId);
    let account = accounts.get(thread.accountId);
    if (!account) {
      const a = await ctx.db.get(thread.accountId);
      if (!a || a.status === "removing") throw appError("not_found");
      account = a;
      accounts.set(a._id, a);
    }
    out.push({ thread, account });
  }
  return out;
}

async function revsFor(ctx: MutationCtx, accountIds: Iterable<Id<"mailAccounts">>) {
  const rev: Record<string, number> = {};
  for (const id of new Set(accountIds)) rev[id] = await currentRev(ctx, id);
  return rev;
}

export const apply = entitledMutation({
  args: { clientOpId: v.string(), threadIds: v.array(v.id("threads")), action: vThreadAction },
  returns: v.object({ opIds: v.array(v.id("ops")), rev: v.record(v.string(), v.number()) }),
  handler: async (ctx, { clientOpId, threadIds, action }) => {
    checkClientOpId(clientOpId);
    await consume(ctx, "mailApply", ctx.userId);
    const existing = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (existing.length > 0) {
      // Replay of the same gesture: return the original ops unchanged.
      return {
        opIds: existing.map((o) => o._id),
        rev: await revsFor(
          ctx,
          existing.map((o) => o.accountId),
        ),
      };
    }
    const loaded = await loadThreads(ctx, ctx.userId, threadIds);
    const opIds: Id<"ops">[] = [];
    const rev: Record<string, number> = {};
    for (const { thread, account } of loaded) {
      const r = await enqueueThreadAction(ctx, { userId: ctx.userId, account, thread, clientOpId, action });
      opIds.push(r.opId);
      rev[account._id] = r.rev;
    }
    return { opIds, rev };
  },
});

export const undo = entitledMutation({
  args: { clientOpId: v.string(), undoClientOpId: v.string() },
  returns: v.object({
    mode: v.union(v.literal("cancelled"), v.literal("inverse"), v.literal("noop")),
    opIds: v.array(v.id("ops")),
  }),
  handler: async (ctx, { clientOpId, undoClientOpId }) => {
    checkClientOpId(clientOpId);
    const replay = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (replay.length > 0) return { mode: "inverse" as const, opIds: replay.map((o) => o._id) };
    const original = await opsForClientOp(ctx, ctx.userId, undoClientOpId);
    if (original.length === 0) throw appError("not_found");
    let cancelled = 0;
    const opIds: Id<"ops">[] = [];
    const now = Date.now();
    for (const op of original) {
      if (op.payload.kind !== "thread" || !op.threadId) continue;
      if (op.state === "queued") {
        await ctx.db.patch(op._id, { state: "cancelled", completedAt: now });
        await recomputeThread(ctx, op.threadId);
        cancelled++;
        opIds.push(op._id);
        continue;
      }
      if (op.state === "cancelled") continue;
      const thread = await ctx.db.get(op.threadId);
      const account = thread ? await ctx.db.get(thread.accountId) : null;
      if (!thread || !account) continue;
      const inverse = inverseAction(op, account);
      if (!inverse) continue;
      const r = await enqueueThreadAction(ctx, {
        userId: ctx.userId,
        account,
        thread,
        clientOpId,
        action: inverse,
        inverseOf: op._id,
      });
      opIds.push(r.opId);
    }
    if (opIds.length === 0) return { mode: "noop" as const, opIds };
    return { mode: cancelled === opIds.length ? ("cancelled" as const) : ("inverse" as const), opIds };
  },
});

// ───────────── snooze and reminders (§10) ─────────────

export const snooze = entitledMutation({
  args: {
    clientOpId: v.string(),
    threadIds: v.array(v.id("threads")),
    until: v.number(),
    returnUnread: v.boolean(),
  },
  returns: v.object({ timerIds: v.array(v.id("timers")) }),
  handler: async (ctx, { clientOpId, threadIds, until, returnUnread }) => {
    checkClientOpId(clientOpId);
    const now = Date.now();
    if (until <= now) throw appError("invalid_argument", "Snooze time must be in the future.");
    const existing = await opsForClientOp(ctx, ctx.userId, clientOpId);
    if (existing.length > 0) {
      const timerIds: Id<"timers">[] = [];
      for (const op of existing) {
        if (!op.threadId) continue;
        const t = await activeTimer(ctx, op.threadId, "snooze");
        if (t) timerIds.push(t._id);
      }
      return { timerIds };
    }
    const loaded = await loadThreads(ctx, ctx.userId, threadIds);
    const timerIds: Id<"timers">[] = [];
    for (const { thread, account } of loaded) {
      const previous = await activeTimer(ctx, thread._id, "snooze");
      if (previous) await cancelTimer(ctx, previous);
      const timerId = await ctx.db.insert("timers", {
        userId: ctx.userId,
        accountId: account._id,
        threadId: thread._id,
        kind: "snooze",
        fireAt: until,
        condition: "always",
        anchorAt: now,
        returnUnread,
        state: "active",
        createdAt: now,
      });
      const scheduledFnId = await ctx.scheduler.runAt(until, internal.timers.fire, { timerId });
      await ctx.db.patch(timerId, { scheduledFnId });
      await ctx.db.patch(thread._id, { snoozedUntil: until });
      const fresh = (await ctx.db.get(thread._id))!;
      // A re-snooze of an already snoozed thread has nothing to change at the provider.
      if (!previous) {
        await enqueueThreadAction(ctx, {
          userId: ctx.userId,
          account,
          thread: fresh,
          clientOpId,
          action: snoozeAction(account),
        });
      } else {
        await recomputeThread(ctx, thread._id);
      }
      timerIds.push(timerId);
    }
    return { timerIds };
  },
});

export const unsnooze = entitledMutation({
  args: { clientOpId: v.string(), threadIds: v.array(v.id("threads")) },
  returns: v.null(),
  handler: async (ctx, { clientOpId, threadIds }) => {
    checkClientOpId(clientOpId);
    if ((await opsForClientOp(ctx, ctx.userId, clientOpId)).length > 0) return null;
    const loaded = await loadThreads(ctx, ctx.userId, threadIds);
    for (const { thread } of loaded) {
      const timer = await activeTimer(ctx, thread._id, "snooze");
      if (timer) await fireTimer(ctx, timer._id, { early: true, clientOpId });
    }
    return null;
  },
});

export const setReminder = entitledMutation({
  args: {
    threadId: v.id("threads"),
    at: v.number(),
    condition: v.union(v.literal("always"), v.literal("if_no_reply")),
    note: v.optional(v.string()),
  },
  returns: v.object({ timerId: v.id("timers") }),
  handler: async (ctx, { threadId, at, condition, note }) => {
    const thread = await getOwned(ctx, threadId, ctx.userId);
    const now = Date.now();
    if (at <= now) throw appError("invalid_argument", "Reminder time must be in the future.");
    if (note !== undefined && note.length > 500) throw appError("invalid_argument", "Note is too long.");
    const previous = await activeTimer(ctx, threadId, "reminder");
    if (previous) await cancelTimer(ctx, previous);
    const timerId = await ctx.db.insert("timers", {
      userId: ctx.userId,
      accountId: thread.accountId,
      threadId,
      kind: "reminder",
      fireAt: at,
      condition,
      anchorAt: now,
      returnUnread: false,
      note,
      state: "active",
      createdAt: now,
    });
    const scheduledFnId = await ctx.scheduler.runAt(at, internal.timers.fire, { timerId });
    await ctx.db.patch(timerId, { scheduledFnId });
    await ctx.db.patch(threadId, { reminderAt: at });
    await recomputeThread(ctx, threadId);
    return { timerId };
  },
});

export const cancelReminder = entitledMutation({
  args: { timerId: v.id("timers") },
  returns: v.null(),
  handler: async (ctx, { timerId }) => {
    const timer = await getOwned(ctx, timerId, ctx.userId);
    if (timer.kind !== "reminder") throw appError("invalid_argument", "Not a reminder.");
    if (timer.state !== "active") return null;
    await cancelTimer(ctx, timer);
    await ctx.db.patch(timer.threadId, { reminderAt: undefined });
    await recomputeThread(ctx, timer.threadId);
    return null;
  },
});

export const listTimers = entitledQuery({
  args: { kind: v.union(v.literal("snooze"), v.literal("reminder")) },
  returns: v.array(vTimerView),
  handler: async (ctx, { kind }) => {
    const rows = await ctx.db
      .query("timers")
      .withIndex("by_user_kind_state_fire", (q) =>
        q.eq("userId", ctx.userId).eq("kind", kind).eq("state", "active"),
      )
      .take(500);
    return rows.map((t) => project(vTimerView, t));
  },
});
