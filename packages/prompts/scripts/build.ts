// Bundles the plain prompt files into pack.json and writes the render goldens in fixtures/render/
// (the byte-exact prompts crates/ai must reproduce). Both outputs are checked in; a test fails when stale.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { loadEvalFixtures, loadPromptPackFromDir, PACK_ROOT } from "../src/fs.ts";
import { renderGoldens } from "../src/goldens.ts";

const pack = loadPromptPackFromDir(PACK_ROOT);
const written: string[] = [];

function write(path: string, value: unknown) {
  const text = `${JSON.stringify(value, null, 2)}\n`;
  const previous = existsSync(path) ? readFileSync(path, "utf8") : undefined;
  // Compare structurally: Biome may have reformatted the previous file.
  if (previous !== undefined && JSON.stringify(JSON.parse(previous)) === JSON.stringify(value)) return;
  writeFileSync(path, text);
  written.push(path);
}

write(join(PACK_ROOT, "pack.json"), pack);

const renderDir = join(PACK_ROOT, "fixtures", "render");
mkdirSync(renderDir, { recursive: true });
const goldens = renderGoldens(pack, loadEvalFixtures(PACK_ROOT));
const wanted = new Set(goldens.map((g) => `${g.feature}.${g.name}.json`));
for (const file of readdirSync(renderDir)) if (!wanted.has(file)) rmSync(join(renderDir, file));
for (const g of goldens) write(join(renderDir, `${g.feature}.${g.name}.json`), g);

if (written.length > 0) {
  // Keep generated JSON in the repository's Biome format so `pnpm lint` stays green.
  const biome = join(PACK_ROOT, "../../node_modules/.bin/biome");
  if (existsSync(biome)) execFileSync(biome, ["format", "--write", ...written], { stdio: "ignore" });
  console.info(`prompts: wrote ${written.length} file(s)`);
}
