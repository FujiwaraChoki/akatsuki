// Thread list view-model (ARCHITECTURE §1.4 label keys, §3.4 splits, §4.2 `mail.listThreads`, §16.2).
import { SPLIT_OTHER, SYS } from "@akatsuki/mail-model";
import { countLabel, listDate, participantsLabel } from "./format.ts";

/** The mailbox views reachable from navigation and the palette (`go.*` shortcuts). */
export type MailboxId = "inbox" | "starred" | "sent" | "drafts" | "snoozed" | "all" | "spam" | "trash";

export interface Mailbox {
  id: MailboxId;
  title: string;
  /** `mail.listThreads` view label key ("all" = every thread). */
  labelKey: string;
  /** The `go.*` shortcut id that opens it. */
  shortcut: string;
  emptyTitle: string;
  emptyBody: string;
}

export const MAILBOXES: readonly Mailbox[] = [
  {
    id: "inbox",
    title: "Inbox",
    labelKey: SYS.inbox,
    shortcut: "go.inbox",
    emptyTitle: "Inbox zero",
    emptyBody: "Nothing waiting. New mail lands here.",
  },
  {
    id: "starred",
    title: "Starred",
    labelKey: SYS.starred,
    shortcut: "go.starred",
    emptyTitle: "Nothing starred",
    emptyBody: "Star a conversation to keep it close.",
  },
  {
    id: "snoozed",
    title: "Snoozed",
    labelKey: SYS.snoozed,
    shortcut: "go.snoozed",
    emptyTitle: "Nothing snoozed",
    emptyBody: "Snoozed conversations come back to the inbox on time.",
  },
  {
    id: "sent",
    title: "Sent",
    labelKey: SYS.sent,
    shortcut: "go.sent",
    emptyTitle: "Nothing sent yet",
    emptyBody: "Mail you send appears here.",
  },
  {
    id: "drafts",
    title: "Drafts",
    labelKey: SYS.drafts,
    shortcut: "go.drafts",
    emptyTitle: "No drafts",
    emptyBody: "Drafts you start on any device appear here.",
  },
  {
    id: "all",
    title: "All mail",
    labelKey: "all",
    shortcut: "go.all",
    emptyTitle: "No mail yet",
    emptyBody: "Connect a mailbox in Settings to start syncing.",
  },
  {
    id: "spam",
    title: "Spam",
    labelKey: SYS.spam,
    shortcut: "go.spam",
    emptyTitle: "No spam",
    emptyBody: "Nothing here.",
  },
  {
    id: "trash",
    title: "Trash",
    labelKey: SYS.trash,
    shortcut: "go.trash",
    emptyTitle: "Trash is empty",
    emptyBody: "Nothing here.",
  },
];

export function mailboxById(id: string | undefined): Mailbox {
  return MAILBOXES.find((m) => m.id === id) ?? MAILBOXES[0]!;
}

export function mailboxForShortcut(shortcutId: string): Mailbox | undefined {
  return MAILBOXES.find((m) => m.shortcut === shortcutId);
}

/** A `splits.list` row (the fields used here). */
export interface SplitLike {
  _id: string;
  name: string;
  order: number;
  showCount: boolean;
  labelKey?: string;
}

export interface SplitTab {
  key: string;
  title: string;
  /** "12" / "99+" / null (hidden when the split does not show counts). */
  count: string | null;
}

/**
 * The split inbox tabs, as on the web (D-263): "Inbox" (`sys:inbox`, the whole inbox) first, then every split in
 * `order`, then "Other" (`split:other`). With no splits the inbox is the one "Inbox" tab. Split tabs show a count
 * only when the split has `showCount`.
 */
export function splitTabs(
  splits: readonly SplitLike[] | undefined,
  counts: Record<string, number> | undefined,
): SplitTab[] {
  const inbox: SplitTab = { key: SYS.inbox, title: "Inbox", count: countLabel(counts?.[SYS.inbox]) };
  if (!splits || splits.length === 0) return [inbox];
  const sorted = [...splits].sort((a, b) => a.order - b.order);
  const tabs = sorted.map((s) => {
    const key = s.labelKey ?? `split:${s._id}`;
    return { key, title: s.name, count: s.showCount ? countLabel(counts?.[key]) : null };
  });
  return [inbox, ...tabs, { key: SPLIT_OTHER, title: "Other", count: countLabel(counts?.[SPLIT_OTHER]) }];
}

/** The label keys whose unread counts the inbox needs (≤ 30 per `mail.unreadCounts` call). */
export function countKeys(tabs: readonly SplitTab[]): string[] {
  return tabs.map((t) => t.key).slice(0, 30);
}

/** Moves between split tabs (`split.next` / `split.prev`, wrapping). */
export function cycleTab(tabs: readonly SplitTab[], current: string, delta: 1 | -1): string {
  if (tabs.length === 0) return current;
  const i = Math.max(
    0,
    tabs.findIndex((t) => t.key === current),
  );
  return tabs[(i + delta + tabs.length) % tabs.length]!.key;
}

/** The thread row fields the list renders (`ThreadRow`, §4.4). */
export interface ThreadRowLike {
  _id: string;
  accountId: string;
  subject: string;
  snippet: string;
  participants: { name?: string; email: string }[];
  lastFrom?: { name?: string; email: string };
  lastMessageAt: number;
  sortAt: number;
  messageCount: number;
  unreadCount: number;
  draftCount: number;
  hasAttachments: boolean;
  labelKeys: string[];
  pendingOpCount: number;
  snoozedUntil?: number;
  reminderAt?: number;
  triage?: { category: string; needsReply: boolean };
}

export interface ThreadRowModel {
  id: string;
  accountId: string;
  sender: string;
  subject: string;
  snippet: string;
  date: string;
  unread: boolean;
  starred: boolean;
  hasAttachments: boolean;
  hasDraft: boolean;
  needsReply: boolean;
  /** "Snoozed until Tue 08:00"-style hint, set in the Snoozed view. */
  badge: string | null;
  /** An action on it is still travelling to the provider. */
  pending: boolean;
  /** Screen-reader summary of the row. */
  accessibilityLabel: string;
}

export function threadRowModel(
  row: ThreadRowLike,
  opts: { now: number; timeZone: string; selfEmails: ReadonlySet<string>; describe?: (at: number) => string },
): ThreadRowModel {
  const unread = row.labelKeys.includes(SYS.unread) || row.unreadCount > 0;
  const starred = row.labelKeys.includes(SYS.starred);
  const sender = participantsLabel(row.participants, row.lastFrom, row.messageCount, opts.selfEmails);
  const subject = row.subject.trim() || "(no subject)";
  const date = listDate(row.lastMessageAt, opts.now, opts.timeZone);
  let badge: string | null = null;
  if (row.snoozedUntil && opts.describe) badge = `Snoozed until ${opts.describe(row.snoozedUntil)}`;
  else if (row.reminderAt && opts.describe) badge = `Reminder ${opts.describe(row.reminderAt)}`;
  const parts = [
    unread ? "Unread." : null,
    `From ${sender}.`,
    subject,
    row.snippet,
    date,
    starred ? "Starred." : null,
  ];
  return {
    id: row._id,
    accountId: row.accountId,
    sender,
    subject,
    snippet: row.snippet,
    date,
    unread,
    starred,
    hasAttachments: row.hasAttachments,
    hasDraft: row.draftCount > 0,
    needsReply: row.triage?.needsReply === true,
    badge,
    pending: row.pendingOpCount > 0,
    accessibilityLabel: parts.filter(Boolean).join(" "),
  };
}

/** Index of the next row for `nav.next` / `nav.prev` (clamped, -1 when the list is empty). */
export function moveSelection(length: number, index: number, delta: number): number {
  if (length === 0) return -1;
  if (index < 0) return delta > 0 ? 0 : length - 1;
  return Math.min(length - 1, Math.max(0, index + delta));
}

/**
 * After an action removes the selected row, keep the cursor on the row that took its place (Superhuman moves to
 * the next conversation), or the new last row.
 */
export function selectionAfterRemoval(ids: readonly string[], removedIndex: number): number {
  if (ids.length === 0) return -1;
  return Math.min(removedIndex, ids.length - 1);
}
