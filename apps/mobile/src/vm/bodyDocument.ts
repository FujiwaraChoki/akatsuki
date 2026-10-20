// The HTML document a thread is shown in (ARCHITECTURE §14.3, mobile row): sanitized bodies from the worker
// (`MessageBody.html`, already passed through crates/mime::sanitize), rendered in react-native-webview with
// JavaScript disabled and the same CSP as web. Expanding messages and quoted text uses <details>, which works
// without script. Navigation is intercepted by `linkPolicy` and opened outside the web view.
import { splitQuoted } from "@akatsuki/wire";

/** The `MessageBody` fields the document uses (§7.3). */
export interface BodyLike {
  html: string | null;
  text: string | null;
  quoted: { htmlStart: number | null; textStart: number | null };
  attachments: {
    partId: string;
    filename: string;
    mimeType: string;
    size: number;
    inline: boolean;
    contentId?: string;
  }[];
  remoteImageCount: number;
  trackersRemoved: number;
}

export interface DocMessage {
  id: string;
  from: string;
  fromEmail: string;
  to: string;
  date: string;
  snippet: string;
  expanded: boolean;
  state: "loading" | "ready" | "error";
  body: BodyLike | null;
}

export interface DocTheme {
  background: string;
  foreground: string;
  muted: string;
  border: string;
  accent: string;
  /** Background behind sender-styled HTML (always light: email HTML assumes a white page). */
  paper: string;
  paperText: string;
  dark: boolean;
}

export interface DocOptions {
  theme: DocTheme;
  /** Origin of the sync worker, the only remote image host the CSP allows (`https://sync.akatsuki.vc`). */
  imageOrigin: string;
  /** Load proxied remote images (prefs.remoteImages = "proxy", the sender is allowed, or "Show images"). */
  showImages: boolean;
  /**
   * Inline `cid:` images already fetched by the app, as `data:image/...;base64,...` URLs, keyed by message id and
   * then part id (part ids are only unique within one message).
   */
  inlineImages?: Record<string, Record<string, string>>;
}

/** Scheme of the links the app intercepts to open an attachment. */
export const ATTACHMENT_SCHEME = "akatsuki-attachment";

export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

const unescapeAttr = (s: string) =>
  s
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&");

/** The CSP injected into every document (§14.3). */
export function contentSecurityPolicy(imageOrigin: string): string {
  const origin = safeOrigin(imageOrigin);
  return [
    "default-src 'none'",
    `img-src data:${origin ? ` ${origin}` : ""}`,
    "style-src 'unsafe-inline'",
    "font-src data:",
    "form-action 'none'",
    "base-uri 'none'",
  ].join("; ");
}

function safeOrigin(url: string): string | null {
  try {
    const u = new URL(url);
    if (u.protocol !== "https:" && u.protocol !== "http:") return null;
    return u.origin;
  } catch {
    return null;
  }
}

const DATA_IMAGE = /^data:image\/(png|gif|jpeg|webp);base64,[A-Za-z0-9+/=]+$/;

/**
 * Turns the sanitizer's deferred images into loadable ones. Remote images (`data-akatsuki-src`) load only when
 * `showImages` and only from the worker's image proxy; `cid:` images (`data-akatsuki-cid`) load when the app has
 * fetched them. Anything else stays without `src`.
 */
export function resolveImages(
  html: string,
  opts: Pick<DocOptions, "imageOrigin" | "showImages"> & { inline?: Record<string, string> },
): string {
  const origin = safeOrigin(opts.imageOrigin);
  let out = html;
  if (opts.showImages && origin) {
    out = out.replace(/\sdata-akatsuki-src="([^"]*)"/g, (whole, raw: string) => {
      const url = unescapeAttr(raw);
      return url.startsWith(`${origin}/v1/img/`) ? ` src="${escapeHtml(url)}"` : whole;
    });
  }
  const inline = opts.inline ?? {};
  out = out.replace(/\sdata-akatsuki-cid="([^"]*)"/g, (whole, raw: string) => {
    const dataUrl = inline[unescapeAttr(raw)];
    return dataUrl && DATA_IMAGE.test(dataUrl) ? ` src="${dataUrl}"` : whole;
  });
  return out;
}

const URL_RE = /\bhttps?:\/\/[^\s<>"']+[^\s<>"'.,;:!?)\]]/g;

/** Escapes plain text and turns http(s) URLs into links. */
export function textToHtml(text: string): string {
  let out = "";
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    const i = m.index ?? 0;
    out += escapeHtml(text.slice(last, i));
    const url = m[0];
    out += `<a href="${escapeHtml(url)}">${escapeHtml(url)}</a>`;
    last = i + url.length;
  }
  return out + escapeHtml(text.slice(last));
}

function quotedBlock(inner: string): string {
  return inner.trim().length === 0
    ? ""
    : `<details class="quoted"><summary aria-label="Show quoted text">&#8226;&#8226;&#8226;</summary>${inner}</details>`;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** The sanitizer's wrapper class (`crates/mime::css::BODY_CLASS`); every sender rule is scoped under it. */
const BODY_CLASS = "akatsuki-body";
const SCOPED_RULE = new RegExp(`\\.${BODY_CLASS}(?![\\w-])`, "g");

/**
 * Gives one message's sanitized HTML its own scope. Every message of a thread shares one document here (web uses
 * one frame per message), and the sanitizer scopes sender CSS only to `.akatsuki-body`, which every message's
 * wrapper carries: without this, one sender's `<style>` would restyle (or hide) the other messages. Each rule is
 * narrowed to `.akatsuki-body.<scope>` and each wrapper gets the scope class (D-408).
 */
export function scopeMessageStyles(html: string, scope: string): string {
  return html
    .replace(
      /(<style\b[^>]*>)([\s\S]*?)(<\/style>)/gi,
      (_, open: string, css: string, close: string) =>
        `${open}${css.replace(SCOPED_RULE, `.${BODY_CLASS}.${scope}`)}${close}`,
    )
    .replace(/(\sclass=")akatsuki-body(")/g, `$1${BODY_CLASS} ${scope}$2`);
}

/** The body of one message (HTML on paper, or themed plain text), with its quoted part collapsed. */
export function messageBodyHtml(
  messageId: string,
  body: BodyLike,
  opts: DocOptions,
  scope = "ak-m0",
): string {
  let content: string;
  if (body.html !== null) {
    const { visible, quoted } = splitQuoted(body.html, body.quoted.htmlStart);
    const img = { ...opts, inline: opts.inlineImages?.[messageId] };
    const html = (h: string) => scopeMessageStyles(resolveImages(h, img), scope);
    content = `<div class="paper">${html(visible)}${quotedBlock(html(quoted))}</div>`;
  } else {
    const text = body.text ?? "";
    const { visible, quoted } = splitQuoted(text, body.quoted.textStart);
    content = `<div class="text">${textToHtml(visible)}${quotedBlock(textToHtml(quoted))}</div>`;
  }
  const files = body.attachments.filter((a) => !a.inline);
  if (files.length > 0) {
    const items = files
      .map(
        (a) =>
          `<li><a class="file" href="${ATTACHMENT_SCHEME}://${encodeURIComponent(messageId)}/${encodeURIComponent(a.partId)}">${escapeHtml(a.filename || "attachment")}<span>${formatSize(a.size)}</span></a></li>`,
      )
      .join("");
    content += `<ul class="files" aria-label="Attachments">${items}</ul>`;
  }
  return content;
}

function styles(t: DocTheme): string {
  return `
:root{color-scheme:${t.dark ? "dark" : "light"}}
html,body{margin:0;padding:0;background:${t.background};color:${t.foreground};}
body{font:15px/1.6 -apple-system,"Geist",Roboto,"Segoe UI",system-ui,sans-serif;-webkit-text-size-adjust:100%;word-wrap:break-word;overflow-wrap:anywhere;padding:0 16px 48px}
a{color:${t.accent}}
details.msg{border-bottom:1px solid ${t.border};padding:12px 0}
details.msg>summary{list-style:none;cursor:pointer;display:block}
details.msg>summary::-webkit-details-marker{display:none}
.who{font-weight:600;font-size:14px}
.when{float:right;font-size:12px;color:${t.muted}}
.to{font-size:12px;color:${t.muted};margin-top:2px}
.snip{font-size:13px;color:${t.muted};white-space:nowrap;overflow:hidden;text-overflow:ellipsis;margin-top:2px}
details.msg[open] .snip{display:none}
details.msg:not([open]) .to{display:none}
.body{margin-top:12px}
.paper{background:${t.paper};color:${t.paperText};border-radius:12px;padding:${t.dark ? "12px" : "0"};overflow-x:auto}
.paper img{max-width:100%;height:auto}
.paper table{max-width:100%}
.text{white-space:pre-wrap;font-size:15px}
details.quoted>summary{display:inline-block;list-style:none;cursor:pointer;color:${t.muted};border:1px solid ${t.border};border-radius:6px;padding:0 8px;margin:8px 0;font-size:12px;letter-spacing:2px}
details.quoted>summary::-webkit-details-marker{display:none}
.state{color:${t.muted};font-size:13px;padding:8px 0}
ul.files{list-style:none;margin:12px 0 0;padding:0}
ul.files li{margin:6px 0}
a.file{display:block;text-decoration:none;color:${t.foreground};border:1px solid ${t.border};border-radius:8px;padding:10px 12px;font-size:13px}
a.file span{float:right;color:${t.muted}}
`;
}

/** The complete document for a thread: every message, oldest first, the expanded ones open. */
export function threadDocument(messages: readonly DocMessage[], opts: DocOptions): string {
  const parts = messages.map((m, i) => {
    let inner: string;
    if (m.state === "ready" && m.body) inner = messageBodyHtml(m.id, m.body, opts, `ak-m${i}`);
    else if (m.state === "error") inner = `<p class="state">This message could not be loaded.</p>`;
    else inner = `<p class="state">Loading…</p>`;
    return `<details class="msg"${m.expanded ? " open" : ""}><summary><span class="when">${escapeHtml(m.date)}</span><div class="who">${escapeHtml(m.from)}</div><div class="to">${escapeHtml(m.to)}</div><div class="snip">${escapeHtml(m.snippet)}</div></summary><div class="body">${inner}</div></details>`;
  });
  return `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${escapeHtml(contentSecurityPolicy(opts.imageOrigin))}"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="referrer" content="no-referrer"><style>${styles(opts.theme)}</style></head><body>${parts.join("")}</body></html>`;
}

export type LinkDecision =
  | { kind: "load" }
  | { kind: "external"; url: string }
  | { kind: "attachment"; messageId: string; partId: string }
  | { kind: "block" };

/**
 * What to do when the web view wants to navigate. Only the initial document loads inside the view; web and mail
 * links open outside it (system browser, mail composer); attachment links go to the app; everything else
 * (`javascript:`, `file:`, `intent:`, custom schemes) is blocked.
 */
export function linkPolicy(url: string, isInitialLoad: boolean): LinkDecision {
  if (isInitialLoad && (url === "about:blank" || url.startsWith("data:text/html") || url === ""))
    return { kind: "load" };
  if (url.startsWith("about:")) return { kind: "load" };
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return { kind: "block" };
  }
  const scheme = u.protocol.replace(/:$/, "").toLowerCase();
  if (scheme === "http" || scheme === "https" || scheme === "mailto" || scheme === "tel")
    return { kind: "external", url };
  if (scheme === ATTACHMENT_SCHEME) {
    const messageId = decodeURIComponent(u.host || u.hostname);
    const partId = decodeURIComponent(u.pathname.replace(/^\/+/, ""));
    if (messageId && partId) return { kind: "attachment", messageId, partId };
  }
  return { kind: "block" };
}

/** Which messages start expanded: every unread message and the newest one (Superhuman's default). */
export function initiallyExpanded(messages: readonly { id: string; isUnread: boolean }[]): Set<string> {
  const ids = new Set(messages.filter((m) => m.isUnread).map((m) => m.id));
  const last = messages[messages.length - 1];
  if (last) ids.add(last.id);
  return ids;
}
