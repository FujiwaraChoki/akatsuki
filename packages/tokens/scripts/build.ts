// Build: tokens.json -> dist/tokens.css, dist/fonts.css, dist/tokens.ts, dist/tokens.js,
// and crates/tokens/src/generated.rs (checked in; the Rust owner owns the crate shell).
// Run with `pnpm tokens:build` (Node runs this file directly through type stripping).
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { toCss, toFontFaceCss, toRust, toTs, validateTokens } from "../src/generate.ts";

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(pkgDir, "../..");

async function write(path: string, content: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  let previous: string | undefined;
  try {
    previous = await readFile(path, "utf8");
  } catch {
    previous = undefined;
  }
  if (previous === content) return;
  await writeFile(path, content);
  console.info(`wrote ${path.slice(repoRoot.length + 1)}`);
}

const tokens = validateTokens(JSON.parse(await readFile(join(pkgDir, "tokens.json"), "utf8")));
const ts = toTs(tokens);

await write(join(pkgDir, "dist/tokens.css"), toCss(tokens));
await write(join(pkgDir, "dist/fonts.css"), toFontFaceCss(tokens, "../fonts"));
await write(join(pkgDir, "dist/tokens.ts"), ts);

// Plain-JS twin for Node consumers that do not transpile workspace TypeScript.
const { transformSync } = await import("esbuild");
await write(
  join(pkgDir, "dist/tokens.js"),
  transformSync(ts, { loader: "ts", format: "esm", target: "es2022" }).code,
);

await write(join(repoRoot, "crates/tokens/src/generated.rs"), toRust(tokens));
