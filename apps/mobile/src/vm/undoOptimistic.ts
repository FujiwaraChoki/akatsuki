// Instant undo (the web's D-265, here D-336): `optimisticApply` patches cached rows but never inserts them (§16.1),
// so undoing an archive would show the thread again only when the server answered. When a gesture is sent, the
// rows are remembered as they were; the `mail.undo` optimistic update applies the per-thread inverse
// (`inverseAction`, D-250) and puts removed rows back into every cached list page whose view contains them.
import { type ApplyContext, effectOf, inverseAction, type ThreadAction } from "@akatsuki/mail-model";
import {
  MAIL_QUERIES,
  type OptimisticThreadRow,
  optimisticApply,
  viewContains,
} from "@akatsuki/mail-model/optimistic";
import type { OptimisticLocalStore } from "convex/browser";

type Row = OptimisticThreadRow & { sortAt?: number };

interface Remembered {
  action: ThreadAction | ((row: Row) => ThreadAction);
  rows: Row[];
  at: number;
}

const MAX_AGE_MS = 60_000;
const remembered = new Map<string, Remembered>();

function prune(now: number) {
  for (const [k, v] of remembered) if (now - v.at > MAX_AGE_MS) remembered.delete(k);
}

/** Captures the cached rows of `threadIds` before an action changes them (call inside the optimistic update). */
export function rememberForUndo(
  localStore: OptimisticLocalStore,
  clientOpId: string,
  threadIds: readonly string[],
  action: Remembered["action"],
  now = Date.now(),
): void {
  prune(now);
  const ids = new Set(threadIds);
  const rows = new Map<string, Row>();
  for (const { value } of localStore.getAllQueries(MAIL_QUERIES.listThreads)) {
    for (const r of value?.page ?? [])
      if (ids.has(r._id) && !rows.has(r._id)) rows.set(r._id, { ...(r as Row) });
  }
  for (const { value } of localStore.getAllQueries(MAIL_QUERIES.getThread)) {
    const r = value?.thread as Row | undefined;
    if (r && ids.has(r._id) && !rows.has(r._id)) rows.set(r._id, { ...r });
  }
  remembered.set(clientOpId, { action, rows: [...rows.values()], at: now });
}

export function forgetUndo(clientOpId: string): void {
  remembered.delete(clientOpId);
}

/** The optimistic update for `mail.undo({clientOpId, undoClientOpId})`. */
export function undoOptimisticUpdate(opts: {
  contextFor: (accountId: string) => ApplyContext;
  isFolderAccount: (accountId: string) => boolean;
}) {
  return (localStore: OptimisticLocalStore, args: { undoClientOpId: string }): void => {
    const entry = remembered.get(args.undoClientOpId);
    if (!entry) return;
    for (const row of entry.rows) {
      const action = typeof entry.action === "function" ? entry.action(row) : entry.action;
      const ctx = opts.contextFor(row.accountId);
      const effect = effectOf({ keys: row.labelKeys, deleted: false }, action, ctx);
      const inverse = inverseAction(action, effect, opts.isFolderAccount(row.accountId));
      if (!inverse) continue;
      // Rows still cached (e.g. a star toggle, or a view that still contains the thread) get the inverse.
      optimisticApply(localStore, [row._id], inverse, { contextFor: opts.contextFor });
      // Rows the action removed come back where their view contains them again.
      for (const { args: qArgs, value } of localStore.getAllQueries(MAIL_QUERIES.listThreads)) {
        if (!value || value.page.some((r) => r._id === row._id)) continue;
        if (!viewContains(qArgs.view, { keys: row.labelKeys, deleted: false }, row)) continue;
        const page = [...value.page] as Row[];
        const sortAt = row.sortAt ?? 0;
        const last = page[page.length - 1];
        if (!value.isDone && last && (last.sortAt ?? 0) > sortAt) continue; // belongs to a later page
        let i = page.findIndex((r) => (r.sortAt ?? 0) < sortAt);
        if (i < 0) i = page.length;
        page.splice(i, 0, { ...row, pendingOpCount: row.pendingOpCount + 1 });
        localStore.setQuery(MAIL_QUERIES.listThreads, qArgs, { ...value, page });
      }
    }
    remembered.delete(args.undoClientOpId);
  };
}
