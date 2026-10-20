// @vitest-environment node
// app.css uses only @akatsuki/tokens variables and the --ak-* roles of @akatsuki/ui (BRIEF: packages/tokens is
// the single source of the palette). No colour literals anywhere in the stylesheet.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

const strip = (s: string) => s.replace(/\/\*[\s\S]*?\*\//g, "");
const css = strip(
  ["app.css", "features.css"]
    .map((f) => readFileSync(new URL(`../src/styles/${f}`, import.meta.url), "utf8"))
    .join("\n"),
);
const req = createRequire(import.meta.url);
const tokensCss = readFileSync(req.resolve("@akatsuki/tokens/tokens.css"), "utf8");
const uiCss = readFileSync(req.resolve("@akatsuki/ui/styles.css"), "utf8");
const defined = (src: string) => new Set([...src.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]!));

describe("app.css and features.css", () => {
  it("has no colour literals", () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(css).not.toMatch(/\b(rgb|rgba|hsl|hsla|oklch|lab)\((?!var\()/);
    // color-mix only over token colours (the horizon's dawn fading to transparent, First Light D-480).
    for (const m of css.matchAll(/\bcolor-mix\(/g))
      expect(
        css
          .slice(m.index + 10, m.index + 60)
          .replace(/\s+/g, " ")
          .trim(),
      ).toMatch(/^in (srgb|oklab), var\(--[a-z-]+\)/);
    expect(css).not.toMatch(/:\s*(white|black|red|gray|grey)\b/i);
  });
  it("has no decoration outside the horizon: no blur, no orbs, no accent tints (First Light, D-480)", () => {
    expect(css).not.toMatch(/backdrop-filter|blur\(|\.ak-orb|--glow\b|--accent-(tint|wash|glow|mid)/);
    const radial = [...css.matchAll(/([^{}]+)\{[^}]*radial-gradient/g)].map((m) => m[1]!.trim());
    for (const sel of radial) expect(sel).toMatch(/ak-horizon__band|ak-zero__band/);
    // The accent fill is the brand's: only `::selection` and the three brand buttons use it (they live in ui).
    expect(css).not.toMatch(/var\(--accent\)|--accent-solid/);
  });
  it("uses only defined variables", () => {
    const known = new Set([...defined(tokensCss), ...defined(uiCss), ...defined(css)]);
    const used = [...css.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1]!);
    expect(used.length).toBeGreaterThan(50);
    expect(used.filter((v) => !known.has(v))).toEqual([]);
  });
  it("stops motion for reduced-motion users", () => {
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*animation: none/);
  });
  it("draws focus in the text colour, never in red (D-484, superseding D-399/D-409)", () => {
    expect(uiCss).toMatch(/--ak-focus-color: var\(--foreground\);/);
    const rules = [
      ...strip(`${css}\n${uiCss}`).matchAll(
        /([^{}]*(?::focus-visible|:focus-within|:has\(:focus-visible\))[^{}]*)\{([^}]*)\}/g,
      ),
    ];
    expect(rules.length).toBeGreaterThan(10);
    for (const [, selector, body] of rules) {
      const indicator = (body ?? "")
        .split(";")
        .filter((d) => /box-shadow|outline|border(-bottom)?-color/.test(d));
      // The one red focus mark is the list's own: the dawn cursor bar on the active row is its focus (SPEC §2.8).
      const cursorBar = /\.ak-row--active/.test(selector ?? "");
      for (const d of indicator) {
        expect(d, selector?.trim()).not.toMatch(cursorBar ? /--accent/ : /--accent|--dawn/);
        if (cursorBar) continue;
        if (/outline:|box-shadow:|border(-bottom)?-color:/.test(d) && !/none|transparent|0 0 0 0/.test(d))
          expect(d, selector?.trim()).toMatch(
            /--ak-focus-color|--foreground|--ak-focus\b|--ak-stroke|--background|--ak-line/,
          );
      }
    }
  });
});

describe("source", () => {
  it('describes the licence only as "open source, free for personal use"', () => {
    const { execSync } = req("node:child_process") as typeof import("node:child_process");
    const grep = (re: string) =>
      execSync(`grep -rnoiIE '${re}' src index.html public || true`, {
        cwd: new URL("..", import.meta.url),
        encoding: "utf8",
      });
    const open = grep("open.source.{0,24}").split("\n").filter(Boolean);
    expect(open.length).toBeGreaterThan(0);
    for (const hit of open) expect(hit).toMatch(/:open source, free for personal use/i);
    expect(grep("source.available|fair.source|\\bOSS\\b")).toBe("");
  });
});
