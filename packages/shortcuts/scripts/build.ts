// Writes crates/shortcuts/src/generated.rs from shortcuts.json (checked in; the Rust owner owns the crate shell).
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { findConflicts, SHORTCUTS, SHORTCUTS_VERSION } from "../src/index.ts";
import { toRust } from "../src/rust.ts";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const target = join(repoRoot, "crates/shortcuts/src/generated.rs");

const conflicts = findConflicts();
if (conflicts.length > 0) {
  for (const c of conflicts)
    console.error(`conflict (${c.platform}, ${c.keymap}): ${c.a} and ${c.b} on "${c.keys}"`);
  process.exit(1);
}

const rs = toRust(SHORTCUTS, SHORTCUTS_VERSION);
await mkdir(dirname(target), { recursive: true });
let previous: string | undefined;
try {
  previous = await readFile(target, "utf8");
} catch {
  previous = undefined;
}
if (previous !== rs) {
  await writeFile(target, rs);
  console.info("wrote crates/shortcuts/src/generated.rs");
}
