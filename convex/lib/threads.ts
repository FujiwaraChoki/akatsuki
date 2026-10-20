// recomputeThread (ARCHITECTURE §3.3): the ONLY writer of threads.labelKeys, split, pendingOpCount,
// deletedPending, threadLabels and threadSearch.inInbox/isUnread/isStarred.
import type { Doc, Id } from "../_generated/dataModel";
import type { MutationCtx, QueryCtx } from "../_generated/server";
import {
  type ApplyContext,
  applyContextFor,
  applyOps,
  domainOf,
  matchRule,
  SYS,
  SYS_ROLES,
  type ThreadActionLike,
} from "./model";

export const isSysRole = (role: string | undefined): boolean =>
  role !== undefined && (SYS_ROLES as readonly string[]).includes(role);

/** The label key a label row is addressed by (§3.2): `sys:<role>` for the 9 system roles, else `l:<id>`. */
export function labelKeyOf(label: Pick<Doc<"labels">, "_id" | "role">): string {
  return isSysRole(label.role) ? `sys:${label.role}` : `l:${label._id}`;
}

import { bumpRev } from "./revs";

export type ReadCtx = Pick<QueryCtx, "db">;

/** Non-terminal ops of a thread (queued, then inflight), in creation order. */
export async function pendingOpsForThread(ctx: ReadCtx, threadId: Id<"threads">): Promise<Doc<"ops">[]> {
  const [queued, inflight] = await Promise.all([
    ctx.db
      .query("ops")
      .withIndex("by_thread_state", (q) => q.eq("threadId", threadId).eq("state", "queued"))
      .collect(),
    ctx.db
      .query("ops")
      .withIndex("by_thread_state", (q) => q.eq("threadId", threadId).eq("state", "inflight"))
      .collect(),
  ]);
  // Only thread ops change what a thread shows (and count as pending on it, §3.2).
  return [...queued, ...inflight]
    .filter((o) => o.payload.kind === "thread")
    .sort((a, b) => a._creationTime - b._creationTime);
}

export async function applyContextForAccount(
  ctx: ReadCtx,
  account: Doc<"mailAccounts">,
): Promise<ApplyContext> {
  if (account.provider === "gmail") return applyContextFor("gmail", []);
  const labels = await ctx.db
    .query("labels")
    .withIndex("by_account", (q) => q.eq("accountId", account._id))
    .collect();
  return applyContextFor(
    account.provider,
    labels.filter((l) => l.exclusive && !isSysRole(l.role)).map((l) => l._id),
  );
}

export async function isVipSender(ctx: ReadCtx, userId: string, email: string | undefined): Promise<boolean> {
  if (!email) return false;
  const e = email.toLowerCase();
  for (const sender of [e, `@${domainOf(e)}`]) {
    const pref = await ctx.db
      .query("senderPrefs")
      .withIndex("by_user_sender", (q) => q.eq("userId", userId).eq("sender", sender))
      .unique();
    if (pref?.vip) return true;
  }
  return false;
}

/** First split (by order) whose rule matches, else "split:other". Inbox threads only. */
export async function evaluateSplit(
  ctx: ReadCtx,
  thread: Doc<"threads">,
  labelKeys: string[],
): Promise<string> {
  const splits = await ctx.db
    .query("splits")
    .withIndex("by_user_order", (q) => q.eq("userId", thread.userId))
    .take(50);
  if (splits.length === 0) return "split:other";
  const facts = {
    fromEmail: thread.lastFrom?.email,
    participants: thread.participants.map((p) => p.email),
    labelKeys,
    triage: thread.triage
      ? { category: thread.triage.category, needsReply: thread.triage.needsReply }
      : undefined,
    isBulk: thread.isBulk,
    listId: thread.listId,
    subject: thread.subject,
    accountId: thread.accountId,
    vip: await isVipSender(ctx, thread.userId, thread.lastFrom?.email),
  };
  for (const s of splits) if (matchRule(s.rule, facts)) return `split:${s._id}`;
  return "split:other";
}

export interface RecomputeResult {
  labelKeys: string[];
  split?: string;
  deletedPending: boolean;
  rev: number;
}

export async function recomputeThread(
  ctx: MutationCtx,
  threadId: Id<"threads">,
): Promise<RecomputeResult | null> {
  const thread = await ctx.db.get(threadId);
  if (!thread) return null;
  const account = await ctx.db.get(thread.accountId);
  if (!account) return null;

  const ops = await pendingOpsForThread(ctx, threadId);
  const actions = ops.flatMap((o) =>
    o.payload.kind === "thread" ? [o.payload.action as ThreadActionLike] : [],
  );
  const visible = applyOps(thread.syncedLabelKeys, actions, await applyContextForAccount(ctx, account));
  const labelKeys = visible.deleted ? [] : visible.keys;
  const inInbox = labelKeys.includes(SYS.inbox);
  const isUnread = labelKeys.includes(SYS.unread);
  const split = inInbox ? await evaluateSplit(ctx, thread, labelKeys) : undefined;

  // threadLabels: one row per visible key plus one split row for inbox threads.
  const desired = new Map<string, { sortAt: number; isUnread: boolean }>();
  for (const key of labelKeys) {
    const sortAt =
      key === SYS.snoozed && thread.snoozedUntil !== undefined ? thread.snoozedUntil : thread.sortAt;
    desired.set(key, { sortAt, isUnread });
  }
  if (split) desired.set(split, { sortAt: thread.sortAt, isUnread });
  const existing = await ctx.db
    .query("threadLabels")
    .withIndex("by_thread", (q) => q.eq("threadId", threadId))
    .collect();
  for (const row of existing) {
    const want = desired.get(row.labelKey);
    if (!want) {
      await ctx.db.delete(row._id);
    } else {
      if (want.sortAt !== row.sortAt || want.isUnread !== row.isUnread) await ctx.db.patch(row._id, want);
      desired.delete(row.labelKey);
    }
  }
  for (const [labelKey, v] of desired) {
    await ctx.db.insert("threadLabels", {
      userId: thread.userId,
      accountId: thread.accountId,
      threadId,
      labelKey,
      sortAt: v.sortAt,
      isUnread: v.isUnread,
    });
  }

  const search = await ctx.db
    .query("threadSearch")
    .withIndex("by_thread", (q) => q.eq("threadId", threadId))
    .unique();
  const isStarred = labelKeys.includes(SYS.starred);
  if (
    search &&
    (search.inInbox !== inInbox || search.isUnread !== isUnread || search.isStarred !== isStarred)
  ) {
    await ctx.db.patch(search._id, { inInbox, isUnread, isStarred });
  }

  const rev = await bumpRev(ctx, thread.accountId);
  await ctx.db.patch(threadId, {
    labelKeys,
    split,
    pendingOpCount: ops.length,
    deletedPending: visible.deleted,
    rev,
    updatedAt: Date.now(),
  });
  return { labelKeys, split, deletedPending: visible.deleted, rev };
}

/** Visible keys a thread would have after one more action (used for ops.effect). */
export async function previewAction(
  ctx: ReadCtx,
  thread: Doc<"threads">,
  account: Doc<"mailAccounts">,
  action: ThreadActionLike,
): Promise<{ before: string[]; after: string[] }> {
  const ops = await pendingOpsForThread(ctx, thread._id);
  const actions = ops.flatMap((o) =>
    o.payload.kind === "thread" ? [o.payload.action as ThreadActionLike] : [],
  );
  const actx = await applyContextForAccount(ctx, account);
  const before = applyOps(thread.syncedLabelKeys, actions, actx).keys;
  const after = applyOps(thread.syncedLabelKeys, [...actions, action], actx).keys;
  return { before, after };
}
