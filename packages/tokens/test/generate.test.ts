import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  hexToRgb,
  kebab,
  screamingSnake,
  type Tokens,
  toCss,
  toFontFaceCss,
  toRust,
  toTs,
  validateTokens,
} from "../src/generate.ts";

const pkgDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const raw = JSON.parse(readFileSync(join(pkgDir, "tokens.json"), "utf8"));
const tokens = validateTokens(raw);
const clone = (): Tokens => structuredClone(raw);

describe("helpers", () => {
  it("converts hex to rgb triplets", () => {
    expect(hexToRgb("#C1272D")).toEqual([193, 39, 45]);
    expect(hexToRgb("#800000")).toEqual([128, 0, 0]);
    expect(hexToRgb("#0a0a0a")).toEqual([10, 10, 10]);
    expect(() => hexToRgb("#fff")).toThrow();
  });
  it("names keys", () => {
    expect(kebab("borderHover")).toBe("border-hover");
    expect(kebab("0_5")).toBe("0-5");
    expect(screamingSnake("borderSubtle")).toBe("BORDER_SUBTLE");
    expect(screamingSnake("0_5")).toBe("0_5");
  });
});

describe("validateTokens", () => {
  it("accepts the brief's theme", () => {
    expect(tokens.color.light).toMatchObject({
      accent: "#C1272D",
      background: "#FFFFFF",
      foreground: "#171717",
    });
    expect(tokens.color.dark).toMatchObject({
      accent: "#800000",
      background: "#0A0A0A",
      foreground: "#EDEDED",
    });
  });
  it("accent text reads at WCAG AA on the background in both modes (D-399)", () => {
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
      expect(ratio(p.accentText, p.background), mode).toBeGreaterThanOrEqual(4.5);
    }
    // The deep crimson itself does not, which is why text uses accentText.
    expect(ratio(tokens.color.dark.accent, tokens.color.dark.background)).toBeLessThan(3);
  });
  it("rejects a bad colour", () => {
    const t = clone();
    t.color.dark.accent = "red" as `#${string}`;
    expect(() => validateTokens(t)).toThrow(/color.dark.accent/);
  });
  it("rejects a bold serif", () => {
    const t = clone();
    (t.text.app.subject as { weight: number }).weight = 700;
    expect(() => validateTokens(t)).toThrow(/serif has no bold/);
  });
  it("rejects spacing off the grid", () => {
    const t = clone();
    t.space.scale["1"] = 7;
    expect(() => validateTokens(t)).toThrow(/multiple of 4/);
  });
  it("rejects an opacity outside [0, 1]", () => {
    const t = clone();
    t.opacity.accent.wash = 8;
    expect(() => validateTokens(t)).toThrow(/opacity.accent.wash/);
  });
});

describe("toCss", () => {
  const css = toCss(tokens);
  it("emits palette variables with rgb triplets", () => {
    expect(css).toContain("--accent: #C1272D;");
    expect(css).toContain("--accent-rgb: 193 39 45;");
    expect(css).toContain("--foreground-rgb: 23 23 23;");
    expect(css).toContain("--accent-rgb: 128 0 0;");
    expect(css).toContain("--background: #0A0A0A;");
  });
  it("supports prefers-color-scheme and data-theme overrides", () => {
    expect(css).toContain("@media (prefers-color-scheme: dark)");
    expect(css).toContain(':root:not([data-theme="light"])');
    expect(css).toContain('[data-theme="dark"] {');
    const media = css.slice(css.indexOf("@media (prefers-color-scheme: dark)"));
    expect(media.slice(0, media.indexOf("}\n}"))).toContain("--accent: #800000;");
  });
  it("emits opacity steps, spacing, radii, type and motion", () => {
    expect(css).toContain("--fg-muted: rgb(var(--foreground-rgb) / 0.6);");
    expect(css).toContain("--accent-wash: rgb(var(--accent-rgb) / 0.08);");
    expect(css).toContain("--space-1: 8px;");
    expect(css).toContain("--space-0-5: 4px;");
    expect(css).toContain("--radius-full: 9999px;");
    expect(css).toContain('--font-serif: "Instrument Serif", Georgia, "Times New Roman", serif;');
    expect(css).toContain("--text-app-label-tracking: 0.12em;");
    expect(css).toContain("--text-brand-label-tracking: 0.3em;");
    expect(css).toContain("--ease-out: cubic-bezier(0, 0, 0.2, 1);");
  });
  it("honours reduced motion", () => {
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
  });
  it("has balanced braces", () => {
    expect(css.split("{").length).toBe(css.split("}").length);
  });
  it("is deterministic", () => {
    expect(toCss(tokens)).toBe(css);
  });
});

describe("First Light additions (D-480..D-482)", () => {
  it("adds dawn and onAccent to both palettes, everywhere", () => {
    for (const mode of ["light", "dark"] as const) {
      expect(tokens.color[mode].dawn).toBe("#C1272D");
      expect(tokens.color[mode].onAccent).toBe("#FFFFFF");
    }
    const css = toCss(tokens);
    expect(css).toContain("--dawn: #C1272D;");
    expect(css).toContain("--dawn-rgb: 193 39 45;");
    expect(css).toContain("--on-accent: #FFFFFF;");
    const rs = toRust(tokens);
    expect(rs).toContain("dawn: Rgb { r: 193, g: 39, b: 45 }");
    expect(rs).toContain("on_accent: Rgb { r: 255, g: 255, b: 255 }");
    expect(rs).toContain("pub on_accent: Rgb,");
    expect(toTs(tokens)).toContain(
      'export type ColorName = "accent" | "accentText" | "background" | "foreground" | "dawn" | "onAccent";',
    );
  });
  it("emits the neutral roles, section style, quick/cine motion, shell layout and pop shadow opacities", () => {
    const css = toCss(tokens);
    expect(css).toContain("--opacity-fg-tertiary: 0.7;");
    expect(css).toContain("--opacity-fg-surface: 0.04;");
    expect(css).toContain("--opacity-fg-surface-strong: 0.07;");
    expect(css).toContain("--text-app-section-weight: 500;");
    expect(css).toContain("--text-app-label-font: var(--font-mono);");
    expect(css).toContain("--text-app-body-strong-weight: 500;");
    expect(css).toContain("--text-app-subject-size-wide: 30px;");
    expect(css).toContain("--duration-quick: 140ms;");
    expect(css).toContain("--ease-cine: cubic-bezier(0.16, 1, 0.3, 1);");
    expect(css).toContain("--layout-sidebar-width: 216px;");
    expect(css).toContain("--layout-status-bar-height: 28px;");
    expect(css).toContain("--effect-pop-opacity-light: 0.14;");
    expect(css).toContain("--effect-pop-opacity-dark: 0.9;");
    const rs = toRust(tokens);
    expect(rs).toContain("pub const POP_OPACITY_DARK: f32 = 0.9;");
    expect(rs).toContain("pub const ROW_HEIGHT_TOUCH: f32 = 64.0;");
    expect(rs).toContain("pub const CINE: [f32; 4] = [0.16, 1.0, 0.3, 1.0];");
  });
  it("zeroes every duration under reduced motion, including new ones", () => {
    const css = toCss(tokens);
    const reduced = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"));
    for (const k of Object.keys(tokens.motion.duration))
      expect(reduced).toContain(`--duration-${kebab(k)}: 0ms;`);
  });
  it("keeps the deprecated tokens until desktop and mobile migrate, and names them in every output", () => {
    expect(tokens.motion.orb.blur).toBeGreaterThan(0);
    expect(tokens.effect.glowBlur).toBeGreaterThan(0);
    expect(toCss(tokens)).toContain("Deprecated (First Light, D-481");
    expect(toRust(tokens)).toContain("// Deprecated (First Light, D-481");
    const t = clone();
    t.$deprecated = { paths: ["effect.nope"] };
    expect(() => validateTokens(t)).toThrow(/effect.nope does not exist/);
  });
});

describe("toFontFaceCss", () => {
  it("references every bundled font file", () => {
    const css = toFontFaceCss(tokens);
    for (const role of ["serif", "sans", "mono"] as const) {
      for (const file of Object.values(tokens.font[role].files)) {
        expect(css).toContain(`url("../fonts/${file}")`);
        expect(readFileSync(join(pkgDir, "fonts", file)).byteLength).toBeGreaterThan(1000);
      }
    }
  });
});

describe("toTs", () => {
  it("compiles and exposes typed constants", async () => {
    const dir = mkdtempSync(join(tmpdir(), "ak-tokens-"));
    try {
      const file = join(dir, "tokens.ts");
      writeFileSync(file, toTs(tokens));
      const mod = await import(file);
      expect(mod.tokens.color.light.accent).toBe("#C1272D");
      expect(mod.tokens.color.dark.rgb.accent).toEqual([128, 0, 0]);
      expect(mod.space["1"]).toBe(8);
      expect(mod.fg("dark", "muted")).toBe("rgba(237, 237, 237, 0.6)");
      expect(mod.accent("light", "tint")).toBe("rgba(193, 39, 45, 0.15)");
      expect(mod.tokens.text.app.label.tracking).toBe(0.12);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("toRust", () => {
  const rs = toRust(tokens);
  it("emits palette and scale constants", () => {
    expect(rs).toContain("pub const LIGHT: Palette");
    expect(rs).toContain("accent: Rgb { r: 193, g: 39, b: 45 }");
    expect(rs).toContain("accent: Rgb { r: 128, g: 0, b: 0 }");
    expect(rs).toContain("pub const S0_5: f32 = 4.0;");
    expect(rs).toContain("pub const BORDER_SUBTLE: f32 = 0.1;");
    expect(rs).toContain('pub const FAMILY: &str = "Instrument Serif";');
  });
  const rustc = spawnSync("rustc", ["--version"], { encoding: "utf8" });
  it.skipIf(rustc.status !== 0)("compiles with rustc", () => {
    const dir = mkdtempSync(join(tmpdir(), "ak-tokens-rs-"));
    try {
      const file = join(dir, "generated.rs");
      writeFileSync(file, rs);
      execFileSync(
        "rustc",
        [
          "--edition",
          "2024",
          "--crate-type",
          "lib",
          "--crate-name",
          "ak_tokens",
          "-o",
          join(dir, "out.rlib"),
          file,
        ],
        {
          stdio: "pipe",
        },
      );
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe("web fonts (D-409)", () => {
  it("every bundled TTF has a WOFF2 sibling, listed first in fonts.css", async () => {
    const { existsSync, readFileSync } = await import("node:fs");
    const dir = new URL("../fonts/", import.meta.url);
    const css = readFileSync(new URL("../dist/fonts.css", import.meta.url), "utf8");
    for (const role of ["serif", "sans", "mono"] as const) {
      for (const file of Object.values(tokens.font[role].files)) {
        const woff2 = file.replace(/\.ttf$/, ".woff2");
        expect(existsSync(new URL(woff2, dir)), woff2).toBe(true);
        expect(css).toContain(
          `url("../fonts/${woff2}") format("woff2"), url("../fonts/${file}") format("truetype")`,
        );
      }
    }
  });
});
