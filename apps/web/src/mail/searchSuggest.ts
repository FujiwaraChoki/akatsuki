// Search-as-you-type suggestions for the §11.1 grammar: operators, then values for the operator being typed
// (contacts for from:/to:/cc:, label and split names, the fixed is:/has:/in:/category: values).
import { CATEGORY_VALUES, HAS_VALUES, IN_VALUES, IS_VALUES } from "@akatsuki/mail-model";

export interface Suggestion {
  /** What the list shows. */
  label: string;
  /** Replaces the token being typed. */
  insert: string;
  hint?: string;
}

export interface SuggestContext {
  contacts: readonly { email: string; name?: string }[];
  labels: readonly string[];
  splits: readonly string[];
  /** The last search run, offered first while the field is empty (the field itself is cleared on leaving). */
  recent?: string;
}

const OPERATORS: { op: string; hint: string }[] = [
  { op: "from:", hint: "Sender" },
  { op: "to:", hint: "Recipient" },
  { op: "subject:", hint: "Words in the subject" },
  { op: "is:unread", hint: "Unread conversations" },
  { op: "is:starred", hint: "Starred conversations" },
  { op: "has:attachment", hint: "With attachments" },
  { op: "in:inbox", hint: "Only the inbox" },
  { op: "in:sent", hint: "Sent by you" },
  { op: "in:anywhere", hint: "Including spam and trash" },
  { op: "label:", hint: "A label" },
  { op: "after:", hint: "After a date (YYYY-MM-DD)" },
  { op: "before:", hint: "Before a date (YYYY-MM-DD)" },
  { op: "newer_than:", hint: "e.g. 7d, 2w, 1m" },
  { op: "older_than:", hint: "e.g. 30d, 1y" },
  { op: "category:", hint: "Triage category" },
  { op: "list:", hint: "Mailing list id" },
  { op: "account:", hint: "One mailbox (address)" },
];

/** The token at the end of `input` (the one being typed) and where it starts. */
export function lastToken(input: string): { token: string; start: number } {
  let i = input.length;
  let inQuote = false;
  for (let j = 0; j < input.length; j++) if (input[j] === '"') inQuote = !inQuote;
  while (i > 0) {
    const c = input[i - 1]!;
    if (c === '"') inQuote = !inQuote;
    if (!inQuote && /\s|\(/.test(c)) break;
    i--;
  }
  return { token: input.slice(i), start: i };
}

const quote = (v: string) => (/[\s"()]/.test(v) ? `"${v.replace(/(["\\])/g, "\\$1")}"` : v);

export function suggest(input: string, ctx: SuggestContext, max = 8): Suggestion[] {
  const { token } = lastToken(input);
  const neg = token.startsWith("-") ? "-" : "";
  const t = token.slice(neg.length);
  const lower = t.toLowerCase();
  const colon = lower.indexOf(":");
  if (colon === -1) {
    if (lower.length === 0) {
      const ops = OPERATORS.map((o) => ({ label: o.op, insert: o.op, hint: o.hint }));
      const recent = input.trim() === "" ? ctx.recent?.trim() : undefined;
      return (recent ? [{ label: recent, insert: recent, hint: "Last search" }, ...ops] : ops).slice(0, max);
    }
    return OPERATORS.filter((o) => o.op.startsWith(lower) && o.op !== lower)
      .slice(0, max)
      .map((o) => ({ label: neg + o.op, insert: neg + o.op, hint: o.hint }));
  }
  const field = lower.slice(0, colon);
  const value = t
    .slice(colon + 1)
    .replace(/^"/, "")
    .toLowerCase();
  const values = (xs: readonly string[], hint?: string) =>
    xs
      .filter((x) => x.toLowerCase().startsWith(value) && x.toLowerCase() !== value)
      .slice(0, max)
      .map((x) => ({
        label: `${neg}${field}:${quote(x)}`,
        insert: `${neg}${field}:${quote(x)}`,
        ...(hint && { hint }),
      }));
  switch (field) {
    case "from":
    case "to":
    case "cc":
      return ctx.contacts
        .filter((c) => !value || c.email.includes(value) || (c.name ?? "").toLowerCase().includes(value))
        .slice(0, max)
        .map((c) => ({
          label: `${neg}${field}:${c.email}`,
          insert: `${neg}${field}:${c.email}`,
          ...(c.name && { hint: c.name }),
        }));
    case "is":
      return values(IS_VALUES);
    case "has":
      return values(HAS_VALUES);
    case "in":
      return values([...IN_VALUES, ...ctx.splits.map((s) => s.toLowerCase())]);
    case "category":
      return values(CATEGORY_VALUES);
    case "label":
      return values(ctx.labels, "Label");
    default:
      return [];
  }
}

/** Replaces the token being typed with `insert` and adds a trailing space (unless the value is still open). */
export function applySuggestion(input: string, s: Suggestion): string {
  const { start } = lastToken(input);
  const open = s.insert.endsWith(":");
  return `${input.slice(0, start)}${s.insert}${open ? "" : " "}`;
}
