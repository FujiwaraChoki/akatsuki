// Optimistic updates for thread actions (ARCHITECTURE §16.1). Web and mobile pass this to
// `useMutation(api.mail.apply).withOptimisticUpdate(...)` (and `mail.snooze`): it applies `applyOps` to the
// cached `mail.listThreads` pages, `mail.getThread` and `mail.unreadCounts`, so a thread leaves the inbox the
// moment the key is pressed. Convex drops the optimistic layer when the server result lands, and the server
// result already reflects success or failure, so there is no rollback code.
//
// Imported as `@akatsuki/mail-model/optimistic` (it needs the `convex` client package; the rest of
// @akatsuki/mail-model does not).
import type { OptimisticLocalStore } from "convex/browser";
import { type FunctionReference, makeFunctionReference } from "convex/server";
import { type ApplyContext, applyOps, type Visible } from "./applyOps.ts";
import { SYS } from "./labelKeys.ts";
import type { ThreadAction } from "./types.ts";

/** `mail.listThreads` `view` argument. `labelKey` is a label key or `"all"`. */
export interface ListView {
  labelKey: string;
  accountId?: string;
  unreadOnly?: boolean;
}

/** The thread row fields the optimistic layer reads and patches (a `vThreadRow` fits). */
export interface OptimisticThreadRow {
  _id: string;
  accountId: string;
  labelKeys: string[];
  split?: string;
  unreadCount: number;
  pendingOpCount: number;
}

interface Page<T> {
  page: T[];
  isDone: boolean;
  continueCursor: string;
}

type ListArgs = { view: ListView; paginationOpts: { numItems: number; cursor: string | null } };
type CountArgs = { labelKeys: string[]; accountId?: string };

/** The cached queries this module touches. Override them in tests or if the function names change. */
export interface MailQueries {
  listThreads: FunctionReference<"query", "public", ListArgs, Page<OptimisticThreadRow>>;
  unreadCounts: FunctionReference<"query", "public", CountArgs, Record<string, number>>;
  getThread: FunctionReference<"query", "public", { threadId: string }, { thread: OptimisticThreadRow }>;
}

export const MAIL_QUERIES: MailQueries = {
  listThreads: makeFunctionReference<"query", ListArgs, Page<OptimisticThreadRow>>("mail:listThreads"),
  unreadCounts: makeFunctionReference<"query", CountArgs, Record<string, number>>("mail:unreadCounts"),
  getThread: makeFunctionReference<"query", { threadId: string }, { thread: OptimisticThreadRow }>(
    "mail:getThread",
  ),
};

/** `mail.unreadCounts` is capped at this value server-side (the UI shows "99+"). */
export const UNREAD_COUNT_CAP = 100;

export interface OptimisticOptions {
  /** The apply context of an account (`applyContextFor(provider, exclusiveLabelIds)`). */
  contextFor: (accountId: string) => ApplyContext;
  queries?: Partial<MailQueries>;
}

/** An action, or a function of the row for actions that differ per account (e.g. snooze). */
export type ActionOrBuilder = ThreadAction | ((row: OptimisticThreadRow) => ThreadAction);

/** Whether a thread with visible state `v` belongs in `view` (mirrors `mail.listThreads`). */
export function viewContains(
  view: ListView,
  v: Visible,
  row: Pick<OptimisticThreadRow, "accountId" | "split">,
): boolean {
  if (v.deleted) return false;
  if (view.accountId !== undefined && view.accountId !== row.accountId) return false;
  if (view.unreadOnly && !v.keys.includes(SYS.unread)) return false;
  if (view.labelKey === "all") return true;
  if (view.labelKey.startsWith("split:")) return v.keys.includes(SYS.inbox) && row.split === view.labelKey;
  return v.keys.includes(view.labelKey);
}

/** Whether the thread counts as unread under `key` (a label key or split key). */
function unreadUnder(key: string, v: Visible, split: string | undefined): boolean {
  if (v.deleted || !v.keys.includes(SYS.unread)) return false;
  if (key.startsWith("split:")) return v.keys.includes(SYS.inbox) && split === key;
  return v.keys.includes(key);
}

function patchRow<T extends OptimisticThreadRow>(row: T, after: Visible): T {
  const unread = after.keys.includes(SYS.unread);
  return {
    ...row,
    labelKeys: after.keys,
    split: after.keys.includes(SYS.inbox) ? row.split : undefined,
    unreadCount: unread ? Math.max(1, row.unreadCount) : 0,
    pendingOpCount: row.pendingOpCount + 1,
  };
}

/**
 * Applies `action` to every cached copy of `threadIds`:
 * - `mail.listThreads` pages: rows are patched, or removed when their view no longer contains the thread
 *   (rows are never inserted: a thread entering a view appears when the server result lands);
 * - `mail.getThread`: the thread row is patched;
 * - `mail.unreadCounts`: each count moves by the threads that entered or left "unread under that key"
 *   (counts at the server cap are left alone, since the true number is unknown).
 * Threads that are in no cached query are skipped; the server result covers them.
 */
export function optimisticApply(
  localStore: OptimisticLocalStore,
  threadIds: readonly string[],
  action: ActionOrBuilder,
  opts: OptimisticOptions,
): void {
  const q = { ...MAIL_QUERIES, ...opts.queries };
  const ids = new Set(threadIds);
  const transitions = new Map<
    string,
    { row: OptimisticThreadRow; before: Visible; after: Visible; afterSplit: string | undefined }
  >();
  const transition = (row: OptimisticThreadRow) => {
    let t = transitions.get(row._id);
    if (!t) {
      const a = typeof action === "function" ? action(row) : action;
      const before: Visible = { keys: row.labelKeys, deleted: false };
      const after = applyOps(row.labelKeys, [a], opts.contextFor(row.accountId));
      t = { row, before, after, afterSplit: after.keys.includes(SYS.inbox) ? row.split : undefined };
      transitions.set(row._id, t);
    }
    return t;
  };

  for (const { args, value } of localStore.getAllQueries(q.listThreads)) {
    if (!value?.page.some((r) => ids.has(r._id))) continue;
    const page: OptimisticThreadRow[] = [];
    for (const row of value.page) {
      if (!ids.has(row._id)) {
        page.push(row);
        continue;
      }
      const t = transition(row);
      if (viewContains(args.view, t.after, { accountId: row.accountId, split: t.afterSplit }))
        page.push(patchRow(row, t.after));
    }
    localStore.setQuery(q.listThreads, args, { ...value, page });
  }

  for (const { args, value } of localStore.getAllQueries(q.getThread)) {
    if (!value || !ids.has(value.thread._id)) continue;
    const t = transition(value.thread);
    localStore.setQuery(q.getThread, args, { ...value, thread: patchRow(value.thread, t.after) });
  }

  if (transitions.size === 0) return;
  for (const { args, value } of localStore.getAllQueries(q.unreadCounts)) {
    if (!value) continue;
    let changed = false;
    const next: Record<string, number> = { ...value };
    for (const [key, count] of Object.entries(value)) {
      if (count >= UNREAD_COUNT_CAP) continue;
      let delta = 0;
      for (const t of transitions.values()) {
        if (args.accountId !== undefined && args.accountId !== t.row.accountId) continue;
        delta +=
          Number(unreadUnder(key, t.after, t.afterSplit)) - Number(unreadUnder(key, t.before, t.row.split));
      }
      if (delta !== 0) {
        next[key] = Math.min(UNREAD_COUNT_CAP, Math.max(0, count + delta));
        changed = true;
      }
    }
    if (changed) localStore.setQuery(q.unreadCounts, args, next);
  }
}

/**
 * The optimistic update for `mail.apply`:
 * `useMutation(api.mail.apply).withOptimisticUpdate(applyOptimisticUpdate({ contextFor }))`.
 */
export function applyOptimisticUpdate(opts: OptimisticOptions) {
  return (localStore: OptimisticLocalStore, args: { threadIds: string[]; action: ThreadAction }): void =>
    optimisticApply(localStore, args.threadIds, args.action, opts);
}

/**
 * The optimistic update for `mail.snooze`. `snoozeActionFor` returns the provider-side snooze action of a
 * row's account (`actionsFor(account).snooze()`).
 */
export function snoozeOptimisticUpdate(
  opts: OptimisticOptions & { snoozeActionFor: (accountId: string) => ThreadAction },
) {
  return (localStore: OptimisticLocalStore, args: { threadIds: string[] }): void =>
    optimisticApply(localStore, args.threadIds, (row) => opts.snoozeActionFor(row.accountId), opts);
}
