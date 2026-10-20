// Snippets (ARCHITECTURE §3.2 `snippets`, §4.2 `snippets.*`, D-295): user-written templates with `{{variables}}`.
// Pure helpers shared by the clients that edit plain text (web composer, mobile composer): variable discovery
// and expansion, the built-in variables, the `;shortcut` trigger, and the plain text ↔ simple HTML conversion a
// snippet body is stored in. Convex re-sanitizes `bodyHtml` on write, so the HTML here only has to be simple.

/** An address as the composers hold it. */
export interface SnippetAddr {
  name?: string;
  email: string;
}

/** The fields of a snippet row the composers use (`SnippetView`). */
export interface SnippetLike {
  _id: string;
  name: string;
  shortcut?: string;
  subject?: string;
  bodyHtml: string;
  useCount?: number;
}

/** `{{ name }}`; names follow Convex's rule (`snippets.save`: lowercase, ≤ 32 characters). */
const VAR_RE = /\{\{\s*([a-z][a-z0-9_]{0,31})\s*\}\}/gi;

/** Shortcut names `snippets.save` accepts (stored lowercased). */
export const SNIPPET_SHORTCUT_RE = /^[a-z0-9][a-z0-9_-]{0,31}$/;

/** `snippets.save` limits. */
export const SNIPPET_BODY_MAX = 20 * 1024;
export const SNIPPET_NAME_MAX = 100;
export const SNIPPET_VARIABLES_MAX = 20;

/** The variables every composer fills in (D-295). */
export const BUILTIN_VARIABLES = ["first_name", "name", "email", "day", "date"] as const;

/** Variable names used in a template, lowercased, in order of first use. */
export function variablesIn(template: string): string[] {
  const out: string[] = [];
  for (const m of template.matchAll(VAR_RE)) {
    const v = m[1]!.toLowerCase();
    if (!out.includes(v)) out.push(v);
  }
  return out;
}

/** Fills `{{var}}`; unknown or empty variables stay as typed and are listed in `missing`. */
export function expandVariables(
  template: string,
  vars: Readonly<Record<string, string | undefined>>,
): { text: string; missing: string[] } {
  const missing: string[] = [];
  const text = template.replace(VAR_RE, (whole, name: string) => {
    const key = name.toLowerCase();
    const v = vars[key];
    if (v === undefined || v === "") {
      if (!missing.includes(key)) missing.push(key);
      return whole;
    }
    return v;
  });
  return { text, missing };
}

/** "Mara" from "Mara Okafor", "Okafor, Mara" or mara.okafor@…; "" when there is no recipient. */
export function firstName(a: SnippetAddr | undefined): string {
  const n = a?.name?.trim();
  if (n) return n.includes(",") ? n.split(",")[1]?.trim() || n : n.split(/\s+/)[0]!;
  const local = a?.email.split("@")[0] ?? "";
  if (!local) return "";
  const word = local.split(/[._+-]/)[0] ?? local;
  return word ? word[0]!.toUpperCase() + word.slice(1) : "";
}

/** The built-in variables from the first recipient and today in the user's time zone. */
export function builtinVariables(
  to: readonly SnippetAddr[],
  now: number,
  timeZone: string,
): Record<(typeof BUILTIN_VARIABLES)[number], string> {
  const first = to[0];
  const day = new Intl.DateTimeFormat("en-US", { weekday: "long", timeZone }).format(now);
  const date = new Intl.DateTimeFormat("en-US", { month: "long", day: "numeric", timeZone }).format(now);
  return {
    first_name: firstName(first),
    name: first?.name?.trim() || firstName(first),
    email: first?.email ?? "",
    day,
    date,
  };
}

/** A `;shortcut` typed right before the cursor: what to replace and which shortcut it names. */
export interface SnippetTrigger {
  start: number;
  end: number;
  shortcut: string;
}

/**
 * The `;shortcut` that ends at `cursor`, or null. The `;` must start the text or follow whitespace, so a `;`
 * inside a word or URL is not a trigger.
 */
export function snippetTrigger(text: string, cursor: number = text.length): SnippetTrigger | null {
  const end = Math.max(0, Math.min(cursor, text.length));
  const m = /(^|\s);([a-z0-9][a-z0-9_-]{0,31})$/i.exec(text.slice(0, end));
  if (!m) return null;
  const shortcut = m[2]!.toLowerCase();
  return { start: end - shortcut.length - 1, end, shortcut };
}

/** The snippet a trigger names (exact shortcut, case-insensitive). */
export function snippetForTrigger<T extends SnippetLike>(
  snippets: readonly T[],
  trigger: SnippetTrigger | null,
): T | undefined {
  if (!trigger) return undefined;
  return snippets.find((s) => s.shortcut?.toLowerCase() === trigger.shortcut);
}

/** Snippets for a picker: most used first, then by name; a query matches name, shortcut or text. */
export function rankSnippets<T extends SnippetLike>(snippets: readonly T[], query = ""): T[] {
  const q = query.trim().toLowerCase().replace(/^;/, "");
  const hit = (s: T) =>
    !q ||
    s.name.toLowerCase().includes(q) ||
    (s.shortcut?.toLowerCase().startsWith(q) ?? false) ||
    snippetHtmlToText(s.bodyHtml).toLowerCase().includes(q);
  const exact = (s: T) => (q && s.shortcut?.toLowerCase() === q ? 0 : 1);
  return snippets
    .filter(hit)
    .sort(
      (a, b) => exact(a) - exact(b) || (b.useCount ?? 0) - (a.useCount ?? 0) || a.name.localeCompare(b.name),
    );
}

/** Inserts `insert` over `[start, end)` of `text`; returns the text and the cursor after the insert. */
export function spliceText(
  text: string,
  start: number,
  end: number,
  insert: string,
): { text: string; cursor: number } {
  const s = Math.max(0, Math.min(start, text.length));
  const e = Math.max(s, Math.min(end, text.length));
  return { text: text.slice(0, s) + insert + text.slice(e), cursor: s + insert.length };
}

export interface AppliedSnippet {
  text: string;
  cursor: number;
  /** Set when the draft had no subject and the snippet has one. */
  subject: string | null;
  /** Variables the user still has to fill in. */
  missing: string[];
}

/** Applies a snippet at a range of the body (the trigger, or the selection). */
export function applySnippet(
  snippet: Pick<SnippetLike, "bodyHtml" | "subject">,
  draft: { text: string; subject: string; to: readonly SnippetAddr[] },
  range: { start: number; end: number },
  ctx: { now: number; timeZone: string },
): AppliedSnippet {
  const { text: body, missing } = expandVariables(
    snippetHtmlToText(snippet.bodyHtml),
    builtinVariables(draft.to, ctx.now, ctx.timeZone),
  );
  const spliced = spliceText(draft.text, range.start, range.end, body);
  return {
    ...spliced,
    subject: snippet.subject?.trim() && !draft.subject.trim() ? snippet.subject.trim() : null,
    missing,
  };
}

/** Problems with a snippet the user is editing (the same rules `snippets.save` enforces), in display order. */
export function snippetProblems(input: { name: string; shortcut: string; text: string }): string[] {
  const problems: string[] = [];
  const name = input.name.trim();
  if (!name) problems.push("Give it a name.");
  else if (name.length > SNIPPET_NAME_MAX) problems.push("Names are at most 100 characters.");
  if (!input.text.trim()) problems.push("Write the snippet text.");
  const sc = input.shortcut.trim().replace(/^;/, "").toLowerCase();
  if (sc && !SNIPPET_SHORTCUT_RE.test(sc))
    problems.push("Shortcuts use letters, digits, - and _ only (up to 32).");
  if (variablesIn(input.text).length > SNIPPET_VARIABLES_MAX) problems.push("At most 20 variables.");
  if (snippetTextToHtml(input.text).length > SNIPPET_BODY_MAX)
    problems.push("Snippets are limited to 20 KB.");
  return problems;
}

/** The `snippets.save` arguments for a plain-text snippet (the body is stored as simple HTML). */
export function snippetSaveArgs(input: {
  snippetId?: string;
  name: string;
  shortcut: string;
  subject: string;
  text: string;
}): {
  snippetId?: string;
  name: string;
  shortcut?: string;
  subject?: string;
  bodyHtml: string;
  variables: string[];
} {
  const sc = input.shortcut.trim().replace(/^;/, "").toLowerCase();
  return {
    ...(input.snippetId ? { snippetId: input.snippetId } : {}),
    name: input.name.trim(),
    ...(sc ? { shortcut: sc } : {}),
    ...(input.subject.trim() ? { subject: input.subject.trim() } : {}),
    bodyHtml: snippetTextToHtml(input.text),
    variables: variablesIn(input.text).slice(0, SNIPPET_VARIABLES_MAX),
  };
}

// ───────────── plain text ↔ simple HTML ─────────────

function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}

/** Plain text → `<p>` paragraphs with `<br>` (blank lines separate paragraphs). Everything is escaped. */
export function snippetTextToHtml(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .split(/\n{2,}/)
    .filter((block) => block.trim() !== "")
    .map((block) => `<p>${block.split("\n").map(escapeHtml).join("<br>")}</p>`)
    .join("\n");
}

const ENTITIES: Record<string, string> = {
  "&nbsp;": " ",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
  "&#x27;": "'",
  "&amp;": "&",
};

/** Snippet or signature HTML → plain text for a text editor (no DOM needed). */
export function snippetHtmlToText(html: string): string {
  return html
    .replace(/<(script|style)\b[\s\S]*?<\/\1>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<li\b[^>]*>/gi, "\n- ")
    .replace(/<\/(p|div|h[1-6]|blockquote|pre|ul|ol|tr)>/gi, "\n\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&(nbsp|lt|gt|quot|#39|#x27|amp);/g, (e) => ENTITIES[e] ?? e)
    .split("\n")
    .map((l) => l.replace(/[ \t]+$/g, ""))
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
