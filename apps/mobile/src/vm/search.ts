// Search view-model (ARCHITECTURE §11): one grammar (packages/mail-model), parsed on the device for chips and
// errors, executed by `search.threads` (Convex) and, for what Convex cannot answer, by the worker's provider
// search ("Search the full mailbox").
import { formatDay, parseQuery, type QueryAst } from "@akatsuki/mail-model";

/** `search.threads` takes `nowAt` rounded down to the minute so the query cache stays effective (§11.1). */
export function searchNow(now: number): number {
  return Math.floor(now / 60_000) * 60_000;
}

/** Debounce for live search while typing (the server limit is 5/s per user, §13.4; clients debounce). */
export const SEARCH_DEBOUNCE_MS = 250;
export const MAX_QUERY_LENGTH = 500;

export interface Chip {
  label: string;
  negated: boolean;
}

function chipFor(ast: QueryAst, timeZone: string, negated = false): Chip[] {
  if ("and" in ast) return ast.and.flatMap((a) => chipFor(a, timeZone, negated));
  if ("or" in ast)
    return [
      {
        label: ast.or
          .map((a) =>
            chipFor(a, timeZone)
              .map((c) => c.label)
              .join(" "),
          )
          .join(" or "),
        negated,
      },
    ];
  if ("not" in ast) return chipFor(ast.not, timeZone, !negated);
  if ("text" in ast) return [{ label: ast.phrase ? `"${ast.text}"` : ast.text, negated }];
  if ("value" in ast) return [{ label: `${ast.field}: ${ast.value}`, negated }];
  if ("at" in ast) return [{ label: `${ast.field}: ${formatDay(ast.at, timeZone)}`, negated }];
  return [{ label: `${ast.field.replace("_", " ")}: ${ast.days}d`, negated }];
}

export interface SearchModel {
  query: string;
  chips: Chip[];
  errors: string[];
  /** Nothing to search for. */
  empty: boolean;
  tooLong: boolean;
}

export function searchModel(query: string, now: number, timeZone: string): SearchModel {
  const q = query.trim();
  if (!q) return { query: q, chips: [], errors: [], empty: true, tooLong: false };
  const { ast, errors } = parseQuery(q, { nowAt: searchNow(now), timezone: timeZone });
  return {
    query: q,
    chips: chipFor(ast, timeZone),
    errors,
    empty: false,
    tooLong: q.length > MAX_QUERY_LENGTH,
  };
}

/** Copy for the `unsupported` parts `search.threads` returns (§11.2: filename, OR across fields, body text). */
export function unsupportedNotice(unsupported: readonly string[]): string | null {
  if (unsupported.length === 0) return null;
  return `Results here skip ${unsupported.join(", ")}. Search the full mailbox to include them.`;
}

/** Search operators shown as suggestions under an empty field. */
export const OPERATOR_HINTS: readonly { label: string; insert: string }[] = [
  { label: "from:", insert: "from:" },
  { label: "to:", insert: "to:" },
  { label: "subject:", insert: "subject:" },
  { label: "is:unread", insert: "is:unread" },
  { label: "is:starred", insert: "is:starred" },
  { label: "has:attachment", insert: "has:attachment" },
  { label: "in:sent", insert: "in:sent" },
  { label: "newer_than:7d", insert: "newer_than:7d" },
];

/** Appends an operator to the query with one separating space. */
export function insertOperator(query: string, insert: string): string {
  const q = query.replace(/\s+$/, "");
  return q ? `${q} ${insert}` : insert;
}
