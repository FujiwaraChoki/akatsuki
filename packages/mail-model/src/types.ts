// Domain types shared by every module of @akatsuki/mail-model. They are structurally identical to the Convex
// validators in convex/lib/validators.ts (`vThreadAction`, `vRule`, `vAddr`, ...), so values flow between this
// package and Convex without casts, but this package never imports Convex code (ARCHITECTURE §18).

/** Mail providers (`vProvider`). Gmail is label-based; Graph and IMAP are folder-based. */
export type Provider = "gmail" | "graph" | "imap";

/** An address (`vAddr`). `email` is lowercased and trimmed by whoever builds it from provider data. */
export interface Addr {
  name?: string;
  email: string;
}

/** How a `List-Unsubscribe` is performed (`unsubscribe` action). */
export type UnsubscribeMethod = "one_click_post" | "mailto";

/** One thread action (`vThreadAction`, ARCHITECTURE §5.1). Build these with `actionsFor()`, never by hand. */
export type ThreadAction =
  | { kind: "modify"; add: string[]; remove: string[] }
  | { kind: "trash" }
  | { kind: "untrash" }
  | { kind: "delete_forever" }
  | { kind: "move"; to: string }
  | { kind: "unsubscribe"; method: UnsubscribeMethod };

/** Triage categories (`vTriageCategory`). */
export type TriageCategory =
  | "important"
  | "personal"
  | "updates"
  | "newsletters"
  | "promotions"
  | "receipts"
  | "calendar"
  | "notifications"
  | "other";

/** One rule condition (`vRuleCond`, ARCHITECTURE §3.4). */
export type RuleCond =
  | { f: "from_email"; eq: string }
  | { f: "from_domain"; eq: string }
  | { f: "to_email"; eq: string }
  | { f: "label"; key: string }
  | { f: "triage"; category: string }
  | { f: "needs_reply"; eq: boolean }
  | { f: "is_bulk"; eq: boolean }
  | { f: "list_id"; eq: string }
  | { f: "subject_contains"; text: string }
  | { f: "vip"; eq: boolean }
  | { f: "account"; accountId: string };

/** A split or filter rule (`vRule`). */
export interface Rule {
  match: "all" | "any";
  conds: RuleCond[];
}
