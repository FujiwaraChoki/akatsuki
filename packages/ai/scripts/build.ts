// Writes schema/ai-settings.schema.json from the zod settings schema (src/settings.ts) for crates/ai::settings.
// Cross-field rules (unique profileIds, routes pointing at known profiles, baseUrl policy, voiceProfile
// consent) are not expressible in JSON Schema; both sides also run fixtures/settings/*.json.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { settingsJsonSchema } from "../src/settings-schema.ts";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const target = join(root, "schema", "ai-settings.schema.json");
const next = settingsJsonSchema();
const previous = existsSync(target) ? JSON.parse(readFileSync(target, "utf8")) : undefined;
if (JSON.stringify(previous) !== JSON.stringify(next)) {
  writeFileSync(target, `${JSON.stringify(next, null, 2)}\n`);
  const biome = join(root, "../../node_modules/.bin/biome");
  if (existsSync(biome)) execFileSync(biome, ["format", "--write", target], { stdio: "ignore" });
  console.info("ai: wrote schema/ai-settings.schema.json");
}
