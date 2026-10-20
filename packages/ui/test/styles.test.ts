// @vitest-environment node
// styles.css uses only @akatsuki/tokens variables: no colour literals, and every var() is defined by the
// tokens stylesheet or by styles.css itself (its derived --ak-* roles).
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(
  /\/\*[\s\S]*?\*\//g,
  "",
);
const tokensCss = readFileSync(createRequire(import.meta.url).resolve("@akatsuki/tokens/tokens.css"), "utf8");

const defined = (src: string) => new Set([...src.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]!));

describe("styles.css", () => {
  it("has no colour literals", () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    // Colour functions only over token variables: rgb(var(--x-rgb) / a) and color-mix of two token colours.
    expect(css).not.toMatch(/\b(rgb|rgba|hsl|hsla|oklch|lab)\((?!var\()/);
    for (const m of css.matchAll(/\bcolor-mix\(/g))
      expect(
        css
          .slice(m.index + 10, m.index + 60)
          .replace(/\s+/g, " ")
          .trim(),
      ).toMatch(/^in (srgb|oklab), var\(--[a-z-]+\)/);
    expect(css).not.toMatch(/:\s*(white|black|red|gray|grey)\b/i);
    for (const m of css.matchAll(/\brgb\(/g))
      expect(css.slice(m.index + 4, m.index + 60)).toMatch(/^var\(--[a-z-]+-rgb\) \/ /);
  });

  it("uses only defined variables", () => {
    const known = new Set([...defined(tokensCss), ...defined(css)]);
    const used = [...css.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1]!);
    expect(used.length).toBeGreaterThan(100);
    expect(used.filter((v) => !known.has(v))).toEqual([]);
  });

  it("draws focus in the text colour, at least 3:1 against the page in both modes (D-484, supersedes D-399/D-409)", () => {
    expect(css).toMatch(/--ak-focus-color: var\(--foreground\);/);
    expect(css).toMatch(
      /:focus-visible \{\s*outline: 2px solid var\(--ak-focus-color\);\s*outline-offset: 2px;/,
    );
    const tokens = createRequire(import.meta.url)("@akatsuki/tokens/tokens.json") as {
      color: Record<"light" | "dark", { foreground: string; background: string }>;
    };
    const lum = (hex: string) => {
      const c = [1, 3, 5].map((i) => Number.parseInt(hex.slice(i, i + 2), 16) / 255);
      const l = c.map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
      return 0.2126 * l[0]! + 0.7152 * l[1]! + 0.0722 * l[2]!;
    };
    const ratio = (a: string, b: string) => {
      const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
      return (x! + 0.05) / (y! + 0.05);
    };
    for (const mode of ["light", "dark"] as const) {
      const p = tokens.color[mode];
      expect(ratio(p.foreground, p.background), mode).toBeGreaterThanOrEqual(3);
    }
  });

  it("has no decoration: no blur, no radial glow, no accent tints (First Light, D-480)", () => {
    expect(css).not.toMatch(/backdrop-filter|blur\(|radial-gradient|--glow\b|--accent-(tint|wash|glow|mid)/);
    expect(css).not.toMatch(/\.ak-orb/);
  });

  it("stops motion for reduced-motion users", () => {
    expect(css).toMatch(
      /@media \(prefers-reduced-motion: reduce\)[\s\S]*\.ak-palette[\s\S]*animation: ak-fade-in 80ms/,
    );
  });
});
