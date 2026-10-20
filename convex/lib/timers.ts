// Snooze and reminder effects (ARCHITECTURE §10). Convex is the clock; the provider effect is an op.
import { internal } from "../_generated/api";
import type { Doc, Id } from "../_generated/dataModel";
import type { MutationCtx } from "../_generated/server";
import { SYS, type ThreadActionLike } from "./model";
import { enqueueThreadAction } from "./ops";
import { recomputeThread } from "./threads";

export function snoozeAction(account: Doc<"mailAccounts">): ThreadActionLike {
  return account.caps.multiLabel
    ? { kind: "modify", add: [SYS.snoozed], remove: [SYS.inbox] }
    : { kind: "move", to: SYS.snoozed };
}

/** Returns a thread to the inbox (snooze wake or reminder fire). */
export async function returnToInbox(
  ctx: MutationCtx,
  args: {
    thread: Doc<"threads">;
    account: Doc<"mailAccounts">;
    clientOpId: string;
    fromSnooze: boolean;
    markUnread: boolean;
  },
): Promise<void> {
  const { thread, account, clientOpId, fromSnooze, markUnread } = args;
  const now = Date.now();
  await ctx.db.patch(thread._id, {
    ...(fromSnooze ? { snoozedUntil: undefined } : { reminderAt: undefined }),
    returnedAt: now,
    sortAt: Math.max(thread.lastMessageAt, now),
  });
  const fresh = (await ctx.db.get(thread._id))!;
  if (account.caps.multiLabel) {
    const add = [SYS.inbox, ...(markUnread ? [SYS.unread] : [])];
    const remove = fromSnooze ? [SYS.snoozed] : [];
    await enqueueThreadAction(ctx, {
      userId: thread.userId,
      account,
      thread: fresh,
      clientOpId,
      action: { kind: "modify", add, remove },
    });
  } else {
    await enqueueThreadAction(ctx, {
      userId: thread.userId,
      account,
      thread: fresh,
      clientOpId,
      action: { kind: "move", to: SYS.inbox },
    });
    if (markUnread) {
      const again = (await ctx.db.get(thread._id))!;
      await enqueueThreadAction(ctx, {
        userId: thread.userId,
        account,
        thread: again,
        clientOpId,
        action: { kind: "modify", add: [SYS.unread], remove: [] },
      });
    }
  }
  await recomputeThread(ctx, thread._id);
}

export async function activeTimer(ctx: MutationCtx, threadId: Id<"threads">, kind: "snooze" | "reminder") {
  return await ctx.db
    .query("timers")
    .withIndex("by_thread_kind_state", (q) =>
      q.eq("threadId", threadId).eq("kind", kind).eq("state", "active"),
    )
    .first();
}

export async function cancelTimer(ctx: MutationCtx, timer: Doc<"timers">): Promise<void> {
  if (timer.scheduledFnId) await ctx.scheduler.cancel(timer.scheduledFnId);
  await ctx.db.patch(timer._id, { state: "cancelled" });
}

async function notifyReturned(ctx: MutationCtx, userId: string, threadId: Id<"threads">) {
  const withToken = await ctx.db
    .query("devices")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .filter((q) => q.neq(q.field("expoPushToken"), undefined))
    .first();
  if (withToken) await ctx.scheduler.runAfter(0, internal.notify.newMail, { userId, threadIds: [threadId] });
}

/**
 * Fires a timer if it is still active (guarded by row state, so duplicate fires do nothing).
 * `early` is the snooze early-wake path from worker.upsertThreads and mail.unsnooze.
 */
export async function fireTimer(
  ctx: MutationCtx,
  timerId: Id<"timers">,
  opts: { early?: boolean; clientOpId?: string } = {},
): Promise<"fired" | "skipped" | "noop"> {
  const timer = await ctx.db.get(timerId);
  if (timer?.state !== "active") return "noop";
  if (opts.early && timer.scheduledFnId) await ctx.scheduler.cancel(timer.scheduledFnId);
  const thread = await ctx.db.get(timer.threadId);
  const account = thread ? await ctx.db.get(thread.accountId) : null;
  if (!thread || !account || account.status === "removing") {
    await ctx.db.patch(timerId, { state: "cancelled" });
    return "noop";
  }
  if (
    timer.kind === "reminder" &&
    timer.condition === "if_no_reply" &&
    (thread.lastInboundAt ?? 0) > timer.anchorAt
  ) {
    await ctx.db.patch(timerId, { state: "skipped" });
    await ctx.db.patch(thread._id, { reminderAt: undefined });
    await recomputeThread(ctx, thread._id);
    return "skipped";
  }
  await ctx.db.patch(timerId, { state: "fired" });
  await returnToInbox(ctx, {
    thread,
    account,
    clientOpId: opts.clientOpId ?? `timer:${timerId}`,
    fromSnooze: timer.kind === "snooze",
    markUnread: timer.returnUnread,
  });
  if (!opts.early) await notifyReturned(ctx, thread.userId, thread._id);
  return "fired";
}
