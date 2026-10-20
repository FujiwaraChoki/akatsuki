// Label keys (ARCHITECTURE §1.4): the one vocabulary for thread state and lists.
//
//   labelKey = sysKey | userKey | splitKey
//   sysKey   = "sys:" ( inbox | sent | drafts | trash | spam | starred | important | unread | snoozed )
//   userKey  = "l:" <Id<"labels">>
//   splitKey = "split:" <Id<"splits">> | "split:other"      (virtual; never sent in actions)
//
// This module and crates/core::model::label_key are the only parsers; both run fixtures/label-keys.json.

export const SYS_ROLES = [
  "inbox",
  "sent",
  "drafts",
  "trash",
  "spam",
  "starred",
  "important",
  "unread",
  "snoozed",
] as const;
export type SysRole = (typeof SYS_ROLES)[number];

/** The nine system keys by role. */
export const SYS = {
  inbox: "sys:inbox",
  sent: "sys:sent",
  drafts: "sys:drafts",
  trash: "sys:trash",
  spam: "sys:spam",
  starred: "sys:starred",
  important: "sys:important",
  unread: "sys:unread",
  snoozed: "sys:snoozed",
} as const satisfies Record<SysRole, string>;
export type SysKey = (typeof SYS)[SysRole];

/** The split key of inbox threads that match no split. */
export const SPLIT_OTHER = "split:other";

/** Convex ids as they appear inside `l:` and `split:` keys. */
const ID_RE = /^[a-z0-9]{8,64}$/;

export type ParsedLabelKey =
  | { kind: "sys"; role: SysRole }
  | { kind: "user"; labelId: string }
  | { kind: "split"; splitId: string | "other" };

/** Parses a label key, or returns `null` when it is not one. */
export function parseLabelKey(key: string): ParsedLabelKey | null {
  if (key.startsWith("sys:")) {
    const role = key.slice(4);
    return (SYS_ROLES as readonly string[]).includes(role) ? { kind: "sys", role: role as SysRole } : null;
  }
  if (key.startsWith("l:")) {
    const id = key.slice(2);
    return ID_RE.test(id) ? { kind: "user", labelId: id } : null;
  }
  if (key.startsWith("split:")) {
    const id = key.slice(6);
    if (id === "other") return { kind: "split", splitId: "other" };
    return ID_RE.test(id) ? { kind: "split", splitId: id } : null;
  }
  return null;
}

export function isLabelKey(key: string): boolean {
  return parseLabelKey(key) !== null;
}

/** `true` for `split:*` (whether or not the id is well-formed). Split keys never appear in actions. */
export const isSplitKey = (key: string): boolean => key.startsWith("split:");
/** `true` for the nine `sys:*` keys. */
export const isSysKey = (key: string): key is SysKey => parseLabelKey(key)?.kind === "sys";
/** `l:<labelId>`. */
export const userKey = (labelId: string): string => `l:${labelId}`;
/** `split:<splitId>` (or `split:other`). */
export const splitKey = (splitId: string): string => `split:${splitId}`;
/** `sys:<role>`. */
export const sysKey = (role: SysRole): SysKey => SYS[role];

/**
 * The key a label row is addressed by (§3.2): `sys:<role>` when its role is one of the nine system roles,
 * otherwise `l:<_id>` (user labels, Graph categories, IMAP keywords, non-system folders, Gmail CATEGORY_*).
 */
export function labelKeyOf(label: { _id: string; role?: string }): string {
  return label.role !== undefined && (SYS_ROLES as readonly string[]).includes(label.role)
    ? `sys:${label.role}`
    : `l:${label._id}`;
}
