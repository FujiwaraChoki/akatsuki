// Multi-select in the conversation list (BRIEF "Keyboard-first", shortcuts `select.*`): long-press a row (or `x`)
// to start, tap rows to add or remove them, Shift+J/K to extend, Ctrl/⌘+A for everything loaded. Bulk actions
// then act on the selection through the same `mail.apply` path as a single row (grouped per account, ≤ 500 ids
// per call). Pure state so it is tested without a device; the list screen holds one `Selection`.
import { SYS } from "@akatsuki/mail-model";

export interface Selection {
  /** Selected thread ids. */
  readonly ids: ReadonlySet<string>;
  /** Where Shift+J/K extends from (the row the selection started on). */
  readonly anchor: string | null;
}

export const EMPTY_SELECTION: Selection = { ids: new Set(), anchor: null };

/** The list is in selection mode: taps toggle rows, swipes are off, the bulk bar shows. */
export function isSelecting(s: Selection): boolean {
  return s.ids.size > 0;
}

/** Adds or removes one row; the first row selected becomes the anchor. */
export function toggleSelected(s: Selection, id: string): Selection {
  const ids = new Set(s.ids);
  if (ids.has(id)) ids.delete(id);
  else ids.add(id);
  if (ids.size === 0) return EMPTY_SELECTION;
  if (s.anchor && ids.has(s.anchor)) return { ids, anchor: s.anchor };
  return { ids, anchor: ids.has(id) ? id : ([...ids][0] ?? null) };
}

/** Selects one row (long-press on an unselected row keeps the others). */
export function addSelected(s: Selection, id: string): Selection {
  if (s.ids.has(id)) return s;
  return toggleSelected(s, id);
}

/**
 * Shift+J / Shift+K: selects the cursor row, moves the cursor by `delta` and selects every row between the anchor
 * and the new cursor (Gmail and Superhuman behaviour). Returns the new selection and cursor.
 */
export function extendSelection(
  s: Selection,
  order: readonly string[],
  cursor: number,
  delta: 1 | -1,
): { selection: Selection; cursor: number } {
  if (order.length === 0) return { selection: s, cursor: -1 };
  const from = cursor < 0 ? 0 : Math.min(cursor, order.length - 1);
  const next = cursor < 0 ? 0 : Math.max(0, Math.min(order.length - 1, from + delta));
  const anchorId = s.anchor && order.includes(s.anchor) ? s.anchor : order[from]!;
  const a = order.indexOf(anchorId);
  const [lo, hi] = a <= next ? [a, next] : [next, a];
  const ids = new Set(s.ids);
  for (let i = lo; i <= hi; i++) ids.add(order[i]!);
  return { selection: { ids, anchor: anchorId }, cursor: next };
}

/** Ctrl/⌘+A: every loaded row; pressed again with everything selected, it clears. */
export function selectAll(s: Selection, order: readonly string[]): Selection {
  if (order.length === 0) return EMPTY_SELECTION;
  if (order.every((id) => s.ids.has(id))) return EMPTY_SELECTION;
  return { ids: new Set(order), anchor: s.anchor ?? order[0]! };
}

/** Drops rows that left the view (an action moved them, or the view changed). Same object when nothing left. */
export function pruneSelection(s: Selection, visible: readonly string[]): Selection {
  if (s.ids.size === 0) return s;
  const keep = new Set(visible);
  let changed = false;
  const ids = new Set<string>();
  for (const id of s.ids) {
    if (keep.has(id)) ids.add(id);
    else changed = true;
  }
  if (!changed) return s;
  if (ids.size === 0) return EMPTY_SELECTION;
  return { ids, anchor: s.anchor && ids.has(s.anchor) ? s.anchor : ([...ids][0] ?? null) };
}

/** "3 selected" (with "all" when every loaded row is in it). */
export function selectionTitle(s: Selection, loaded: number): string {
  const n = s.ids.size;
  if (n > 0 && n === loaded && loaded > 1) return `All ${n} selected`;
  return `${n} selected`;
}

/** The bulk actions the selection offers. Ids match `ACTIONS` in state/actions.ts plus the pickers. */
export type BulkActionId =
  | "archive"
  | "moveToInbox"
  | "untrash"
  | "notSpam"
  | "trash"
  | "spam"
  | "markRead"
  | "markUnread"
  | "star"
  | "unstar"
  | "snooze"
  | "label"
  | "move";

export interface BulkAction {
  id: BulkActionId;
  label: string;
  /** The word under the icon in the bottom bar. */
  short: string;
  /** The shared shortcut id that runs it (for hints and the key map). */
  shortcut: string;
  destructive?: boolean;
}

export interface SelectedRowLike {
  labelKeys: readonly string[];
  /** The thread's account can move to folders (`actionsFor(...).available("moveTo")`). */
  canMove?: boolean;
}

/**
 * What a selection can do, in display order: the first four are the bar's buttons, the rest go in "More".
 * Mixed selections offer the action that changes something for at least one row (archive if any is in the
 * inbox; mark read if any is unread; star if any is unstarred), like web.
 */
export function bulkActions(rows: readonly SelectedRowLike[]): BulkAction[] {
  if (rows.length === 0) return [];
  const any = (k: string) => rows.some((r) => r.labelKeys.includes(k));
  const all = (k: string) => rows.every((r) => r.labelKeys.includes(k));
  const out: BulkAction[] = [];
  if (all(SYS.trash))
    out.push({ id: "untrash", short: "Restore", label: "Restore", shortcut: "thread.trash" });
  else if (all(SYS.spam))
    out.push({ id: "notSpam", short: "Not spam", label: "Not spam", shortcut: "thread.spam" });
  else if (any(SYS.inbox))
    out.push({ id: "archive", short: "Archive", label: "Archive", shortcut: "thread.archive" });
  else
    out.push({ id: "moveToInbox", short: "Inbox", label: "Move to Inbox", shortcut: "thread.moveToInbox" });
  const trashOrSpam = any(SYS.trash) || any(SYS.spam);
  if (!trashOrSpam) out.push({ id: "snooze", short: "Snooze", label: "Snooze", shortcut: "thread.snooze" });
  out.push(
    any(SYS.unread)
      ? { id: "markRead", short: "Read", label: "Mark read", shortcut: "thread.toggleRead" }
      : { id: "markUnread", short: "Unread", label: "Mark unread", shortcut: "thread.toggleRead" },
  );
  if (!all(SYS.trash))
    out.push({
      id: "trash",
      short: "Trash",
      label: "Move to Trash",
      shortcut: "thread.trash",
      destructive: true,
    });
  out.push(
    rows.every((r) => r.labelKeys.includes(SYS.starred))
      ? { id: "unstar", short: "Unstar", label: "Unstar", shortcut: "thread.star" }
      : { id: "star", short: "Star", label: "Star", shortcut: "thread.star" },
  );
  out.push({ id: "label", short: "Label", label: "Label", shortcut: "thread.label" });
  if (rows.some((r) => r.canMove))
    out.push({ id: "move", short: "Move", label: "Move to folder", shortcut: "thread.move" });
  if (!any(SYS.inbox) && !all(SYS.trash) && !all(SYS.spam) && out[0]!.id !== "moveToInbox")
    out.push({ id: "moveToInbox", short: "Inbox", label: "Move to Inbox", shortcut: "thread.moveToInbox" });
  if (!any(SYS.spam) && !all(SYS.trash))
    out.push({ id: "spam", short: "Spam", label: "Report spam", shortcut: "thread.spam", destructive: true });
  return out;
}

/** The bulk action a list-scope shortcut means for the current selection, or null when it has none. */
export function bulkActionForShortcut(actions: readonly BulkAction[], shortcut: string): BulkAction | null {
  return actions.find((a) => a.shortcut === shortcut) ?? null;
}
