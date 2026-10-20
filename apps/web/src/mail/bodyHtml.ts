// Rendering sanitized message HTML (ARCHITECTURE §14.3). The worker's `crates/mime::sanitize` is the one
// sanitizer; this module only prepares its output for the sandboxed frame:
//
// - `<iframe sandbox="allow-popups allow-popups-to-escape-sandbox" srcdoc=…>`: never `allow-scripts`, never
//   `allow-same-origin`, so the frame has an opaque origin and no script ever runs in it;
// - a meta CSP as the first thing in `<head>`: `default-src 'none'; img-src data: <sync origin>;
//   style-src 'unsafe-inline'; font-src data:`;
// - remote images arrive as `data-akatsuki-src="<proxy URL>"` with no `src`; they are turned into `src` only
//   when the user shows images, and only for URLs on the sync worker's `/v1/img/` proxy;
// - `cid:` images arrive as `data-akatsuki-cid="<partId>"`; the host fetches the part with the bearer token
//   and injects it as a `data:image/…` URL.
//
// A second, cheap pass drops anything executable that should never be there (defence in depth: it does not
// replace the worker's sanitizer, it only makes a sanitizer regression harmless here).
import { color, font } from "@akatsuki/tokens";

export const IFRAME_SANDBOX = "allow-popups allow-popups-to-escape-sandbox";

/**
 * The frame's CSP allows `font-src data:` only, so the app's Geist files cannot load there. The frame and the
 * off-frame measurement both use the system part of the tokens' sans stack, so they lay out identically.
 */
export const FRAME_FONTS = font.sans.stack
  .split(",")
  .slice(1)
  .map((s) => s.trim())
  .join(", ");

const DANGEROUS_TAGS =
  "script,iframe,frame,frameset,object,embed,applet,form,input,button,select,textarea,meta,base,link,noscript,template,portal";
const URL_ATTRS = [
  "href",
  "src",
  "srcset",
  "action",
  "formaction",
  "xlink:href",
  "background",
  "poster",
  "cite",
];
const SAFE_DATA_IMAGE = /^data:image\/(png|gif|jpe?g|webp);base64,[a-z0-9+/=\s]*$/i;

/** The frame's CSP (§14.3). `syncOrigin` is the worker origin (https://sync.akatsuki.vc in production). */
export function bodyCsp(syncOrigin: string | null): string {
  const img = syncOrigin ? `data: ${syncOrigin}` : "data:";
  return `default-src 'none'; img-src ${img}; style-src 'unsafe-inline'; font-src data:`;
}

/** Whether `url` is an image-proxy URL signed by our worker (`/v1/img/{u}/{sig}/{b64url}`, §14.4). */
export function isProxyUrl(url: string, syncOrigin: string | null): boolean {
  if (!syncOrigin) return false;
  try {
    const u = new URL(url);
    return u.origin === syncOrigin && /^\/v1\/img\/[^/]+\/[^/]+\/[^/]+$/.test(u.pathname);
  } catch {
    return false;
  }
}

export interface PrepareOptions {
  showImages: boolean;
  syncOrigin: string | null;
  /** partId → `data:image/…` URL for inline (cid) images fetched by the host. */
  cidImages?: ReadonlyMap<string, string>;
}

export interface Prepared {
  html: string;
  /** Remote images left blocked (shown as the "Show images" banner). */
  blockedImages: number;
  /** cid parts referenced but not yet injected (the host should fetch these). */
  missingCids: string[];
}

function isSafeUrl(v: string): boolean {
  const s = v
    .trim()
    .replace(/[\p{Cc}\s]+/gu, "")
    .toLowerCase();
  if (s.startsWith("javascript:") || s.startsWith("vbscript:")) return false;
  if (s.startsWith("data:")) return SAFE_DATA_IMAGE.test(v.trim());
  return true;
}

/** A blocked remote image as a sized, labelled box (`.ak-img-ph`), so no broken-image glyph shows. */
function placeholderFor(img: HTMLImageElement, doc: Document): HTMLElement {
  const box = doc.createElement("span");
  box.className = "ak-img-ph";
  box.setAttribute("data-akatsuki-blocked", "");
  box.setAttribute("role", "img");
  box.setAttribute("aria-label", img.getAttribute("alt")?.trim() || "Image not loaded");
  const px = (v: string | null) => {
    const n = v && /^\d{1,4}$/.test(v.trim()) ? Number(v.trim()) : null;
    return n !== null && n > 0 ? n : null;
  };
  const w = px(img.getAttribute("width"));
  const h = px(img.getAttribute("height"));
  const style = [w ? `width:${w}px` : "", h ? `height:${h}px` : ""].filter(Boolean).join(";");
  if (style) box.setAttribute("style", style);
  return box;
}

/** Prepares sanitized HTML for the frame. Pure apart from DOMParser (inert: nothing loads or runs). */
export function prepareHtml(html: string, opts: PrepareOptions): Prepared {
  const doc = new DOMParser().parseFromString(`<!doctype html><body>${html}</body>`, "text/html");
  for (const el of doc.querySelectorAll(DANGEROUS_TAGS)) el.remove();
  for (const el of doc.body.querySelectorAll("*")) {
    for (const attr of [...el.attributes]) {
      const name = attr.name.toLowerCase();
      if (name.startsWith("on") || name === "srcdoc" || name === "formaction") el.removeAttribute(attr.name);
      else if (URL_ATTRS.includes(name) && !isSafeUrl(attr.value)) el.removeAttribute(attr.name);
    }
  }
  for (const a of doc.querySelectorAll("a[href]")) {
    a.setAttribute("target", "_blank");
    a.setAttribute("rel", "noopener noreferrer");
  }
  let blockedImages = 0;
  const missingCids: string[] = [];
  for (const img of doc.querySelectorAll("img")) {
    img.removeAttribute("srcset");
    const src = img.getAttribute("src");
    // The sanitizer only leaves small data: images in `src`; anything else is not ours to load.
    if (src !== null && !SAFE_DATA_IMAGE.test(src)) img.removeAttribute("src");
    const remote = img.getAttribute("data-akatsuki-src");
    if (remote !== null) {
      if (opts.showImages && (isProxyUrl(remote, opts.syncOrigin) || SAFE_DATA_IMAGE.test(remote)))
        img.setAttribute("src", remote);
      else {
        blockedImages++;
        img.replaceWith(placeholderFor(img, doc));
        continue;
      }
    }
    const cid = img.getAttribute("data-akatsuki-cid");
    if (cid !== null) {
      const url = opts.cidImages?.get(cid);
      if (url && SAFE_DATA_IMAGE.test(url)) img.setAttribute("src", url);
      else if (!missingCids.includes(cid)) missingCids.push(cid);
    }
  }
  return { html: doc.body.innerHTML, blockedImages, missingCids };
}

/**
 * The frame's base styles. Mail HTML is laid out on the light "paper" palette in both themes (D-266), because
 * most senders assume dark text on white. `root` is the body selector (a class when measuring off-frame).
 */
export function frameCss(root = "body", mode: "light" | "dark" = "light"): string {
  const p = color[mode];
  const ph = `rgb(${p.rgb.foreground.join(" ")} / 0.25)`;
  return [
    root === "body" ? `html{color-scheme:${mode}}` : "",
    `${root}{margin:0;padding:16px 20px;background:${p.background};color:${p.foreground};font-family:${FRAME_FONTS};font-size:14px;line-height:1.55;overflow-wrap:anywhere;word-break:break-word}`,
    // Alt text of an unloaded image must not inherit the body's break-anywhere wrapping (it would stack one
    // character per line inside the image box and inflate the line).
    `${root} img{max-width:100%;height:auto;word-break:normal;overflow-wrap:normal}`,
    `${root} table{max-width:100%}`,
    `${root} a{color:${p.accentText}}`,
    // A remote image that is not loaded: a sized, quiet box instead of the browser's broken-image glyph.
    `${root} .ak-img-ph{display:inline-block;box-sizing:border-box;max-width:100%;min-width:16px;min-height:16px;vertical-align:middle;border:1px dashed ${ph};border-radius:4px}`,
    `${root} blockquote{margin:0 0 0 8px;padding-left:12px;border-left:2px solid currentColor;opacity:.75}`,
    `${root} pre{white-space:pre-wrap}`,
  ].join("");
}

/** The full `srcdoc` document. The CSP meta comes first so it governs everything after it. */
export function buildSrcdoc(
  innerHtml: string,
  syncOrigin: string | null,
  mode: "light" | "dark" = "light",
): string {
  return (
    "<!doctype html><html><head>" +
    `<meta http-equiv="Content-Security-Policy" content="${bodyCsp(syncOrigin)}">` +
    '<meta charset="utf-8"><meta name="referrer" content="no-referrer">' +
    '<base target="_blank">' +
    `<style>${frameCss("body", mode)}</style>` +
    `</head><body>${innerHtml}</body></html>`
  );
}

const COLOUR_DECL = /(^|[;{\s])(color|background(-color|-image)?|border(-[a-z]+)?-color)\s*:/i;

/**
 * Whether prepared HTML sets no colours of its own (no colour or background styles or attributes, no style
 * sheet that does): such a message reads the same on the dark palette, so dark mode renders it dark (D-399).
 * Anything a sender coloured keeps the light "paper" (D-266), because its colours assume white.
 */
export function isPlainLooking(preparedHtml: string): boolean {
  const doc = new DOMParser().parseFromString(`<!doctype html><body>${preparedHtml}</body>`, "text/html");
  for (const st of doc.querySelectorAll("style")) if (COLOUR_DECL.test(st.textContent ?? "")) return false;
  if (doc.querySelector("[bgcolor], [background], font[color], [text], [link], [vlink]")) return false;
  for (const el of doc.querySelectorAll("[style]"))
    if (COLOUR_DECL.test(el.getAttribute("style") ?? "")) return false;
  return true;
}

/** `visibility` changes nothing about height; dropped so no part of a measured copy can ask to be shown. */
const VISIBILITY_DECL = /(^|[;{\s])visibility\s*:[^;}]*;?/gi;

/**
 * A copy of prepared HTML for measuring its height outside the frame: no remote fetches at all, and no
 * `visibility` declarations (the measuring host is also transparent and paint-contained, D-267, D-399).
 */
export function measurementHtml(preparedHtml: string): string {
  const doc = new DOMParser().parseFromString(`<!doctype html><body>${preparedHtml}</body>`, "text/html");
  for (const el of doc.querySelectorAll("[style]")) {
    const s = el.getAttribute("style") ?? "";
    el.setAttribute("style", s.replace(/url\s*\([^)]*\)/gi, "none").replace(VISIBILITY_DECL, "$1"));
  }
  for (const st of doc.querySelectorAll("style"))
    st.textContent = (st.textContent ?? "")
      .replace(/url\s*\([^)]*\)/gi, "none")
      .replace(VISIBILITY_DECL, "$1");
  for (const a of doc.querySelectorAll("a")) a.removeAttribute("href");
  return doc.body.innerHTML;
}

/**
 * Styles of the measuring host, set through the CSSOM so they hold even without the stylesheet. Nothing in it
 * can paint: it is transparent (descendants cannot undo `opacity`), clips to a zero-height paint-contained box
 * (`contain: strict` also makes it the containing block of any fixed or absolute descendant), and sits far off
 * screen behind everything (D-399).
 */
export const MEASURE_HOST_STYLE: Readonly<Record<string, string>> = {
  position: "absolute",
  top: "0",
  left: "-100000px",
  height: "0",
  overflow: "hidden",
  contain: "strict",
  "clip-path": "inset(50%)",
  opacity: "0",
  visibility: "hidden",
  "pointer-events": "none",
  "z-index": "-1",
};

/** Plain-text bodies render in the app itself (escaped by React); this splits them into paragraphs. */
export function textParagraphs(text: string): string[] {
  return text.replace(/\r\n?/g, "\n").split(/\n{2,}/);
}
