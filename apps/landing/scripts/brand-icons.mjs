// The Akatsuki logo is a red dot, nothing else (D-430). This script draws every raster icon the
// apps ship from that one shape, in the token accent (packages/tokens/tokens.json, light theme), so
// no icon is drawn by hand and none can drift. The outputs are committed; run it again only when the
// shape or the accent changes:
//
//   node apps/landing/scripts/brand-icons.mjs        (from the repository root; needs ImageMagick 7)
//
// Writes the source SVG to packages/tokens/brand/dot.svg, the landing manifest icons and the Expo
// assets referenced by apps/mobile/app.config.ts. The web app needs no file (its favicon is an
// inline SVG drawn from the tokens in apps/web/src/main.tsx); the desktop app has no bundle icon yet.
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

const repo = path.resolve(import.meta.dirname, "../../..");
const tokens = JSON.parse(readFileSync(path.join(repo, "packages/tokens/tokens.json"), "utf8"));
const RED = tokens.color.light.accent;
const WHITE = tokens.color.light.background;

/** A dot of `ratio` × size, centred; `ground` paints an opaque background (no alpha channel). */
function dot(file, size, ratio, { ground, fill = RED } = {}) {
  const r = (size * ratio) / 2;
  const c = size / 2;
  const out = path.join(repo, file);
  mkdirSync(path.dirname(out), { recursive: true });
  const args = [
    "-size",
    `${size}x${size}`,
    ground ? `xc:${ground}` : "xc:none",
    "-fill",
    fill,
    "-draw",
    `circle ${c},${c} ${c + r},${c}`,
  ];
  if (ground) args.push("-alpha", "off");
  execFileSync("magick", [...args, `${ground ? "PNG24" : "PNG32"}:${out}`]);
  console.info(`${file}  ${size}px  dot ${Math.round(ratio * 100)}%${ground ? `  on ${ground}` : ""}`);
}

// The source shape, for anything that takes vector art.
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" role="img" aria-label="Akatsuki"><circle cx="32" cy="32" r="28" fill="${RED}"/></svg>\n`;
mkdirSync(path.join(repo, "packages/tokens/brand"), { recursive: true });
writeFileSync(path.join(repo, "packages/tokens/brand/dot.svg"), svg);
console.info("packages/tokens/brand/dot.svg");

// Landing: web manifest icons (the favicon and Apple touch icon are drawn by next/og at build time).
dot("apps/landing/public/icon-192.png", 192, 0.86);
dot("apps/landing/public/icon-512.png", 512, 0.86);

// Mobile (Expo). iOS icons must be opaque; Android's adaptive foreground keeps the dot inside the
// 66% safe circle; the notification icon is a white silhouette that Android tints with the accent.
dot("apps/mobile/assets/icon.png", 1024, 0.5, { ground: WHITE });
dot("apps/mobile/assets/adaptive-icon.png", 1024, 0.34);
dot("apps/mobile/assets/splash-icon.png", 512, 0.62);
dot("apps/mobile/assets/splash-icon-dark.png", 512, 0.62);
dot("apps/mobile/assets/notification-icon.png", 96, 0.7, { fill: WHITE });
dot("apps/mobile/assets/favicon.png", 48, 0.75, { ground: WHITE });
