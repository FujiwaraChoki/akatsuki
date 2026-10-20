// Markdown from the repository's own documents (docs/PRIVACY.md, SECURITY.md, LICENSE), read at
// build time and rendered to HTML, split into numbered sections for the document layout.
import { Marked, type Tokens } from "marked";
import { readRepoFile } from "./repo";

const LINK_MAP: Record<string, string> = {
  "SECURITY.md": "/security",
  "PRIVACY.md": "/privacy",
  LICENSE: "/license",
  "CODE_OF_CONDUCT.md": "/license",
};

/** The repository files the site renders. Edit those files, not the pages. */
export const SOURCES = { privacy: "docs/PRIVACY.md", security: "SECURITY.md", licence: "LICENSE" } as const;

export function readContent(which: keyof typeof SOURCES): string {
  return readRepoFile(SOURCES[which]);
}

export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/&[a-z]+;/g, "")
    .replace(/[^\p{L}\p{N}\s-]/gu, "")
    .trim()
    .replace(/\s+/g, "-");
}

export function rewriteHref(href: string): string {
  if (/^(https?:|mailto:|#)/.test(href)) return href;
  const [p = "", hash] = href.split("#");
  const mapped = LINK_MAP[p.split("/").pop() ?? ""];
  if (mapped) return hash ? `${mapped}#${hash}` : mapped;
  return href;
}

const escAttr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");

/** Serif headings: tighten word-internal hyphens (see `.hy`) in text, never inside tags. */
export const hyphens = (html: string) =>
  html.replace(
    /(^|>)([^<]*)/g,
    (_m, a: string, text: string) =>
      a + text.replace(/([A-Za-z])-([A-Za-z])/g, '$1<span class="hy">-</span>$2'),
  );

/** Serif headlines carry no trailing period (only the home hero and the finale do). */
export const period = (t: string) => t.trim().replace(/\.$/, "");

export interface DocSection {
  id: string;
  title: string;
  num: string;
  html: string;
}
export interface Doc {
  title: string | null;
  intro: string;
  sections: DocSection[];
  words: number;
}

export function renderDoc(source: string): Doc {
  let title: string | null = null;
  let body = source;
  const h1 = /^#\s+(.+)\n/m.exec(source);
  if (h1?.[1] && source.slice(0, h1.index).trim() === "") {
    title = h1[1].trim();
    body = source.slice(h1.index + h1[0].length);
  }
  const words = body.split(/\s+/).filter(Boolean).length;
  const seen = new Map<string, number>();
  const marked = new Marked({ gfm: true, async: false });
  let n = 0;
  marked.use({
    renderer: {
      heading(this: { parser: { parseInline: (t: Tokens.Generic[]) => string } }, token: Tokens.Heading) {
        const inner = this.parser.parseInline(token.tokens);
        const base = slugify(token.text) || "section";
        const k = seen.get(base) ?? 0;
        seen.set(base, k + 1);
        const id = k === 0 ? base : `${base}-${k}`;
        if (token.depth <= 2) {
          n++;
          const num = String(n).padStart(2, "0");
          return `<!--SECTION ${id}|${escAttr(token.text)}|${num}--><h2 id="${id}"><span class="hn">${num}</span>${hyphens(period(inner))}</h2>\n`;
        }
        return `<h3 id="${id}">${inner}</h3>\n`;
      },
      link(this: { parser: { parseInline: (t: Tokens.Generic[]) => string } }, token: Tokens.Link) {
        const inner = this.parser.parseInline(token.tokens);
        const href = rewriteHref(token.href);
        const ext = /^https?:/.test(href) ? ' rel="noopener noreferrer"' : "";
        return `<a href="${escAttr(href)}"${ext}>${inner}</a>`;
      },
    },
  });
  const html = (marked.parse(body) as string)
    .replace(/<table>/g, '<div class="table-wrap"><table>')
    .replace(/<\/table>/g, "</table></div>");
  const parts = html.split(/(?=<!--SECTION )/);
  const intro = parts[0]?.startsWith("<!--SECTION") ? "" : (parts.shift() ?? "");
  const sections = parts.map((p) => {
    const m = /^<!--SECTION ([^|]+)\|([^|]*)\|([^>]+)-->/.exec(p);
    return {
      id: m?.[1] ?? "",
      title: (m?.[2] ?? "").replace(/&quot;/g, '"').replace(/&amp;/g, "&"),
      num: m?.[3] ?? "",
      html: p.replace(/^<!--SECTION[^>]*-->/, ""),
    };
  });
  return { title, intro, sections, words };
}

export const readMinutes = (words: number) => Math.max(1, Math.round(words / 230));
