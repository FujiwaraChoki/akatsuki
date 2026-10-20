// Allow-list sanitizer for user-authored HTML (snippets, signatures) on write (ARCHITECTURE §3.3, §14.3).
// The worker sanitizes again with crates/mime at send time; this is the first line, not the only one.
// It tokenizes tags, keeps a small set of formatting elements, keeps only safe attributes, drops the
// contents of dangerous elements entirely and escapes everything else.

const ALLOWED_TAGS = new Set([
  "a",
  "b",
  "blockquote",
  "br",
  "code",
  "div",
  "em",
  "h1",
  "h2",
  "h3",
  "hr",
  "i",
  "li",
  "ol",
  "p",
  "pre",
  "s",
  "span",
  "strong",
  "sub",
  "sup",
  "u",
  "ul",
]);
const VOID_TAGS = new Set(["br", "hr"]);
/** Elements whose content is dropped with them. */
const DROP_WITH_CONTENT = new Set([
  "script",
  "style",
  "iframe",
  "object",
  "embed",
  "template",
  "noscript",
  "svg",
  "math",
  "textarea",
  "select",
  "title",
  "head",
  "frameset",
  "frame",
  "applet",
]);
const SAFE_URL = /^(https?:|mailto:)/i;

const escapeText = (s: string) =>
  s
    .replace(/&(?!(#\d+|#x[0-9a-f]+|[a-z][a-z0-9]*);)/gi, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
const escapeAttr = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

function decodeEntities(s: string): string {
  return s
    .replace(/&#x([0-9a-f]+);?/gi, (_, h) => String.fromCodePoint(Number.parseInt(h, 16)))
    .replace(/&#(\d+);?/g, (_, d) => String.fromCodePoint(Number.parseInt(d, 10)))
    .replace(/&colon;/gi, ":")
    .replace(/&tab;/gi, "\t")
    .replace(/&newline;/gi, "\n")
    .replace(/&amp;/gi, "&");
}

function safeHref(raw: string): string | null {
  // Strip control characters and whitespace that browsers ignore inside schemes ("java\nscript:").
  // biome-ignore lint/suspicious/noControlCharactersInRegex: intentional control-character strip
  const cleaned = decodeEntities(raw).replace(/[\u0000- \u007f-\u009f]/g, "");
  if (!SAFE_URL.test(cleaned)) return null;
  return raw.trim();
}

const ATTR_RE = /([^\s"'>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;

function sanitizeAttrs(tag: string, attrText: string): string {
  if (tag !== "a") return "";
  let href: string | null = null;
  for (const m of attrText.matchAll(ATTR_RE)) {
    const name = m[1]!.toLowerCase();
    const value = m[2] ?? m[3] ?? m[4] ?? "";
    if (name === "href") href = safeHref(value);
  }
  return href ? ` href="${escapeAttr(href)}" target="_blank" rel="noopener noreferrer"` : "";
}

const TOKEN_RE =
  /<!--[\s\S]*?(?:-->|$)|<\/?([a-zA-Z][a-zA-Z0-9-]*)((?:\s+[^>]*?)?)\s*(\/?)>|<![^>]*>|<\?[^>]*>/g;

export function sanitizeUserHtml(input: string): string {
  let out = "";
  let last = 0;
  let dropDepth = 0;
  let dropTag = "";
  const open: string[] = [];
  for (const m of input.matchAll(TOKEN_RE)) {
    const idx = m.index ?? 0;
    if (dropDepth === 0) out += escapeText(input.slice(last, idx));
    last = idx + m[0].length;
    const rawTag = m[1];
    if (!rawTag) continue; // comments, doctype, processing instructions
    const tag = rawTag.toLowerCase();
    const closing = m[0].startsWith("</");
    if (dropDepth > 0) {
      if (tag === dropTag) dropDepth += closing ? -1 : 1;
      continue;
    }
    if (DROP_WITH_CONTENT.has(tag)) {
      if (!closing && m[3] !== "/") {
        dropDepth = 1;
        dropTag = tag;
      }
      continue;
    }
    if (!ALLOWED_TAGS.has(tag)) continue;
    if (closing) {
      const at = open.lastIndexOf(tag);
      if (at >= 0) {
        while (open.length > at) out += `</${open.pop()}>`;
      }
      continue;
    }
    out += `<${tag}${sanitizeAttrs(tag, m[2] ?? "")}>`;
    if (!VOID_TAGS.has(tag)) open.push(tag);
  }
  if (dropDepth === 0) out += escapeText(input.slice(last));
  while (open.length > 0) out += `</${open.pop()}>`;
  return out;
}

/** Plain text of user HTML, for search text. */
export function htmlToText(html: string): string {
  return decodeEntities(
    html
      .replace(/<br\s*\/?>/gi, "\n")
      .replace(/<\/(p|div|li|h[1-3])>/gi, "\n")
      .replace(/<[^>]*>/g, ""),
  )
    .replace(/[ \t]+/g, " ")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
