// "Open original" on the web (§14.3, D-399): the newest message's sanitized body, full size, in a new tab, to
// read a complex layout or print. The tab is a small wrapper page (a blob: URL) whose only content is a
// sandboxed `srcdoc` frame, exactly like the reading pane's: mail markup never runs in, or gets the origin of,
// the app. Remote images follow the same rule as in the pane (the worker's proxy only, when shown).
import type { MessageBody, MessageMeta } from "../backend/types.ts";
import { displayName } from "../lib/format.ts";
import { buildSrcdoc, IFRAME_SANDBOX, prepareHtml } from "./bodyHtml.ts";

const escapeHtml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** The wrapper page for one message. Pure, so it can be tested without a browser. */
export function originalPage(opts: {
  subject: string;
  meta: MessageMeta;
  body: MessageBody;
  date: string;
  showImages: boolean;
  syncOrigin: string | null;
}): string {
  const { body } = opts;
  const inner =
    body.html !== null
      ? prepareHtml(body.html, { showImages: opts.showImages, syncOrigin: opts.syncOrigin }).html
      : `<pre style="white-space:pre-wrap;font:inherit">${escapeHtml(body.text ?? "")}</pre>`;
  const srcdoc = buildSrcdoc(inner, opts.syncOrigin);
  const img = opts.syncOrigin ? `data: ${opts.syncOrigin}` : "data:";
  const header = [
    `<h1>${escapeHtml(opts.subject || "(no subject)")}</h1>`,
    `<p>${escapeHtml(displayName(opts.meta.from))}${opts.meta.from?.email ? ` &lt;${escapeHtml(opts.meta.from.email)}&gt;` : ""} · ${escapeHtml(opts.date)}</p>`,
  ].join("");
  return (
    '<!doctype html><html><head><meta charset="utf-8">' +
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src ${img}; frame-src 'self'; form-action 'none'; base-uri 'none'">` +
    '<meta name="referrer" content="no-referrer">' +
    `<title>${escapeHtml(opts.subject || "Message")}</title>` +
    "<style>html,body{margin:0;height:100%}body{display:flex;flex-direction:column;font:14px/1.5 system-ui,sans-serif}" +
    "header{padding:16px 20px;border-bottom:1px solid #ddd}h1{margin:0 0 4px;font-size:18px;font-weight:600}p{margin:0;color:#555}" +
    "iframe{flex:1;width:100%;border:0}@media print{header{border:0}}</style>" +
    `</head><body><header>${header}</header>` +
    `<iframe title="Message" sandbox="${IFRAME_SANDBOX}" referrerpolicy="no-referrer" srcdoc="${escapeHtml(srcdoc)}"></iframe>` +
    "</body></html>"
  );
}

/** Opens the page in a new tab; the blob URL is revoked once the tab has had time to load it. */
export function openOriginalTab(html: string): void {
  const url = URL.createObjectURL(new Blob([html], { type: "text/html" }));
  window.open(url, "_blank", "noopener,noreferrer");
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}
