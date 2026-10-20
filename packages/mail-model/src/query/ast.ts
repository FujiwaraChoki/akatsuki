// Search query grammar (ARCHITECTURE §11.1, resolution rules D-094): AST types and value vocabularies.

export type StringField =
  | "from"
  | "to"
  | "cc"
  | "subject"
  | "label"
  | "in"
  | "is"
  | "has"
  | "list"
  | "filename"
  | "account"
  | "category";

export type QueryAst =
  | { and: QueryAst[] }
  | { or: QueryAst[] }
  | { not: QueryAst }
  | { text: string; phrase: boolean }
  | { field: StringField; value: string }
  | { field: "before" | "after"; at: number }
  | { field: "older_than" | "newer_than"; days: number };

/** `in:` values with fixed meaning (anything else is a split name). */
export const IN_VALUES = [
  "inbox",
  "sent",
  "drafts",
  "trash",
  "spam",
  "archive",
  "snoozed",
  "anywhere",
] as const;
/** `is:` values. */
export const IS_VALUES = ["unread", "read", "starred", "important", "snoozed", "needs_reply"] as const;
/** `has:` values. */
export const HAS_VALUES = ["attachment"] as const;
/** `category:` values (triage categories). */
export const CATEGORY_VALUES = [
  "important",
  "personal",
  "updates",
  "newsletters",
  "promotions",
  "receipts",
  "calendar",
  "notifications",
  "other",
] as const;

/** Maximum group nesting; deeper parentheses are reported and treated as plain separators. */
export const MAX_DEPTH = 32;
