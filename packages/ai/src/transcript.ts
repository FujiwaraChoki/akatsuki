// Shared renderers that turn mail metadata into prompt variables. The spec is
// packages/prompts/shared/transcript.md; crates/ai::transcript must produce byte-identical output for
// packages/prompts/shared/fixtures/transcript/*.json and fixtures/triage-items/*.json.

export interface Addr {
  name?: string | null;
  email: string;
}

export interface TranscriptMessage {
  from?: Addr | null;
  to: Addr[];
  cc?: Addr[];
  /** ms since the Unix epoch. */
  date: number;
  /** Plain text with quoted history already stripped (crates/mime quote detection). */
  text: string;
}

export interface ThreadForTranscript {
  /** IANA zone name, `profiles.timezone`. */
  timezone: string;
  maxInputChars?: number;
  messages: TranscriptMessage[];
}

export const MESSAGE_TEXT_MAX = 8000;
export const DEFAULT_MAX_INPUT_CHARS = 120_000;
const CUT_MARKER = "\n[…]";

/** Unicode scalar values, so TypeScript and Rust (`chars()`) count and cut identically. */
const chars = (s: string) => Array.from(s);
export const charLength = (s: string) => chars(s).length;
function cutChars(s: string, max: number): string {
  const c = chars(s);
  return c.length <= max ? s : c.slice(0, max).join("");
}

/**
 * The one whitespace set of both renderers (here and `crates/ai::transcript`): Unicode `White_Space`
 * (Rust `char::is_whitespace`) plus U+FEFF. JavaScript's own `trim` and `\s` differ from Rust's: they
 * include U+FEFF but not U+0085 (NEXT LINE), so neither is used here (D-342). Every member is in the BMP.
 */
const WHITESPACE = new Set([
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005,
  0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff,
]);
const WHITESPACE_RUN = /[\t\n\v\f\r \u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000\ufeff]+/g;

/** True for a character of the shared renderer whitespace set. */
export function isRenderWhitespace(ch: string): boolean {
  return ch.length === 1 && WHITESPACE.has(ch.charCodeAt(0));
}

/** `trimEnd` over the shared whitespace set (Rust `trim_end_matches(is_render_whitespace)`). */
export function trimEndWhitespace(s: string): string {
  let end = s.length;
  while (end > 0 && WHITESPACE.has(s.charCodeAt(end - 1))) end--;
  return s.slice(0, end);
}

/** `trim` over the shared whitespace set. */
export function trimWhitespace(s: string): string {
  let start = 0;
  while (start < s.length && WHITESPACE.has(s.charCodeAt(start))) start++;
  return trimEndWhitespace(s.slice(start));
}

export function formatAddr(a: Addr | null | undefined): string {
  if (!a) return "";
  const name = a.name == null ? "" : trimWhitespace(a.name);
  return name ? `${name} <${a.email}>` : a.email;
}

const pad = (n: number, w = 2) => String(Math.abs(n)).padStart(w, "0");

/** `YYYY-MM-DDTHH:mm:ss±HH:MM` in `timeZone` (the offset in effect at that instant; never `Z`). */
export function formatInstant(ms: number, timeZone: string): string {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).formatToParts(new Date(ms));
  const get = (type: string) => Number(parts.find((p) => p.type === type)?.value);
  const [y, mo, d, h, mi, s] = [
    get("year"),
    get("month"),
    get("day"),
    get("hour"),
    get("minute"),
    get("second"),
  ];
  const wall = Date.UTC(y, mo - 1, d, h, mi, s);
  const offsetMin = Math.round((wall - Math.floor(ms / 1000) * 1000) / 60000);
  const sign = offsetMin < 0 ? "-" : "+";
  return `${pad(y, 4)}-${pad(mo)}-${pad(d)}T${pad(h)}:${pad(mi)}:${pad(s)}${sign}${pad(Math.trunc(offsetMin / 60))}:${pad(offsetMin % 60)}`;
}

function normalizeText(text: string): string {
  const t = trimEndWhitespace(text.replace(/\r\n?/g, "\n"));
  return charLength(t) > MESSAGE_TEXT_MAX ? cutChars(t, MESSAGE_TEXT_MAX) + CUT_MARKER : t;
}

function renderMessage(m: TranscriptMessage, timeZone: string): string {
  const lines = [
    `From: ${formatAddr(m.from)}`,
    `Date: ${formatInstant(m.date, timeZone)}`,
    `To: ${m.to.map(formatAddr).join(", ")}`,
  ];
  if (m.cc && m.cc.length > 0) lines.push(`Cc: ${m.cc.map(formatAddr).join(", ")}`);
  return `${lines.join("\n")}\n\n${normalizeText(m.text)}`;
}

/** Renders `{{transcript}}`: oldest first, `\n---\n` between messages, capped at maxInputChars. */
export function renderTranscript(thread: ThreadForTranscript): string {
  const max = thread.maxInputChars ?? DEFAULT_MAX_INPUT_CHARS;
  const sorted = thread.messages
    .map((m, i) => ({ m, i }))
    .sort((a, b) => a.m.date - b.m.date || a.i - b.i)
    .map(({ m }) => renderMessage(m, thread.timezone));
  const sizes = sorted.map(charLength);
  let start = 0;
  let total = sizes.reduce((a, b) => a + b, 0) + Math.max(0, sorted.length - 1) * 5;
  while (total > max && start < sorted.length - 1) {
    total -= sizes[start]! + 5;
    start += 1;
  }
  const out = sorted.slice(start).join("\n---\n");
  if (charLength(out) <= max) return out;
  return cutChars(out, Math.max(0, max - charLength(CUT_MARKER))) + CUT_MARKER;
}

/** `TriageInput` (convex/lib/rows.ts): headers and snippet only, never body text. */
export interface TriageInput {
  threadId: string;
  from?: Addr | null;
  to: Addr[];
  subject: string;
  snippet: string;
  listId?: string | null;
  isBulk: boolean;
  hasAttachments: boolean;
}

export const TRIAGE_BATCH_MAX = 25;

/** Collapses every run of the shared whitespace set to one space and trims (Rust `one_line`). */
const oneLine = (s: string) => trimWhitespace(s.replace(WHITESPACE_RUN, " "));

/**
 * Renders `{{items}}` for the triage feature. Each item gets the key `String(index + 1)`; blank line between
 * items. Format (one field per line): key, from, to, subject, list-id (only when set), bulk, attachments,
 * snippet. Whitespace runs inside values collapse to one space.
 */
export function renderTriageItems(items: TriageInput[]): { text: string; keys: Map<string, string> } {
  if (items.length > TRIAGE_BATCH_MAX) throw new RangeError(`at most ${TRIAGE_BATCH_MAX} triage items`);
  const keys = new Map<string, string>();
  const blocks = items.map((it, i) => {
    const key = String(i + 1);
    keys.set(key, it.threadId);
    const lines = [
      `key: ${key}`,
      `from: ${oneLine(formatAddr(it.from))}`,
      `to: ${oneLine(it.to.map(formatAddr).join(", "))}`,
      `subject: ${oneLine(it.subject)}`,
    ];
    if (it.listId) lines.push(`list-id: ${oneLine(it.listId)}`);
    lines.push(`bulk: ${it.isBulk ? "yes" : "no"}`, `attachments: ${it.hasAttachments ? "yes" : "no"}`);
    lines.push(`snippet: ${oneLine(it.snippet)}`);
    return lines.join("\n");
  });
  return { text: blocks.join("\n\n"), keys };
}
