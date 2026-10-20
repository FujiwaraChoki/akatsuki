// Post-build: the export is static HTML, so ship it without the framework runtime.
// 1. Bundle src/client/entry.ts (home) and src/client/entry-doc.ts (every other page) with esbuild,
//    minified, one ES module each. There is no React on the client.
// 2. In every exported page, remove Next's runtime scripts, the inline RSC payload and script
//    preloads, keep the theme boot script, dedupe font preloads, and load the bundle as a module.
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { build } from "esbuild";

const root = path.join(import.meta.dirname, "..");
const out = path.join(root, "out");

for (const [entry, file] of [
  ["entry.ts", "app.js"],
  ["entry-doc.ts", "site.js"],
]) {
  await build({
    entryPoints: [path.join(root, "src/client", entry)],
    outfile: path.join(out, file),
    bundle: true,
    minify: true,
    format: "esm",
    target: "es2022",
    platform: "browser",
    legalComments: "none",
    logLevel: "warning",
  });
}

// Generated images get real extensions, so any static host serves them as image/png.
const IMAGES = ["opengraph-image", "twitter-image", "apple-icon", "icon"];
for (const name of IMAGES) {
  const f = path.join(out, name);
  if (existsSync(f) && statSync(f).isFile()) renameSync(f, `${f}.png`);
}
const withPng = (html) =>
  html.replace(/(?<![\w-])(opengraph-image|twitter-image|apple-icon|icon)\?/g, "$1.png?");

const pages = [];
const walk = (d) => {
  for (const f of readdirSync(d)) {
    const p = path.join(d, f);
    if (statSync(p).isDirectory()) {
      if (f !== "_next") walk(p);
    } else if (f.endsWith(".html")) pages.push(p);
  }
};
walk(out);

// vercel.json allows exactly one inline script, the theme boot script, by hash ('unsafe-inline' is
// gone). Fail the build if a page carries any other inline script, or the boot script changed and
// the CSP was not updated.
const csp = JSON.parse(readFileSync(path.join(root, "vercel.json"), "utf8"))
  .headers.flatMap((r) => r.headers)
  .find((h) => h.key === "Content-Security-Policy").value;

for (const file of pages) {
  let html = withPng(readFileSync(file, "utf8"));
  const before = html.length;
  html = html.replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, (tag) =>
    tag.includes("ak-theme") && !tag.includes("__next_f") ? tag : "",
  );
  html = html.replace(/<link rel="(?:preload|modulepreload)" as="script"[^>]*\/?>/g, "");
  html = html.replace(/<link rel="modulepreload"[^>]*\/?>/g, "");
  const seen = new Set();
  html = html.replace(/<link rel="preload" href="([^"]+)" as="font"[^>]*\/?>/g, (tag, href) => {
    if (seen.has(href)) return "";
    seen.add(href);
    return tag;
  });
  const inline = [...html.matchAll(/<script(?![^>]*\bsrc=)[^>]*>([\s\S]*?)<\/script>/g)].map(
    (m) => m[1] ?? "",
  );
  for (const body of inline) {
    const hash = `'sha256-${createHash("sha256").update(body).digest("base64")}'`;
    if (!csp.includes(hash)) {
      throw new Error(
        `${path.relative(out, file)}: inline script ${hash} is not allowed by the CSP in vercel.json`,
      );
    }
  }
  const bundle = html.includes("data-inbox") ? "/app.js" : "/site.js";
  html = html.replace("</body>", `<script type="module" src="${bundle}"></script></body>`);
  writeFileSync(file, html);
  console.info(path.relative(out, file), before, "→", html.length);
}

// The RSC payload files and runtime chunks are no longer referenced by any page.
for (const f of readdirSync(out)) if (f.endsWith(".txt") && f !== "robots.txt") rmSync(path.join(out, f));
const walkTxt = (d) => {
  for (const f of readdirSync(d)) {
    const p = path.join(d, f);
    if (statSync(p).isDirectory()) walkTxt(p);
    else if (f.endsWith(".txt") && p !== path.join(out, "robots.txt")) rmSync(p);
  }
};
walkTxt(out);
rmSync(path.join(out, "_next", "static", "chunks"), { recursive: true, force: true });

// Leave nothing a static host could trip over: the build's empty route directories (with
// try_files $uri $uri/, an empty pricing/ can win over pricing.html), the unused _not-found page
// and the build manifests nothing references any more.
for (const f of ["_not-found.html", "_not-found"])
  rmSync(path.join(out, f), { recursive: true, force: true });
const pruneEmpty = (d) => {
  for (const f of readdirSync(d)) {
    const p = path.join(d, f);
    if (statSync(p).isDirectory()) pruneEmpty(p);
  }
  if (d !== out && readdirSync(d).length === 0) rmSync(d, { recursive: true });
};
const nextStatic = path.join(out, "_next", "static");
if (existsSync(nextStatic)) {
  for (const f of readdirSync(nextStatic)) {
    if (f === "media" || f === "css") continue;
    rmSync(path.join(nextStatic, f), { recursive: true, force: true });
  }
}
pruneEmpty(out);
