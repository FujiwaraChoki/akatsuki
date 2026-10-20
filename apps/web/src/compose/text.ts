// Pure text helpers for the composer: plain text ↔ HTML, address lists, snippet variables, subjects, quoting.
// The composer edits plain text (D-280); the HTML part sent to the worker is derived here and re-sanitized by
// the worker (`crates/mime::compose`, §7.3), which also inserts the identity's signature.

export interface Addr {
  name?: string;
  email: string;
}

export function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!,
  );
}

const URL_RE = /\bhttps?:\/\/[^\s<>"']+[^\s<>"'.,;:!?)\]]/g;

/** Escapes a line and turns http(s) URLs into links (nothing else: no javascript:, no data:). */
function inline(line: string): string {
  let out = "";
  let last = 0;
  for (const m of line.matchAll(URL_RE)) {
    out += escapeHtml(line.slice(last, m.index));
    const url = m[0];
    out += `<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`;
    last = m.index! + url.length;
  }
  return out + escapeHtml(line.slice(last));
}

/**
 * Plain text → simple HTML: blank lines separate paragraphs, single newlines become `<br>`, lines starting with
 * `>` become a blockquote, URLs become links. Everything else is escaped.
 */
export function textToHtml(text: string): string {
  const blocks = text.replace(/\r\n?/g, "\n").split(/\n{2,}/);
  const out: string[] = [];
  for (const block of blocks) {
    if (block.trim() === "") continue;
    // Runs of quoted (`>`) and plain lines, so "On …, Mara wrote:" followed by its quote renders right.
    let run: string[] = [];
    let quoted = false;
    const flush = () => {
      if (run.length === 0) return;
      if (quoted)
        out.push(`<blockquote>${textToHtml(run.map((l) => l.replace(/^> ?/, "")).join("\n"))}</blockquote>`);
      else out.push(`<p>${run.map(inline).join("<br>")}</p>`);
      run = [];
    };
    for (const line of block.split("\n")) {
      const q = line.startsWith(">");
      if (q !== quoted) {
        flush();
        quoted = q;
      }
      run.push(line);
    }
    flush();
  }
  return out.join("\n");
}

const BLOCK_TAGS = new Set(["P", "DIV", "LI", "TR", "H1", "H2", "H3", "H4", "H5", "H6", "BLOCKQUOTE", "PRE"]);

/** HTML (snippets, signatures, adopted drafts) → plain text for the editor. Scripts and styles are dropped. */
export function htmlToText(html: string): string {
  if (typeof DOMParser === "undefined") {
    return html
      .replace(/<(script|style)[\s\S]*?<\/\1>/gi, "")
      .replace(/<br\s*\/?>/gi, "\n")
      .replace(/<\/(p|div|li|h\d|blockquote)>/gi, "\n\n")
      .replace(/<[^>]+>/g, "")
      .replace(/&nbsp;/g, " ")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&quot;/g, '"')
      .replace(/&#39;/g, "'")
      .replace(/&amp;/g, "&")
      .replace(/\n{3,}/g, "\n\n")
      .trim();
  }
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
  const walk = (node: Node, quote: number): string => {
    if (node.nodeType === 3) return (node.textContent ?? "").replace(/\s+/g, " ");
    if (node.nodeType !== 1) return "";
    const el = node as Element;
    const tag = el.tagName;
    if (tag === "SCRIPT" || tag === "STYLE" || tag === "HEAD") return "";
    if (tag === "BR") return "\n";
    const inner = [...el.childNodes].map((c) => walk(c, tag === "BLOCKQUOTE" ? quote + 1 : quote)).join("");
    if (tag === "BLOCKQUOTE")
      return `\n\n${inner
        .trim()
        .split("\n")
        .map((l) => `> ${l}`.trimEnd())
        .join("\n")}\n\n`;
    if (tag === "LI") return `\n- ${inner.trim()}`;
    if (BLOCK_TAGS.has(tag)) return `\n\n${inner.trim()}\n\n`;
    return inner;
  };
  return walk(doc.body, 0)
    .split("\n")
    .map((l) => l.replace(/[ \t]+$/g, "").replace(/^ +/, ""))
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

const EMAIL_RE = /^[^\s@<>(),;:"]+@[^\s@<>(),;:"]+\.[^\s@<>(),;:"]+$/;

export function isEmail(s: string): boolean {
  return EMAIL_RE.test(s.trim());
}

/** Splits on commas and semicolons outside quotes and angle brackets. */
function splitList(input: string): string[] {
  const out: string[] = [];
  let cur = "";
  let quoted = false;
  let angle = false;
  for (const ch of input) {
    if (ch === '"') quoted = !quoted;
    else if (ch === "<") angle = true;
    else if (ch === ">") angle = false;
    if ((ch === "," || ch === ";") && !quoted && !angle) {
      out.push(cur);
      cur = "";
    } else cur += ch;
  }
  out.push(cur);
  return out.map((s) => s.trim()).filter(Boolean);
}

/** `Name <a@b.c>`, `"Last, First" <a@b.c>` or `a@b.c`. Addresses are lowercased (§3.1 vAddr). */
export function parseAddr(token: string): Addr | null {
  const t = token.trim();
  const angle = /^(.*)<([^<>]+)>$/.exec(t);
  if (angle) {
    const email = angle[2]!.trim().toLowerCase();
    if (!isEmail(email)) return null;
    const name = angle[1]!
      .trim()
      .replace(/^"(.*)"$/, "$1")
      .trim();
    return name ? { name, email } : { email };
  }
  return isEmail(t) ? { email: t.toLowerCase() } : null;
}

export function parseAddressList(input: string): { valid: Addr[]; invalid: string[] } {
  const valid: Addr[] = [];
  const invalid: string[] = [];
  for (const token of splitList(input)) {
    const a = parseAddr(token);
    if (a) {
      if (!valid.some((v) => v.email === a.email)) valid.push(a);
    } else invalid.push(token);
  }
  return { valid, invalid };
}

export function formatAddr(a: Addr): string {
  if (!a.name) return a.email;
  const name = /[",;<>@]/.test(a.name) ? `"${a.name.replace(/"/g, "")}"` : a.name;
  return `${name} <${a.email}>`;
}

export function firstName(a: Addr | undefined): string {
  const n = a?.name?.trim();
  if (n) return n.includes(",") ? (n.split(",")[1]?.trim() ?? n) : n.split(/\s+/)[0]!;
  const local = a?.email.split("@")[0] ?? "";
  return local ? local[0]!.toUpperCase() + local.slice(1).split(/[._-]/)[0]! : "";
}

const VAR_RE = /\{\{\s*([a-z][a-z0-9_]{0,39})\s*\}\}/gi;

/** Variable names used in a snippet body (`{{first_name}}`), in order of first use. */
export function variablesIn(template: string): string[] {
  const out: string[] = [];
  for (const m of template.matchAll(VAR_RE)) {
    const v = m[1]!.toLowerCase();
    if (!out.includes(v)) out.push(v);
  }
  return out;
}

/** Fills `{{var}}`; unknown variables stay as typed and are reported in `missing`. */
export function expandVariables(
  template: string,
  vars: Record<string, string | undefined>,
): { text: string; missing: string[] } {
  const missing: string[] = [];
  const text = template.replace(VAR_RE, (whole, name: string) => {
    const v = vars[name.toLowerCase()];
    if (v === undefined || v === "") {
      if (!missing.includes(name.toLowerCase())) missing.push(name.toLowerCase());
      return whole;
    }
    return v;
  });
  return { text, missing };
}

/** Built-in snippet variables from the first recipient and the clock (user time zone). */
export function builtinVariables(to: readonly Addr[], now: number, tz: string): Record<string, string> {
  const first = to[0];
  const day = new Intl.DateTimeFormat("en-US", { weekday: "long", timeZone: tz }).format(now);
  const date = new Intl.DateTimeFormat("en-US", { month: "long", day: "numeric", timeZone: tz }).format(now);
  return {
    first_name: firstName(first),
    name: first?.name ?? firstName(first),
    email: first?.email ?? "",
    day,
    date,
  };
}

const RE_PREFIX = /^(re|aw|sv|antw)\s*:\s*/i;
const FWD_PREFIX = /^(fwd?|wg)\s*:\s*/i;

export function replySubject(subject: string): string {
  const s = subject.trim();
  return RE_PREFIX.test(s) ? s : `Re: ${s || "(no subject)"}`;
}

export function forwardSubject(subject: string): string {
  const s = subject.trim().replace(RE_PREFIX, "");
  return FWD_PREFIX.test(s) ? s : `Fwd: ${s || "(no subject)"}`;
}

/** Prefixes each line with `> ` (the plain-text quote of a reply). */
export function quoteLines(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .trimEnd()
    .split("\n")
    .map((l) => (l ? `> ${l}` : ">"))
    .join("\n");
}
