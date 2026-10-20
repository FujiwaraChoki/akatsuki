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
    expect(css).not.toMatch(/\b(rgb|rgba|hsl|hsla|oklch|lab|color-mix)\((?!var\()/);
    expect(css).not.toMatch(/:\s*(white|black|red|gray|grey)\b/i);
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
  it("draws focus with the focus colour, never with the faint accent mid or tint (D-409)", () => {
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
      for (const d of indicator) expect(d, selector?.trim()).not.toMatch(/--accent-(mid|tint|wash|glow)/);
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
