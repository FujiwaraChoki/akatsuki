// Canonical query text (crates/search::query::normalize twin): `normalized` in fixtures/query.json.
import type { QueryAst } from "./ast.ts";
import { isWs } from "./chars.ts";
import { formatDay } from "./dates.ts";

// ───────────── normalize (crates/search::query::normalize) ─────────────

function quote(s: string): string {
  let out = '"';
  for (const c of s) out += c === '"' || c === "\\" ? `\\${c}` : c;
  return `${out}"`;
}

function needsQuotes(s: string): boolean {
  if (s === "" || s === "OR") return true;
  for (const c of s) if (isWs(c) || c === '"' || c === "(" || c === ")") return true;
  return false;
}

const valueStr = (s: string) => (needsQuotes(s) ? quote(s) : s);

/** Renders an AST back to canonical query text (`normalized` in the fixtures); dates as `YYYY-MM-DD` in `timezone`. */
export function normalizeQuery(ast: QueryAst, timezone: string): string {
  if ("and" in ast)
    return ast.and
      .map((i) => ("or" in i ? `(${normalizeQuery(i, timezone)})` : normalizeQuery(i, timezone)))
      .join(" ");
  if ("or" in ast)
    return ast.or
      .map((i) =>
        "and" in i && i.and.length !== 1 ? `(${normalizeQuery(i, timezone)})` : normalizeQuery(i, timezone),
      )
      .join(" OR ");
  if ("not" in ast) {
    const inner = ast.not;
    return "and" in inner || "or" in inner
      ? `-(${normalizeQuery(inner, timezone)})`
      : `-${normalizeQuery(inner, timezone)}`;
  }
  if ("text" in ast)
    return ast.phrase || needsQuotes(ast.text) || ast.text.startsWith("-") ? quote(ast.text) : ast.text;
  if ("value" in ast) return `${ast.field}:${valueStr(ast.value)}`;
  if ("at" in ast) return `${ast.field}:${formatDay(ast.at, timezone)}`;
  return `${ast.field}:${ast.days}d`;
}
