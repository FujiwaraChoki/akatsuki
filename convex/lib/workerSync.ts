// Worker-side write paths shared by worker.upsertThreads, worker.completeOps and worker.upsertLabels
// (ARCHITECTURE §4.3, §4.4, §5.4). Bodies are never written here: headers, snippets and flags only.
import { internal } from "../_generated/api";
import type { Doc, Id } from "../_generated/dataModel";
import type { MutationCtx } from "../_generated/server";
import { triageRouteKind } from "../ai";
import { appError } from "./errors";
import {
  CAPS,
  isLabelKey,
  isSplitKey,
  matchRule,
  SYS,
  type ThreadActionLike,
  threadAggregates,
  threadKeysFromMessages,
  threadSearchText,
  truncate,
} from "./model";
import { enqueueThreadAction, opsForClientOp } from "./ops";
import { bumpRev, tombstone } from "./revs";
import { isVipSender, labelKeyOf, recomputeThread } from "./threads";
import { activeTimer, fireTimer } from "./timers";

export type Addr = { name?: string; email: string };

export interface MessageUpsert {
  providerMessageId: string;
  rfcMessageId?: string;
  inReplyTo?: string;
  references: string[];
  from?: Addr;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  replyTo: Addr[];
  subject: string;
  snippet: string;
  date: number;
  labelKeys: string[];
  isUnread: boolean;
  isDraft: boolean;
  isOutbound: boolean;
  hasAttachments: boolean;
  sizeBytes?: number;
  listId?: string;
  listUnsubscribe?: { http?: string; mailto?: string; oneClick: boolean };
  authSummary?: { spf: string; dkim: string; dmarc: string };
  imapLocations?: { remoteFolderId: string; uidValidity: number; uid: number }[];
}

export interface ThreadUpsert {
  providerThreadId: string;
  providerVersion?: bigint;
  complete: boolean;
  messages: MessageUpsert[];
  removedProviderMessageIds?: string[];
}

export type CursorUpdate =
  | { kind: "gmail"; historyId: string }
  | {
      kind: "folder";
      remoteFolderId: string;
      labelKey: string;
      uidValidity?: number;
      uidNext?: number;
      highestModSeq?: string;
      deltaLink?: string;
    }
  | { kind: "backfill"; beforeAt: number }
  | { kind: "phase"; syncPhase: "baseline" | "inbox_first" | "window" | "backfill" | "complete" };

export interface RemoteLabelIn {
  providerLabelId: string;
  name: string;
  kind: "system" | "user" | "folder" | "category" | "keyword";
  role?: Doc<"labels">["role"];
  exclusive: boolean;
  color?: string;
  hidden: boolean;
}

const normAddr = (a: Addr): Addr => {
  const email = a.email.trim().toLowerCase();
  return a.name ? { name: truncate(a.name, 200), email } : { email };
};
const normAddrs = (list: Addr[]) => list.slice(0, CAPS.addrList).map(normAddr);
const cleanKeys = (keys: string[]) =>
  [...new Set(keys.filter((k) => isLabelKey(k) && !isSplitKey(k)))].sort();

export async function requireLease(ctx: MutationCtx, accountId: Id<"mailAccounts">, workerId: string) {
  const state = await ctx.db
    .query("syncState")
    .withIndex("by_account", (q) => q.eq("accountId", accountId))
    .unique();
  if (!state || state.leaseOwner !== workerId) throw appError("lease_lost");
  const account = await ctx.db.get(accountId);
  if (!account) throw appError("not_found");
  return { state, account };
}

// ───────────── labels ─────────────

/** The folder created on folder accounts whose server has no `\\Archive` folder (D-392). */
export const ARCHIVE_FOLDER_NAME = "Archive";

export async function upsertLabelsImpl(
  ctx: MutationCtx,
  account: Doc<"mailAccounts">,
  labels: RemoteLabelIn[],
  complete: boolean,
): Promise<Record<string, string>> {
  const existing = await ctx.db
    .query("labels")
    .withIndex("by_account", (q) => q.eq("accountId", account._id))
    .collect();
  const byProvider = new Map(existing.filter((l) => l.providerLabelId).map((l) => [l.providerLabelId!, l]));
  // Rows still waiting for their label_create: a label the provider reports with the same name (or the
  // archive role, D-392) is that row, so it is adopted instead of duplicated.
  const pending = existing.filter((l) => !l.providerLabelId && !l.pendingDelete);
  const adopt = (l: RemoteLabelIn) => {
    const i = pending.findIndex(
      (p) => (l.role === "archive" && p.role === "archive") || p.name.toLowerCase() === l.name.toLowerCase(),
    );
    return i < 0 ? undefined : pending.splice(i, 1)[0];
  };
  const out: Record<string, string> = {};
  const seen = new Set<string>();
  let rev: number | undefined;
  const nextRev = async () => (rev ??= await bumpRev(ctx, account._id));
  for (const l of labels) {
    seen.add(l.providerLabelId);
    const fields = {
      name: truncate(l.name, 225),
      kind: l.kind,
      role: l.role,
      exclusive: l.exclusive,
      color: l.color,
      hidden: l.hidden,
    };
    const row = byProvider.get(l.providerLabelId);
    const adopted = row ? undefined : adopt(l);
    if (row) {
      const changed = (Object.keys(fields) as (keyof typeof fields)[]).some((k) => row[k] !== fields[k]);
      if (changed) await ctx.db.patch(row._id, { ...fields, rev: await nextRev() });
      out[l.providerLabelId] = labelKeyOf({ _id: row._id, role: l.role });
    } else if (adopted) {
      await ctx.db.patch(adopted._id, {
        providerLabelId: l.providerLabelId,
        ...fields,
        rev: await nextRev(),
      });
      byProvider.set(l.providerLabelId, { ...adopted, providerLabelId: l.providerLabelId, ...fields });
      out[l.providerLabelId] = labelKeyOf({ _id: adopted._id, role: l.role });
    } else {
      const id = await ctx.db.insert("labels", {
        userId: account.userId,
        accountId: account._id,
        providerLabelId: l.providerLabelId,
        ...fields,
        pendingDelete: false,
        rev: await nextRev(),
      });
      out[l.providerLabelId] = labelKeyOf({ _id: id, role: l.role });
    }
  }
  if (complete) {
    for (const row of existing) {
      // Labels still waiting for their label_create op have no provider id yet and stay.
      if (!row.providerLabelId || seen.has(row.providerLabelId)) continue;
      await ctx.db.delete(row._id);
      await tombstone(ctx, account._id, "labels", row._id);
    }
    // Folder accounts always have an Archive folder to archive into (§5.1): when the server has none,
    // a placeholder row stands in, and the first archive creates the folder (D-392).
    const hasArchive =
      labels.some((l) => l.role === "archive") ||
      existing.some((l) => l.role === "archive" && !l.pendingDelete);
    if (!account.caps.multiLabel && !hasArchive) {
      await ctx.db.insert("labels", {
        userId: account.userId,
        accountId: account._id,
        name: ARCHIVE_FOLDER_NAME,
        kind: "folder",
        role: "archive",
        exclusive: true,
        hidden: false,
        pendingDelete: false,
        rev: await nextRev(),
      });
    }
  }
  return out;
}

// ───────────── cursors ─────────────

export async function applyCursor(
  ctx: MutationCtx,
  state: Doc<"syncState">,
  account: Doc<"mailAccounts">,
  c: CursorUpdate,
) {
  const now = Date.now();
  switch (c.kind) {
    case "gmail":
      if (!/^\d+$/.test(c.historyId))
        throw appError("invalid_argument", "historyId must be a decimal string.");
      await ctx.db.patch(state._id, { gmailHistoryId: c.historyId, updatedAt: now });
      return;
    case "backfill":
      await ctx.db.patch(state._id, { backfillBeforeAt: c.beforeAt, updatedAt: now });
      return;
    case "phase":
      await ctx.db.patch(account._id, { syncPhase: c.syncPhase });
      return;
    case "folder": {
      const row = await ctx.db
        .query("folderCursors")
        .withIndex("by_account_folder", (q) =>
          q.eq("accountId", account._id).eq("remoteFolderId", c.remoteFolderId),
        )
        .unique();
      const fields = {
        labelKey: c.labelKey,
        uidValidity: c.uidValidity,
        uidNext: c.uidNext,
        highestModSeq: c.highestModSeq,
        deltaLink: c.deltaLink,
        lastSyncedAt: now,
      };
      if (row) await ctx.db.patch(row._id, fields);
      else
        await ctx.db.insert("folderCursors", {
          accountId: account._id,
          remoteFolderId: c.remoteFolderId,
          ...fields,
        });
      return;
    }
  }
}

// ───────────── threads ─────────────

async function deleteThreadCompletely(ctx: MutationCtx, thread: Doc<"threads">) {
  const msgs = await ctx.db
    .query("messages")
    .withIndex("by_thread_date", (q) => q.eq("threadId", thread._id))
    .collect();
  for (const m of msgs) {
    await ctx.db.delete(m._id);
    await tombstone(ctx, thread.accountId, "messages", m._id);
  }
  for (const r of await ctx.db
    .query("threadLabels")
    .withIndex("by_thread", (q) => q.eq("threadId", thread._id))
    .collect())
    await ctx.db.delete(r._id);
  for (const r of await ctx.db
    .query("threadSearch")
    .withIndex("by_thread", (q) => q.eq("threadId", thread._id))
    .collect())
    await ctx.db.delete(r._id);
  for (const r of await ctx.db
    .query("timers")
    .withIndex("by_thread_kind_state", (q) => q.eq("threadId", thread._id))
    .collect()) {
    if (r.state === "active" && r.scheduledFnId) await ctx.scheduler.cancel(r.scheduledFnId);
    await ctx.db.delete(r._id);
  }
  await ctx.db.delete(thread._id);
  await tombstone(ctx, thread.accountId, "threads", thread._id);
}

async function bumpContact(
  ctx: MutationCtx,
  userId: string,
  addr: Addr,
  at: number,
  dir: "sent" | "received",
) {
  const email = addr.email.trim().toLowerCase();
  if (!email.includes("@")) return;
  const row = await ctx.db
    .query("contacts")
    .withIndex("by_user_email", (q) => q.eq("userId", userId).eq("email", email))
    .unique();
  const name = addr.name ?? row?.name;
  const searchText = `${name ?? ""} ${email}`.trim();
  if (row) {
    await ctx.db.patch(row._id, {
      name,
      searchText,
      lastAt: Math.max(row.lastAt, at),
      sentCount: row.sentCount + (dir === "sent" ? 1 : 0),
      receivedCount: row.receivedCount + (dir === "received" ? 1 : 0),
    });
  } else {
    await ctx.db.insert("contacts", {
      userId,
      email,
      name,
      searchText,
      lastAt: at,
      sentCount: dir === "sent" ? 1 : 0,
      receivedCount: dir === "received" ? 1 : 0,
    });
  }
}

export interface UpsertResult {
  written: number;
  skippedStale: number;
  newInbound: { threadId: Id<"threads">; isInbox: boolean }[];
}

/**
 * Whether a message dated `date` arrived after `account` was connected (D-402). Provider dates are receive
 * times (Gmail internalDate, Graph receivedDateTime, IMAP INTERNALDATE). Mail older than the connection is
 * history the user already dealt with elsewhere, whichever sync step brings it in.
 */
export function arrivedSinceConnect(account: Pick<Doc<"mailAccounts">, "createdAt">, date: number): boolean {
  return date >= account.createdAt;
}

/**
 * Folds provider message ids that are gone at the provider into the thread upserts (D-410). IMAP threads are
 * partial (`complete: false`, D-223), so an expunged or moved-away message is never dropped by omission: the
 * worker names it, and this finds its thread and adds it to that thread's `removedProviderMessageIds` (a thread
 * that is not part of the call gets an upsert with no messages, which recomputes it or deletes it when empty).
 * Ids Convex does not know, and ids the same call upserts, are ignored.
 */
async function withDeletedMessages(
  ctx: MutationCtx,
  account: Doc<"mailAccounts">,
  threads: ThreadUpsert[],
  deletedMessages: string[],
): Promise<ThreadUpsert[]> {
  if (deletedMessages.length === 0) return threads;
  const listed = new Set(threads.flatMap((t) => t.messages.map((m) => m.providerMessageId)));
  const out = threads.map((t) => ({ ...t }));
  const byProviderThread = new Map(out.map((t) => [t.providerThreadId, t]));
  const providerThreadOf = new Map<Id<"threads">, string | null>();
  for (const providerMessageId of new Set(deletedMessages)) {
    if (listed.has(providerMessageId)) continue;
    const m = await ctx.db
      .query("messages")
      .withIndex("by_account_provider", (q) =>
        q.eq("accountId", account._id).eq("providerMessageId", providerMessageId),
      )
      .unique();
    if (!m) continue;
    let providerThreadId = providerThreadOf.get(m.threadId);
    if (providerThreadId === undefined) {
      providerThreadId = (await ctx.db.get(m.threadId))?.providerThreadId ?? null;
      providerThreadOf.set(m.threadId, providerThreadId);
    }
    if (providerThreadId === null) continue;
    let tu = byProviderThread.get(providerThreadId);
    if (!tu) {
      tu = { providerThreadId, complete: false, messages: [] };
      out.push(tu);
      byProviderThread.set(providerThreadId, tu);
    }
    tu.removedProviderMessageIds = [...(tu.removedProviderMessageIds ?? []), providerMessageId];
  }
  return out;
}

/**
 * A sent message reaches Convex: its read-receipt row (created by `worker.recordDraft` for the draft with the same
 * `Message-ID`, D-412) now points at it, so `readReceipts.forMessage` finds the opens. A message that moves (IMAP)
 * is a new row, and the link follows it.
 */
async function linkReadReceipt(
  ctx: MutationCtx,
  userId: string,
  rfcMessageId: string,
  messageId: Id<"messages">,
) {
  const row = await ctx.db
    .query("readReceipts")
    .withIndex("by_user_rfc", (q) => q.eq("userId", userId).eq("rfcMessageId", rfcMessageId))
    .first();
  if (row && row.messageId !== messageId) await ctx.db.patch(row._id, { messageId });
}

export async function upsertThreadsImpl(
  ctx: MutationCtx,
  account: Doc<"mailAccounts">,
  threads: ThreadUpsert[],
  deletes: string[] = [],
  deletedMessages: string[] = [],
): Promise<UpsertResult> {
  if (threads.length > 100) throw appError("invalid_argument", "At most 100 threads per call.");
  if (deletedMessages.length > 400)
    throw appError("invalid_argument", "At most 400 deleted messages per call.");
  threads = await withDeletedMessages(ctx, account, threads, deletedMessages);
  const userId = account.userId;
  const now = Date.now();
  const result: UpsertResult = { written: 0, skippedStale: 0, newInbound: [] };
  const pushThreads: Id<"threads">[] = [];
  const aiSettings = await ctx.db
    .query("aiSettings")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
  const triageOn = triageRouteKind(aiSettings) !== null;
  const rules = (
    await ctx.db
      .query("rules")
      .withIndex("by_user_order", (q) => q.eq("userId", userId))
      .take(100)
  ).filter((r) => r.enabled && (r.accountId === undefined || r.accountId === account._id));

  for (const tu of threads) {
    let thread = await ctx.db
      .query("threads")
      .withIndex("by_account_provider", (q) =>
        q.eq("accountId", account._id).eq("providerThreadId", tu.providerThreadId),
      )
      .unique();
    if (
      thread?.providerVersion !== undefined &&
      tu.providerVersion !== undefined &&
      tu.providerVersion < thread.providerVersion
    ) {
      result.skippedStale++;
      continue;
    }
    const rev = await bumpRev(ctx, account._id);
    const previousSynced = thread?.syncedLabelKeys ?? [];
    if (!thread) {
      const id = await ctx.db.insert("threads", {
        userId,
        accountId: account._id,
        providerThreadId: tu.providerThreadId,
        subject: "",
        snippet: "",
        participants: [],
        isBulk: false,
        lastMessageAt: 0,
        sortAt: 0,
        messageCount: 0,
        unreadCount: 0,
        draftCount: 0,
        hasAttachments: false,
        syncedLabelKeys: [],
        labelKeys: [],
        pendingOpCount: 0,
        deletedPending: false,
        triageState: triageOn ? "none" : "off",
        rev,
        updatedAt: now,
      });
      thread = (await ctx.db.get(id))!;
    }

    // 1. messages
    const newInboundMessages: Doc<"messages">[] = [];
    // Of those, the ones that arrived after the mailbox was connected. Initial sync, backfill, resync re-fetches
    // and search hits also insert messages Convex has not seen, but that is old mail: rules and push
    // notifications are for mail that arrives from now on (§4.4 steps 5 and 8, D-402).
    const arrivedMessages: Doc<"messages">[] = [];
    const listed = new Set<string>();
    let listUnsubscribe = thread.listUnsubscribe;
    for (const m of tu.messages) {
      listed.add(m.providerMessageId);
      const fields = {
        userId,
        accountId: account._id,
        threadId: thread._id,
        providerMessageId: m.providerMessageId,
        rfcMessageId: m.rfcMessageId?.replace(/^<|>$/g, ""),
        inReplyTo: m.inReplyTo?.replace(/^<|>$/g, ""),
        references: m.references.slice(-CAPS.references),
        from: m.from ? normAddr(m.from) : undefined,
        to: normAddrs(m.to),
        cc: normAddrs(m.cc),
        bcc: normAddrs(m.bcc),
        replyTo: normAddrs(m.replyTo),
        subject: truncate(m.subject, CAPS.subject),
        snippet: truncate(m.snippet, CAPS.snippet),
        date: m.date,
        labelKeys: cleanKeys(m.labelKeys),
        isUnread: m.isUnread,
        isDraft: m.isDraft,
        isOutbound: m.isOutbound,
        hasAttachments: m.hasAttachments,
        sizeBytes: m.sizeBytes,
        listId: m.listId,
        authSummary: m.authSummary,
        imapLocations: m.imapLocations,
        rev,
      };
      if (m.listUnsubscribe) listUnsubscribe = m.listUnsubscribe;
      const existing = await ctx.db
        .query("messages")
        .withIndex("by_account_provider", (q) =>
          q.eq("accountId", account._id).eq("providerMessageId", m.providerMessageId),
        )
        .unique();
      if (existing) {
        await ctx.db.replace(existing._id, fields);
      } else {
        const id = await ctx.db.insert("messages", fields);
        if (m.isOutbound && !m.isDraft && fields.rfcMessageId)
          await linkReadReceipt(ctx, userId, fields.rfcMessageId, id);
        if (!m.isOutbound && !m.isDraft) {
          const doc = (await ctx.db.get(id))!;
          newInboundMessages.push(doc);
          if (arrivedSinceConnect(account, m.date)) arrivedMessages.push(doc);
        }
        // 6. contacts: recipients of outbound mail, senders of non-bulk inbound mail.
        if (m.isOutbound && !m.isDraft) {
          for (const a of [...fields.to, ...fields.cc]) await bumpContact(ctx, userId, a, m.date, "sent");
        } else if (!m.isDraft && !m.listId && !m.listUnsubscribe && fields.from) {
          await bumpContact(ctx, userId, fields.from, m.date, "received");
        }
      }
    }
    // A conversation that synced while triage was off gets triaged once new inbound mail arrives (§12.5, D-413).
    if (thread.triageState === "off" && triageOn && newInboundMessages.length > 0) {
      await ctx.db.patch(thread._id, { triageState: "none" });
      thread = { ...thread, triageState: "none" };
    }
    const removed = new Set(tu.removedProviderMessageIds ?? []);
    let all = await ctx.db
      .query("messages")
      .withIndex("by_thread_date", (q) => q.eq("threadId", thread._id))
      .collect();
    for (const m of all) {
      if (removed.has(m.providerMessageId) || (tu.complete && !listed.has(m.providerMessageId))) {
        await ctx.db.delete(m._id);
        await tombstone(ctx, account._id, "messages", m._id);
      }
    }
    all = all.filter(
      (m) => !(removed.has(m.providerMessageId) || (tu.complete && !listed.has(m.providerMessageId))),
    );
    if (all.length === 0) {
      await deleteThreadCompletely(ctx, thread);
      continue;
    }

    // 3. aggregates and provider-confirmed keys
    const agg = threadAggregates(all.slice(-200));
    const syncedLabelKeys = threadKeysFromMessages(all);
    const isBulk = agg.isBulk || listUnsubscribe !== undefined;
    await ctx.db.patch(thread._id, {
      subject: agg.subject,
      snippet: agg.snippet,
      participants: agg.participants,
      lastFrom: agg.lastFrom,
      isBulk,
      listId: agg.listId,
      listUnsubscribe,
      lastMessageAt: agg.lastMessageAt,
      lastInboundAt: agg.lastInboundAt,
      lastOutboundAt: agg.lastOutboundAt,
      sortAt: Math.max(agg.lastMessageAt, thread.returnedAt ?? 0),
      messageCount: agg.messageCount,
      unreadCount: agg.unreadCount,
      draftCount: agg.draftCount,
      hasAttachments: agg.hasAttachments,
      syncedLabelKeys,
      providerVersion: tu.providerVersion ?? thread.providerVersion,
      rev,
      updatedAt: now,
    });

    // search text lives apart from the thread (D-020)
    const searchFields = {
      userId,
      accountId: account._id,
      threadId: thread._id,
      text: threadSearchText(
        agg.subject,
        agg.participants,
        all.filter((m) => !m.isDraft).map((m) => m.snippet),
      ),
      inInbox: syncedLabelKeys.includes(SYS.inbox),
      isUnread: syncedLabelKeys.includes(SYS.unread),
      isStarred: syncedLabelKeys.includes(SYS.starred),
      hasAttachments: agg.hasAttachments,
      isOutbound: all.some((m) => m.isOutbound && !m.isDraft),
      lastMessageAt: agg.lastMessageAt,
    };
    const search = await ctx.db
      .query("threadSearch")
      .withIndex("by_thread", (q) => q.eq("threadId", thread._id))
      .unique();
    if (search) await ctx.db.replace(search._id, searchFields);
    else await ctx.db.insert("threadSearch", searchFields);

    // 4. snooze early wake and reminder skip
    const firstInboundAt = newInboundMessages.reduce((m, x) => Math.min(m, x.date), Number.POSITIVE_INFINITY);
    const snooze = await activeTimer(ctx, thread._id, "snooze");
    if (snooze) {
      const replied = firstInboundAt > snooze.anchorAt && Number.isFinite(firstInboundAt);
      const reAdded =
        syncedLabelKeys.includes(SYS.inbox) &&
        !previousSynced.includes(SYS.inbox) &&
        previousSynced.length > 0;
      if (replied || reAdded) await fireTimer(ctx, snooze._id, { early: true });
    }
    const reminder = await activeTimer(ctx, thread._id, "reminder");
    if (
      reminder?.condition === "if_no_reply" &&
      Number.isFinite(firstInboundAt) &&
      firstInboundAt > reminder.anchorAt
    ) {
      if (reminder.scheduledFnId) await ctx.scheduler.cancel(reminder.scheduledFnId);
      await ctx.db.patch(reminder._id, { state: "skipped" });
      await ctx.db.patch(thread._id, { reminderAt: undefined });
    }

    // 5. filter rules on newly arrived inbound mail (D-046, D-402)
    if (arrivedMessages.length > 0 && rules.length > 0) {
      const fresh = (await ctx.db.get(thread._id))!;
      const facts = {
        fromEmail: fresh.lastFrom?.email,
        participants: fresh.participants.map((p) => p.email),
        labelKeys: syncedLabelKeys,
        triage: fresh.triage
          ? { category: fresh.triage.category, needsReply: fresh.triage.needsReply }
          : undefined,
        isBulk: fresh.isBulk,
        listId: fresh.listId,
        subject: fresh.subject,
        accountId: account._id,
        vip: await isVipSender(ctx, userId, fresh.lastFrom?.email),
      };
      const newest = arrivedMessages[arrivedMessages.length - 1]!;
      for (const rule of rules) {
        if (!matchRule(rule.rule, facts)) continue;
        const clientOpId = `rule:${rule._id}:${newest._id}`;
        if ((await opsForClientOp(ctx, userId, clientOpId)).length > 0) continue;
        for (const action of rule.actions) {
          const current = (await ctx.db.get(thread._id))!;
          try {
            await enqueueThreadAction(ctx, {
              userId,
              account,
              thread: current,
              clientOpId,
              action: action as ThreadActionLike,
            });
          } catch {
            // A rule that no longer fits the account (deleted label, wrong caps) is skipped, not fatal.
          }
        }
      }
    }

    // 7. visible state
    const visible = await recomputeThread(ctx, thread._id);
    result.written++;
    if (newInboundMessages.length > 0) {
      const isInbox = visible?.labelKeys.includes(SYS.inbox) ?? false;
      result.newInbound.push({ threadId: thread._id, isInbox });
      if (isInbox && arrivedMessages.length > 0) pushThreads.push(thread._id);
    }
  }

  for (const providerThreadId of deletes) {
    const t = await ctx.db
      .query("threads")
      .withIndex("by_account_provider", (q) =>
        q.eq("accountId", account._id).eq("providerThreadId", providerThreadId),
      )
      .unique();
    if (t) await deleteThreadCompletely(ctx, t);
  }

  // 8. push notifications for inbox threads with newly arrived mail, only for users with push tokens
  const inboxNew = pushThreads;
  if (inboxNew.length > 0 && account.status === "live") {
    const device = await ctx.db
      .query("devices")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .filter((q) => q.neq(q.field("expoPushToken"), undefined))
      .first();
    if (device)
      await ctx.scheduler.runAfter(0, internal.notify.newMail, { userId, threadIds: inboxNew.slice(0, 20) });
  }
  return result;
}
