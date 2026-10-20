// Instant undo (z) for the Convex backend. `optimisticApply` from @akatsuki/mail-model patches rows but never
// inserts them, so an undone archive would only reappear when the server answers. The controller records
// each gesture's rows as they were before it ran; the `mail.undo` optimistic update then re-applies the
// inverse to rows that are still cached and puts removed rows back into the list pages they fit (D-265).
import { type ApplyContext, inverseAction, isFolderProvider, type ThreadAction } from "@akatsuki/mail-model";
import { type ListView, MAIL_QUERIES, optimisticApply, viewContains } from "@akatsuki/mail-model/optimistic";
import type { OptimisticLocalStore } from "convex/browser";
import type { ThreadRow } from "../backend/types.ts";

export interface UndoSnapshot {
  /** Rows before the gesture. */
  before: ThreadRow[];
  /** Per-thread inverse action (`null`: nothing to undo for that thread). */
  inverse: Map<string, ThreadAction | null>;
  at: number;
}

const snapshots = new Map<string, UndoSnapshot>();
const MAX = 50;

export function rememberUndo(clientOpId: string, snap: UndoSnapshot): void {
  snapshots.set(clientOpId, snap);
  while (snapshots.size > MAX) snapshots.delete(snapshots.keys().next().value as string);
}

export function undoSnapshot(clientOpId: string): UndoSnapshot | undefined {
  return snapshots.get(clientOpId);
}

/** Builds the snapshot for `action` over `rows`, using the same inverse rule as Convex (`inverseAction`, D-250). */
export function snapshotFor(
  rows: ThreadRow[],
  action: ThreadAction,
  providerOf: (accountId: string) => "gmail" | "graph" | "imap",
  contextFor: (accountId: string) => ApplyContext,
  applyFn: (keys: string[], a: ThreadAction, ctx: ApplyContext) => string[],
  diff: (before: string[], after: string[]) => { added: string[]; removed: string[] },
): UndoSnapshot {
  const inverse = new Map<string, ThreadAction | null>();
  for (const r of rows) {
    const after = applyFn(r.labelKeys, action, contextFor(r.accountId));
    inverse.set(
      r._id,
      inverseAction(action, diff(r.labelKeys, after), isFolderProvider(providerOf(r.accountId))),
    );
  }
  return { before: rows, inverse, at: Date.now() };
}

const NOOP: ThreadAction = { kind: "modify", add: [], remove: [] };

/** The `mail.undo` optimistic update. */
export function undoOptimisticUpdate(contextFor: (accountId: string) => ApplyContext) {
  return (store: OptimisticLocalStore, args: { undoClientOpId: string }) => {
    const snap = snapshots.get(args.undoClientOpId);
    if (!snap) return;
    const ids = snap.before.map((r) => r._id);
    optimisticApply(store, ids, (row) => snap.inverse.get(row._id) ?? NOOP, { contextFor });
    reinsert(store, snap.before);
  };
}

type Page = { page: ThreadRow[]; isDone: boolean; continueCursor: string };
type ListArgs = { view: ListView; paginationOpts: { numItems: number; cursor: string | null } };

/** Puts rows back into cached list pages whose view contains them and whose range covers their `sortAt`. */
export function reinsert(store: OptimisticLocalStore, rows: ThreadRow[]): void {
  for (const { args, value } of store.getAllQueries(MAIL_QUERIES.listThreads)) {
    if (!value) continue;
    const a = args as unknown as ListArgs;
    const v = value as unknown as Page;
    let page = v.page;
    let changed = false;
    for (const row of rows) {
      if (page.some((r) => r._id === row._id)) continue;
      if (!viewContains(a.view, { keys: row.labelKeys, deleted: false }, row)) continue;
      const top = page[0];
      const last = page[page.length - 1];
      const belowTop = a.paginationOpts.cursor === null || !top || row.sortAt <= top.sortAt;
      const aboveBottom = v.isDone || !last || row.sortAt >= last.sortAt;
      if (!belowTop || !aboveBottom) continue;
      const restored = { ...row, pendingOpCount: row.pendingOpCount + 1 };
      const i = page.findIndex((r) => r.sortAt < row.sortAt);
      page = i === -1 ? [...page, restored] : [...page.slice(0, i), restored, ...page.slice(i)];
      changed = true;
    }
    if (changed) store.setQuery(MAIL_QUERIES.listThreads, args, { ...value, page } as typeof value);
  }
}
