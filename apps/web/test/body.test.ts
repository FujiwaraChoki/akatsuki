// §14.3: what reaches the sandboxed frame. The worker's sanitizer is the boundary; these tests pin the
// frame contract (sandbox, CSP first, deferred images, proxy-only remote images) and the defence-in-depth pass.
import type { MessageBody } from "@akatsuki/wire";
import { describe, expect, it } from "vitest";
import { memoryStore } from "../src/lib/kv.ts";
import type { CachedBody } from "../src/mail/bodyCache.ts";
import { BodyCache, BodyLoader } from "../src/mail/bodyCache.ts";
import {
  bodyCsp,
  buildSrcdoc,
  IFRAME_SANDBOX,
  isPlainLooking,
  isProxyUrl,
  measurementHtml,
  prepareHtml,
} from "../src/mail/bodyHtml.ts";

const SYNC = "https://sync.akatsuki.vc";
const PROXY = `${SYNC}/v1/img/abcd/sig_123/aHR0cHM6Ly9leGFtcGxlLmNvbS9hLnBuZw`;

describe("frame contract", () => {
  it("never allows scripts or same-origin", () => {
    expect(IFRAME_SANDBOX).toBe("allow-popups allow-popups-to-escape-sandbox");
    expect(IFRAME_SANDBOX).not.toMatch(/allow-scripts|allow-same-origin|allow-forms|allow-top-navigation/);
  });
  it("puts the CSP first", () => {
    const doc = buildSrcdoc("<p>Hi</p>", SYNC);
    expect(doc.indexOf('http-equiv="Content-Security-Policy"')).toBeLessThan(doc.indexOf("<style>"));
    expect(doc).toContain(
      `content="default-src 'none'; img-src data: ${SYNC}; style-src 'unsafe-inline'; font-src data:"`,
    );
    expect(bodyCsp(null)).toBe(
      "default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:",
    );
  });
  it("recognises only signed proxy URLs on the worker", () => {
    expect(isProxyUrl(PROXY, SYNC)).toBe(true);
    expect(isProxyUrl("https://evil.example/v1/img/a/b/c", SYNC)).toBe(false);
    expect(isProxyUrl(`${SYNC}/v1/messages/x`, SYNC)).toBe(false);
    expect(isProxyUrl(PROXY, null)).toBe(false);
  });
});

describe("prepareHtml", () => {
  it("defers remote images until asked, then loads only proxy URLs", () => {
    const html = `<p>x</p><img data-akatsuki-src="${PROXY}" alt="a"><img data-akatsuki-src="https://tracker.example/p.gif">`;
    const hidden = prepareHtml(html, { showImages: false, syncOrigin: SYNC });
    expect(hidden.blockedImages).toBe(2);
    expect(hidden.html).not.toContain(" src=");
    const shown = prepareHtml(html, { showImages: true, syncOrigin: SYNC });
    expect(shown.blockedImages).toBe(1);
    expect(shown.html).toContain(`src="${PROXY}"`);
    expect(shown.html).not.toMatch(/\ssrc="https:\/\/tracker/);
  });
  it("strips anything executable a broken sanitizer might let through", () => {
    const evil =
      '<script>alert(1)</script><img src="x" onerror="alert(1)"><a href="javascript:alert(1)">x</a>' +
      '<iframe src="https://e.example"></iframe><form action="/x"><input></form><div onclick="x()">y</div>' +
      '<a href=" jav&#x09;ascript:alert(1)">z</a><meta http-equiv="refresh" content="0;url=https://e.example">' +
      '<img src="https://e.example/pixel.gif"><svg><a href="data:text/html,<script>1</script>">s</a></svg>';
    const out = prepareHtml(evil, { showImages: true, syncOrigin: SYNC }).html.toLowerCase();
    expect(out).not.toMatch(/<script|<iframe|<form|<input|<meta|onerror|onclick|javascript:|data:text/);
    expect(out).not.toContain("https://e.example/pixel.gif");
  });
  it("opens links outside the frame without a referrer", () => {
    const out = prepareHtml('<a href="https://example.com">x</a>', {
      showImages: false,
      syncOrigin: SYNC,
    }).html;
    expect(out).toContain('target="_blank"');
    expect(out).toContain('rel="noopener noreferrer"');
  });
  it("injects fetched cid parts only as image data URLs", () => {
    const html = '<img data-akatsuki-cid="1.2"><img data-akatsuki-cid="1.3">';
    const first = prepareHtml(html, { showImages: false, syncOrigin: SYNC });
    expect(first.missingCids).toEqual(["1.2", "1.3"]);
    const cid = new Map([
      ["1.2", "data:image/png;base64,iVBORw0KGgo="],
      ["1.3", "data:text/html;base64,PHNjcmlwdD4="],
    ]);
    const out = prepareHtml(html, { showImages: false, syncOrigin: SYNC, cidImages: cid });
    expect(out.html).toContain('src="data:image/png;base64,iVBORw0KGgo="');
    expect(out.html).not.toContain("data:text/html");
    expect(out.missingCids).toEqual(["1.3"]);
  });
  it("blocked images become sized, labelled boxes, not broken-image glyphs (D-399)", () => {
    const out = prepareHtml(`<p>x</p><img alt="Logo" width="120" height="40" data-akatsuki-src="${PROXY}">`, {
      showImages: false,
      syncOrigin: SYNC,
    });
    expect(out.blockedImages).toBe(1);
    expect(out.html).not.toContain("<img");
    expect(out.html).toContain('class="ak-img-ph"');
    expect(out.html).toContain('aria-label="Logo"');
    expect(out.html).toContain("width:120px;height:40px");
    const shown = prepareHtml(`<img alt="Logo" data-akatsuki-src="${PROXY}">`, {
      showImages: true,
      syncOrigin: SYNC,
    });
    expect(shown.html).toContain(`src="${PROXY}"`);
  });
  it("only messages without their own colours render dark (D-399)", () => {
    expect(isPlainLooking("<p>Hello</p><blockquote>quoted</blockquote>")).toBe(true);
    expect(isPlainLooking('<p style="color:#333">Hello</p>')).toBe(false);
    expect(isPlainLooking('<table bgcolor="#fff"><tr><td>x</td></tr></table>')).toBe(false);
    expect(isPlainLooking("<style>.a{background:#fafafa}</style><p class=a>x</p>")).toBe(false);
    expect(buildSrcdoc("<p>x</p>", SYNC, "dark")).toContain("color-scheme:dark");
    expect(buildSrcdoc("<p>x</p>", SYNC)).toContain("color-scheme:light");
  });
  it("measures without remote fetches", () => {
    const out = measurementHtml(
      '<div style="background:url(https://e.example/x.png)">a</div><a href="https://e">b</a>',
    );
    expect(out).not.toContain("e.example");
    expect(out).not.toContain("href");
  });
});

const body = (id: string, v = 1): MessageBody => ({
  messageId: id as never,
  providerMessageId: `p-${id}`,
  sanitizerVersion: v,
  html: "<p>hello</p>",
  text: "hello",
  quoted: { htmlStart: null, textStart: null },
  attachments: [],
  remoteImageCount: 0,
  trackersRemoved: 0,
  headers: { to: [], cc: [], replyTo: [], listUnsubscribe: null, auth: null },
});

describe("body cache and loader", () => {
  it("caches, persists, and drops bodies from an older sanitizer", async () => {
    const store = memoryStore<CachedBody>();
    const cache = new BodyCache({ store });
    await cache.put(body("m1", 1));
    expect((await store.get("m1"))?.body.messageId).toBe("m1");
    await cache.put(body("m2", 2));
    expect(cache.peek("m1" as never)).toBeUndefined();
    const fresh = new BodyCache({ store });
    expect(await fresh.get("m1" as never)).toBeUndefined();
    expect((await fresh.get("m2" as never))?.sanitizerVersion).toBe(2);
  });
  it("keeps bodies in memory only when cacheBodiesOnDevice is off", async () => {
    const store = memoryStore<CachedBody>();
    const cache = new BodyCache({ store, persist: false });
    await cache.put(body("m1"));
    expect(cache.peek("m1" as never)).toBeDefined();
    expect(await store.entries()).toEqual([]);
  });
  it("evicts least recently used bodies over the cap", async () => {
    const store = memoryStore<CachedBody>();
    const cache = new BodyCache({ store, capBytes: 1000 }); // each test body counts ~546 bytes
    for (const id of ["a", "b", "c"]) await cache.put(body(id));
    await cache.evict();
    expect((await store.entries()).map(([k]) => k)).toEqual(["c"]);
  });
  it("streams a thread once even when asked twice", async () => {
    let calls = 0;
    const worker = {
      async *threadBodies() {
        calls++;
        yield body("m1");
        yield body("m2");
      },
    };
    const loader = new BodyLoader(worker as never, new BodyCache({ store: memoryStore() }));
    await Promise.all([
      loader.loadThread("t1" as never, ["m1", "m2"] as never),
      loader.loadThread("t1" as never, ["m1"] as never),
    ]);
    expect(calls).toBe(1);
    expect(loader.cache.peek("m2" as never)?.html).toBe("<p>hello</p>");
    await loader.loadThread("t1" as never, ["m1", "m2"] as never);
    expect(calls).toBe(1);
  });
  it("remembers a failure instead of retrying in a loop", async () => {
    let calls = 0;
    const worker = {
      // biome-ignore lint/correctness/useYield: fails before yielding
      async *threadBodies() {
        calls++;
        throw Object.assign(new Error("x"), { code: "not_implemented" });
      },
    };
    const loader = new BodyLoader(worker as never, new BodyCache({ store: memoryStore() }));
    await loader.loadThread("t1" as never, ["m1"] as never);
    await loader.loadThread("t1" as never, ["m1"] as never);
    expect(calls).toBe(1);
    expect(loader.errorFor("t1" as never)).toBeDefined();
    await loader.loadThread("t1" as never, ["m1"] as never, { force: true });
    expect(calls).toBe(2);
  });
});

describe("open original (web, D-399)", () => {
  it("wraps the body in the same sandboxed frame, with a strict CSP and escaped headers", async () => {
    const { originalPage } = await import("../src/mail/original.ts");
    const html = originalPage({
      subject: "<Quarterly> numbers",
      meta: { from: { name: "Ada", email: "ada@example.com" } } as never,
      body: {
        ...body("m1"),
        html: `<p>Hi</p><img data-akatsuki-src="${PROXY}"><img data-akatsuki-src="https://tracker.example/p.gif">`,
      },
      date: "20 October 2026",
      showImages: true,
      syncOrigin: SYNC,
    });
    expect(html).toContain(`sandbox="${IFRAME_SANDBOX}"`);
    expect(html).toContain(`img-src data: ${SYNC}`);
    expect(html).toContain("&lt;Quarterly&gt; numbers");
    expect(html).not.toContain("<Quarterly>");
    // Mail markup only appears escaped inside srcdoc, never as live markup of the wrapper.
    expect(html).not.toContain("<p>Hi</p>");
    expect(html).toContain("&lt;p&gt;Hi&lt;/p&gt;");
    expect(html).not.toContain("tracker.example/p.gif&quot; src");
    expect(html).toContain(`src=&quot;${PROXY}&quot;`);
  });
});
