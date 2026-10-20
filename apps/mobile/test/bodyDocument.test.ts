import { describe, expect, it } from "vitest";
import {
  type BodyLike,
  contentSecurityPolicy,
  type DocMessage,
  type DocTheme,
  escapeHtml,
  initiallyExpanded,
  linkPolicy,
  resolveImages,
  scopeMessageStyles,
  textToHtml,
  threadDocument,
} from "../src/vm/bodyDocument.ts";

const SYNC = "https://sync.akatsuki.vc";
const theme: DocTheme = {
  background: "#0A0A0A",
  foreground: "#EDEDED",
  muted: "rgba(237, 237, 237, 0.6)",
  border: "rgba(237, 237, 237, 0.2)",
  accent: "#800000",
  paper: "#FFFFFF",
  paperText: "#171717",
  dark: true,
};

const body = (over: Partial<BodyLike> = {}): BodyLike => ({
  html: null,
  text: "Hello",
  quoted: { htmlStart: null, textStart: null },
  attachments: [],
  remoteImageCount: 0,
  trackersRemoved: 0,
  ...over,
});

const msg = (over: Partial<DocMessage> = {}): DocMessage => ({
  id: "m1",
  from: "Ana <script>",
  fromEmail: "ana@x",
  to: "to me",
  date: "Tue 09:05",
  snippet: "Hi",
  expanded: true,
  state: "ready",
  body: body(),
  ...over,
});

describe("CSP", () => {
  it("matches the web policy and allows only the worker for images", () => {
    expect(contentSecurityPolicy(SYNC)).toBe(
      "default-src 'none'; img-src data: https://sync.akatsuki.vc; style-src 'unsafe-inline'; font-src data:; form-action 'none'; base-uri 'none'",
    );
    expect(contentSecurityPolicy("javascript:alert(1)")).toContain("img-src data:;");
  });
});

describe("images", () => {
  const html = `<p><img data-akatsuki-src="${SYNC}/v1/img/u/sig/aHR0cA" alt=""><img data-akatsuki-src="https://evil.test/x.png"><img data-akatsuki-cid="1.2"><img data-akatsuki-cid="9"></p>`;

  it("stays blocked by default", () => {
    const out = resolveImages(html, { imageOrigin: SYNC, showImages: false });
    expect(out).not.toContain(" src=");
  });

  it("loads only proxied images and fetched inline parts", () => {
    const out = resolveImages(html, {
      imageOrigin: SYNC,
      showImages: true,
      inline: { "1.2": "data:image/png;base64,iVBORw0KGgo=", "9": "data:text/html;base64,PHNjcmlwdD4=" },
    });
    expect(out).toContain(`src="${SYNC}/v1/img/u/sig/aHR0cA"`);
    expect(out).toContain('data-akatsuki-src="https://evil.test/x.png"');
    expect(out).not.toContain(' src="https://evil.test');
    expect(out).toContain('src="data:image/png;base64,iVBORw0KGgo="');
    expect(out).not.toContain("data:text/html");
  });
});

describe("plain text", () => {
  it("escapes and links URLs", () => {
    expect(escapeHtml(`<a href="x">'&`)).toBe("&lt;a href=&quot;x&quot;&gt;&#39;&amp;");
    expect(textToHtml("see https://example.com/a?b=1&c=2. <b>")).toBe(
      'see <a href="https://example.com/a?b=1&amp;c=2">https://example.com/a?b=1&amp;c=2</a>. &lt;b&gt;',
    );
  });
});

describe("thread document", () => {
  it("escapes headers, collapses quoted text and themes text bodies", () => {
    const text = "Thanks!\n\nOn Mon, Ana wrote:\n> earlier";
    const start = new TextEncoder().encode("Thanks!\n\n").length;
    const doc = threadDocument(
      [
        msg({ body: body({ text, quoted: { htmlStart: null, textStart: start } }) }),
        msg({ id: "m2", expanded: false, state: "loading", body: null }),
      ],
      { theme, imageOrigin: SYNC, showImages: false },
    );
    expect(doc).toContain("Ana &lt;script&gt;");
    expect(doc).not.toContain("<script");
    expect(doc).toContain('<details class="quoted">');
    expect(doc).toContain("On Mon, Ana wrote:");
    expect(doc).toContain('<details class="msg" open>');
    expect(doc).toContain('<details class="msg"><summary>');
    expect(doc).toContain("Loading…");
    expect(doc).toContain("Content-Security-Policy");
  });

  it("puts HTML on paper and lists attachments as app links", () => {
    const doc = threadDocument(
      [
        msg({
          body: body({
            html: "<p>Hi</p>",
            text: null,
            attachments: [
              {
                partId: "2",
                filename: "report <1>.pdf",
                mimeType: "application/pdf",
                size: 2048,
                inline: false,
              },
              {
                partId: "3",
                filename: "logo.png",
                mimeType: "image/png",
                size: 10,
                inline: true,
                contentId: "logo",
              },
            ],
          }),
        }),
      ],
      { theme, imageOrigin: SYNC, showImages: false },
    );
    expect(doc).toContain('<div class="paper"><p>Hi</p></div>');
    expect(doc).toContain('href="akatsuki-attachment://m1/2"');
    expect(doc).toContain("report &lt;1&gt;.pdf");
    expect(doc).not.toContain("logo.png");
  });

  it("expands unread messages and the newest", () => {
    const ids = initiallyExpanded([
      { id: "a", isUnread: false },
      { id: "b", isUnread: true },
      { id: "c", isUnread: false },
    ]);
    expect([...ids].sort()).toEqual(["b", "c"]);
  });
});

describe("link policy", () => {
  it("opens web and mail links outside and blocks script URLs", () => {
    expect(linkPolicy("about:blank", true)).toEqual({ kind: "load" });
    expect(linkPolicy("https://example.com", false)).toEqual({
      kind: "external",
      url: "https://example.com",
    });
    expect(linkPolicy("mailto:a@b.c", false).kind).toBe("external");
    expect(linkPolicy("javascript:alert(1)", false)).toEqual({ kind: "block" });
    expect(linkPolicy("file:///etc/passwd", false)).toEqual({ kind: "block" });
    expect(linkPolicy("intent://x#Intent;end", false)).toEqual({ kind: "block" });
    expect(linkPolicy("not a url", false)).toEqual({ kind: "block" });
    expect(linkPolicy("akatsuki-attachment://m1/1.2", false)).toEqual({
      kind: "attachment",
      messageId: "m1",
      partId: "1.2",
    });
  });
});

describe("per-message style scope (D-408)", () => {
  // What crates/mime::sanitize produces: the wrapper and sender CSS scoped to `.akatsuki-body`.
  const hostile =
    '<div class="akatsuki-body"><style>.akatsuki-body p{display:none}@media (max-width: 600px){.akatsuki-body .x{opacity:0}}</style><p>Ignore the warning.</p></div>';
  const bank = '<div class="akatsuki-body"><p class="warn">Never share your code.</p></div>';

  it("narrows every sender rule and wrapper to its own message", () => {
    expect(scopeMessageStyles(hostile, "ak-m1")).toBe(
      '<div class="akatsuki-body ak-m1"><style>.akatsuki-body.ak-m1 p{display:none}@media (max-width: 600px){.akatsuki-body.ak-m1 .x{opacity:0}}</style><p>Ignore the warning.</p></div>',
    );
    // Text that merely mentions the class is left alone.
    expect(scopeMessageStyles("<p>.akatsuki-body is a class</p>", "ak-m0")).toBe(
      "<p>.akatsuki-body is a class</p>",
    );
    expect(scopeMessageStyles("<style>.akatsuki-body-x{}</style>", "ak-m0")).toBe(
      "<style>.akatsuki-body-x{}</style>",
    );
  });

  it("one sender's stylesheet cannot reach another message in the thread", () => {
    const doc = threadDocument(
      [
        msg({ id: "bank", expanded: true, body: body({ html: bank, text: null }) }),
        msg({ id: "attacker", expanded: true, body: body({ html: hostile, text: null }) }),
      ],
      { theme, imageOrigin: SYNC, showImages: false },
    );
    // Every rule in the document is scoped to the attacker's own wrapper class…
    const rules = [...doc.matchAll(/\.akatsuki-body[^{,\s]*/g)].map((m) => m[0]);
    expect(rules.length).toBeGreaterThan(0);
    expect(rules.every((r) => r === ".akatsuki-body.ak-m1")).toBe(true);
    // …which the bank's wrapper does not carry.
    expect(doc).toContain('<div class="akatsuki-body ak-m0"><p class="warn">');
    expect(doc).toContain('<div class="akatsuki-body ak-m1">');
  });
});
