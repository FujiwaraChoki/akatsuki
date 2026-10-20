// The search query parser (crates/search::query::parse twin, ARCHITECTURE §11.1, D-094).
import {
  CATEGORY_VALUES,
  HAS_VALUES,
  IN_VALUES,
  IS_VALUES,
  MAX_DEPTH,
  type QueryAst,
  type StringField,
} from "./ast.ts";
import { asciiLower, isWs } from "./chars.ts";
import { relativeDays, startOfDayMs } from "./dates.ts";

type FieldKind =
  | { k: "str"; field: StringField }
  | { k: "date"; field: "before" | "after" }
  | { k: "age"; field: "older_than" | "newer_than" };

const FIELD_KINDS = new Map<string, FieldKind>(
  Object.entries({
    from: { k: "str", field: "from" },
    to: { k: "str", field: "to" },
    cc: { k: "str", field: "cc" },
    subject: { k: "str", field: "subject" },
    label: { k: "str", field: "label" },
    in: { k: "str", field: "in" },
    is: { k: "str", field: "is" },
    has: { k: "str", field: "has" },
    list: { k: "str", field: "list" },
    filename: { k: "str", field: "filename" },
    account: { k: "str", field: "account" },
    category: { k: "str", field: "category" },
    before: { k: "date", field: "before" },
    after: { k: "date", field: "after" },
    older_than: { k: "age", field: "older_than" },
    newer_than: { k: "age", field: "newer_than" },
  } satisfies Record<string, FieldKind>),
);

// ───────────── parser (crates/search::query::parse) ─────────────

const EMPTY = (): QueryAst => ({ and: [] });
const isEmpty = (a: QueryAst) => "and" in a && a.and.length === 0;

function flattenAnd(items: QueryAst[]): QueryAst {
  const out: QueryAst[] = [];
  for (const i of items) {
    if ("and" in i) out.push(...i.and);
    else out.push(i);
  }
  return out.length === 1 ? out[0]! : { and: out };
}

class Parser {
  pos = 0;
  errors: string[] = [];
  depth = 0;
  tooDeep = false;
  readonly chars: string[];
  readonly timezone: string;
  constructor(chars: string[], timezone: string) {
    this.chars = chars;
    this.timezone = timezone;
  }

  atEnd() {
    return this.pos >= this.chars.length;
  }
  peek(): string | undefined {
    return this.chars[this.pos];
  }
  skipWs(): boolean {
    const start = this.pos;
    while (isWs(this.peek())) this.pos++;
    return this.pos > start;
  }
  /** `OR` as a standalone token (followed by whitespace or the end). */
  atOr(): boolean {
    const next = this.chars[this.pos + 2];
    return (
      this.chars[this.pos] === "O" && this.chars[this.pos + 1] === "R" && (next === undefined || isWs(next))
    );
  }

  orExpr(): QueryAst {
    const alts = [this.andExpr()];
    for (;;) {
      const save = this.pos;
      this.skipWs();
      if (this.atOr() && save < this.pos) {
        this.pos += 2;
        if (!this.skipWs() || this.atEnd() || this.peek() === ")") {
          // "a OR" at the end: OR is a plain word.
          alts.push({ text: "OR", phrase: false });
          this.errors.push("OR needs a term on both sides");
          continue;
        }
        alts.push(this.andExpr());
      } else {
        this.pos = save;
        break;
      }
    }
    return alts.length === 1 ? alts[0]! : { or: alts };
  }

  andExpr(): QueryAst {
    const items: QueryAst[] = [];
    for (;;) {
      const save = this.pos;
      this.skipWs();
      if (this.atEnd() || this.peek() === ")") {
        this.pos = save;
        break;
      }
      if (this.atOr() && items.length > 0) {
        this.pos = save;
        break;
      }
      if (this.atOr()) {
        // Leading "OR": a plain word.
        this.pos += 2;
        this.errors.push("OR needs a term on both sides");
        items.push({ text: "OR", phrase: false });
        continue;
      }
      const item = this.unary();
      if (!isEmpty(item)) items.push(item);
    }
    return items.length === 1 ? items[0]! : { and: items };
  }

  unary(): QueryAst {
    if (this.peek() === "-") {
      const next = this.chars[this.pos + 1];
      if (next !== undefined && !isWs(next) && next !== ")") {
        this.pos++;
        return { not: this.atom() };
      }
    }
    return this.atom();
  }

  atom(): QueryAst {
    const c = this.peek();
    if (c === "(") {
      const open = this.pos;
      this.pos++;
      if (this.depth >= MAX_DEPTH) {
        // Too deep: the "(" is dropped (its ")" is reported as unmatched later).
        if (!this.tooDeep) {
          this.tooDeep = true;
          this.errors.push(`groups nested too deeply at ${open}`);
        }
        return EMPTY();
      }
      this.depth++;
      const inner = this.orExprInGroup();
      this.depth--;
      return inner;
    }
    if (c === '"') return { text: this.phrase(), phrase: true };
    return this.wordOrField();
  }

  orExprInGroup(): QueryAst {
    const open = Math.max(0, this.pos - 1);
    this.skipWs();
    const inner = this.peek() === ")" ? EMPTY() : this.orExpr();
    this.skipWs();
    if (this.peek() === ")") this.pos++;
    else this.errors.push(`unmatched '(' at ${open}`);
    return inner;
  }

  phrase(): string {
    const start = this.pos;
    this.pos++; // opening quote
    let out = "";
    for (let c = this.peek(); c !== undefined; c = this.peek()) {
      this.pos++;
      if (c === '"') return out;
      if (c === "\\") {
        const n = this.peek();
        if (n === '"' || n === "\\") {
          out += n;
          this.pos++;
        } else out += "\\";
      } else out += c;
    }
    this.errors.push(`unterminated quote at ${start}`);
    return out;
  }

  word(): string {
    let out = "";
    for (let c = this.peek(); c !== undefined; c = this.peek()) {
      if (isWs(c) || c === '"' || c === "(" || c === ")") break;
      out += c;
      this.pos++;
    }
    return out;
  }

  wordOrField(): QueryAst {
    const start = this.pos;
    let name = "";
    let i = this.pos;
    for (let c = this.chars[i]; c !== undefined; c = this.chars[++i]) {
      if (c === ":" || isWs(c) || c === '"' || c === "(" || c === ")") break;
      name += c;
    }
    const kind = this.chars[i] === ":" ? FIELD_KINDS.get(asciiLower(name)) : undefined;
    if (kind) {
      this.pos = i + 1;
      const quoted = this.peek() === '"';
      const value = quoted ? this.phrase() : this.word();
      const raw = this.chars.slice(start, this.pos).join("");
      if (value === "" && !quoted) {
        this.errors.push(`missing value for ${asciiLower(name)}:`);
        return { text: raw, phrase: false };
      }
      return this.field(kind, value, raw);
    }
    const word = this.word();
    if (word === "") {
      // Defensive: never loop on an unexpected character.
      const c = this.peek() ?? "";
      this.pos++;
      return { text: c, phrase: false };
    }
    return { text: word, phrase: false };
  }

  field(kind: FieldKind, value: string, raw: string): QueryAst {
    if (kind.k === "str") {
      const { field } = kind;
      const lower = value.toLowerCase();
      const v =
        field === "is" || field === "has" || field === "category"
          ? lower
          : field === "in" && (IN_VALUES as readonly string[]).includes(lower)
            ? lower
            : value;
      const known: readonly string[] | undefined =
        field === "is"
          ? IS_VALUES
          : field === "has"
            ? HAS_VALUES
            : field === "category"
              ? CATEGORY_VALUES
              : undefined;
      if (known && !known.includes(v)) this.errors.push(`unknown value for ${field}: ${v}`);
      return { field, value: v };
    }
    if (kind.k === "date") {
      const r = startOfDayMs(value, this.timezone);
      if (r.ok) return { field: kind.field, at: r.at };
      this.errors.push(r.error);
      return { text: raw, phrase: false };
    }
    const days = relativeDays(value);
    if (days !== null) return { field: kind.field, days };
    this.errors.push(`invalid relative date: ${value}`);
    return { text: raw, phrase: false };
  }
}

/**
 * Parses a query. `nowAt` is accepted for the contract signature (§11.1) and used by callers to resolve
 * `older_than`/`newer_than` days; the parser itself only needs `timezone`.
 */
export function parseQuery(
  q: string,
  opts: { nowAt: number; timezone: string },
): { ast: QueryAst; errors: string[] } {
  const p = new Parser(Array.from(q), opts.timezone);
  p.skipWs();
  let ast = p.atEnd() ? EMPTY() : p.orExpr();
  for (;;) {
    p.skipWs();
    if (p.atEnd()) break;
    if (p.peek() === ")") {
      // A stray ")" at top level: reported (at most 16 errors) and skipped.
      if (p.errors.length < 16) p.errors.push(`unmatched ')' at ${p.pos}`);
      p.pos++;
      continue;
    }
    // Anything after a stray ")" is parsed and AND-ed on.
    ast = flattenAnd([ast, p.orExpr()]);
  }
  return { ast, errors: p.errors };
}
