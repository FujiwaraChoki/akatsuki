// Copy, palette and security rules for the landing site, plus the facts the pages promise.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import tokens from "@akatsuki/tokens/tokens.json";
import { createElement, type ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import robots from "@/app/robots";
import sitemap from "@/app/sitemap";
import { ACTS } from "@/components/KeyMap";
import { STOPS } from "@/components/Timeline";
import { BOOT_SCRIPT } from "@/lib/boot";
import { readRepoFile } from "@/lib/repo";
import { SHORTCUT_COUNT, shortcut } from "@/lib/shortcuts";
import { mailto, PAGES, SITE } from "@/lib/site";

const APP = path.join(import.meta.dirname, "..");
const SRC = path.join(APP, "src");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = path.join(dir, name);
    return statSync(p).isDirectory() ? sourceFiles(p) : /\.(tsx?|css)$/.test(name) ? [p] : [];
  });
}

/** A page's rendered `<main>` and footer as plain text (the pages are synchronous server components). */
async function renderPage(href: string): Promise<{ main: string; footer: string }> {
  const file = href === "/" ? "../src/app/page.tsx" : `../src/app${href}/page.tsx`;
  const mod = (await import(/* @vite-ignore */ file)) as { default: () => ReactElement };
  const html = renderToStaticMarkup(createElement(mod.default));
  const text = (part: string) =>
    part
      .replace(/<(script|style)\b[\s\S]*?<\/\1>/g, " ")
      .replace(/<[^>]+>/g, "")
      .replace(/&amp;/g, "&")
      .replace(/&#x27;/g, "'")
      .replace(/&quot;/g, '"')
      .replace(/\s+/g, " ");
  return {
    main: text(/<main id="main">([\s\S]*)<\/main>/.exec(html)?.[1] ?? ""),
    footer: text(/<footer\b[^>]*>([\s\S]*)<\/footer>/.exec(html)?.[1] ?? ""),
  };
}

const PHRASE = "open source, free for personal use";
/** "Commercial use, including by any organisation, needs written permission." and its short forms. */
const CAVEAT = /commercial use[^.]*needs (written )?permission/i;

describe("copy", () => {
  it(`describes the licence one way: "${PHRASE}"`, () => {
    const offenders: string[] = [];
    for (const file of sourceFiles(SRC)) {
      const text = readFileSync(file, "utf8").replace(/\s+/g, " ");
      const at = (i: number) => `${path.relative(SRC, file)}: …${text.slice(Math.max(0, i - 40), i + 48)}`;
      for (const m of text.matchAll(/open[ -]source/gi)) {
        const rest = text.slice(m.index).toLowerCase();
        const around = text.slice(m.index - 1, m.index + m[0].length + 1);
        // Allowed: the phrase itself, the FAQ question ("Is Akatsuki open source?") and the bare
        // search keyword in the root layout's metadata.
        if (!rest.startsWith(PHRASE) && !rest.startsWith("open source?") && around !== '"open source"')
          offenders.push(at(m.index));
      }
      // "Source-available" survives only in the licence's name.
      for (const m of text.matchAll(/source[- ]available/gi))
        if (!text.slice(m.index).startsWith("Source-Available License")) offenders.push(at(m.index));
      for (const m of text.matchAll(/\bOSS\b|fair[ -]source/gi)) offenders.push(at(m.index));
    }
    expect(offenders).toEqual([]);
  });

  it("says commercial use needs permission on every page that names the licence", async () => {
    const naming: string[] = [];
    for (const { href } of PAGES) {
      const { main, footer } = await renderPage(href);
      // Every page ends in the same short line, which links to /license.
      expect(footer, href).toContain(
        "Open source, free for personal use · ASAL-1.0 · No trackers on this site",
      );
      if (!/open source|ASAL-1\.0/i.test(main)) continue;
      naming.push(href);
      expect(main, href).toMatch(CAVEAT);
      // The licence text on /license also names the Open Source Initiative.
      for (const m of main.matchAll(/open source(?! initiative)/gi)) {
        const rest = main.slice(m.index).toLowerCase();
        const ok = rest.startsWith(PHRASE) || rest.startsWith("open source?");
        expect(ok, `${href}: ${rest.slice(0, 60)}`).toBe(true);
      }
    }
    expect(naming).toEqual(expect.arrayContaining(["/pricing", "/license"]));
  });

  it("answers the FAQ with the same phrase and the caveat", () => {
    const pricing = readFileSync(path.join(SRC, "app/pricing/page.tsx"), "utf8").replace(/\s+/g, " ");
    const answer = /"Is Akatsuki open source\?", "([^"]+)"/.exec(pricing)?.[1] ?? "";
    expect(answer).toMatch(/^Yes, and it’s free for personal use\./);
    expect(answer).toMatch(CAVEAT);
  });

  it("uses the shortcut ids that exist in packages/shortcuts", () => {
    for (const [id] of Object.values(ACTS)) expect(() => shortcut(id)).not.toThrow();
    expect(SHORTCUT_COUNT).toBeGreaterThanOrEqual(50);
  });

  it("ends only the hero and the finale headlines in a period", () => {
    const home = readFileSync(path.join(SRC, "app/page.tsx"), "utf8");
    // Both periods are the red dot (`.hero-dot`), and there are exactly two.
    expect(home.match(/<span className="hero-dot">\.<\/span>/g)).toHaveLength(2);
    for (const s of STOPS) expect(s.what.endsWith(".")).toBe(false);
  });
});

describe("palette", () => {
  it("keeps colour literals out of the stylesheets, except the documented landing tokens", () => {
    const hex = /#[0-9a-f]{3}(?:[0-9a-f]{3})?\b/i;
    const offenders = sourceFiles(SRC).filter(
      (f) =>
        f.endsWith(".css") &&
        !f.endsWith("landing-tokens.css") &&
        hex.test(readFileSync(f, "utf8").replace(/\/\*[\s\S]*?\*\//g, "")),
    );
    expect(offenders).toEqual([]);
  });

  it("derives every landing-only token from the shared palette in packages/tokens", () => {
    const css = readFileSync(path.join(SRC, "app/landing-tokens.css"), "utf8");
    const vars = Object.fromEntries(
      [...css.matchAll(/--([\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]?.toLowerCase()]),
    );
    const c = tokens.color;
    // The red dot is the light-theme accent in both themes.
    expect(vars.dawn).toBe(c.light.accent.toLowerCase());
    expect(vars["on-accent"]).toBe(c.light.background.toLowerCase());
    expect(vars["night-bg"]).toBe(c.dark.background.toLowerCase());
    expect(vars["night-fg"]).toBe(c.dark.foreground.toLowerCase());
    expect(Object.keys(vars).sort()).toEqual([
      "dawn",
      "night-bg",
      "night-bg-lifted",
      "night-fg",
      "on-accent",
    ]);
  });
});

describe("vercel.json", () => {
  const vercel = JSON.parse(readFileSync(path.join(APP, "vercel.json"), "utf8")) as {
    headers: { source: string; headers: { key: string; value: string }[] }[];
  };
  const all = vercel.headers.find((h) => h.source === "/(.*)")?.headers ?? [];
  const csp = all.find((h) => h.key === "Content-Security-Policy")?.value ?? "";

  it("allows the theme boot script by hash and no other inline script", () => {
    const hash = createHash("sha256").update(BOOT_SCRIPT).digest("base64");
    const scriptSrc = /script-src ([^;]+)/.exec(csp)?.[1] ?? "";
    expect(scriptSrc.split(" ")).toEqual(["'self'", `'sha256-${hash}'`]);
  });

  it("keeps the site first-party only", () => {
    for (const d of [
      "default-src 'self'",
      "connect-src 'self'",
      "font-src 'self'",
      "frame-ancestors 'none'",
      "object-src 'none'",
    ]) {
      expect(csp).toContain(d);
    }
    expect(csp).not.toMatch(/https?:\/\//);
    expect(all.map((h) => h.key)).toEqual(
      expect.arrayContaining([
        "Strict-Transport-Security",
        "X-Content-Type-Options",
        "X-Frame-Options",
        "Referrer-Policy",
        "Permissions-Policy",
      ]),
    );
  });
});

describe("docs/PRIVACY.md (rendered at /privacy)", () => {
  const doc = readRepoFile("docs/PRIVACY.md");
  it.each([
    ["is marked as a draft for lawyer review", /Draft for lawyer review/],
    ["says bodies and attachments are never stored", /message bodies and attachments are never stored/i],
    ["says Convex stores headers and short snippets", /snippet of up to 200/i],
    ["names OpenAI for managed dictation with up to 30 days retention", /OpenAI[\s\S]{0,200}up to 30 days/],
    ["names Amazon SES", /Amazon SES/],
    ["names Stripe", /Stripe/],
    ["says there are no trackers", /No trackers/],
  ])("%s", (_name, re) => {
    expect(doc).toMatch(re);
  });

  it("refuses to read env files", () => {
    expect(() => readRepoFile(".env")).toThrow(/env file/);
    expect(() => readRepoFile("../outside.txt")).toThrow(/escapes/);
  });
});

describe("site", () => {
  it("lists every page in the sitemap, on the production origin", () => {
    const urls = sitemap().map((e) => e.url);
    expect(urls).toHaveLength(PAGES.length);
    for (const u of urls) expect(u.startsWith("https://akatsuki.vc")).toBe(true);
    expect(robots().sitemap).toBe("https://akatsuki.vc/sitemap.xml");
  });

  it("builds mailto links to the owner's address", () => {
    expect(mailto("Akatsuki waitlist", "Hi")).toBe(
      `mailto:${SITE.contact}?subject=Akatsuki%20waitlist&body=Hi`,
    );
  });
});
