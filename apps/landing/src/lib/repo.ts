// Build-time access to files elsewhere in the monorepo (docs, SECURITY.md, LICENSE).
// Only server components call this; with `output: "export"` it runs during `next build` only.
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

function findRepoRoot(start: string): string {
  let dir = start;
  for (;;) {
    if (existsSync(path.join(dir, "pnpm-workspace.yaml"))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) throw new Error(`repository root not found above ${start}`);
    dir = parent;
  }
}

let root: string | undefined;

export function repoRoot(): string {
  root ??= findRepoRoot(process.cwd());
  return root;
}

/** Reads a repository file. Refuses .env files and paths outside the repository. */
export function readRepoFile(relative: string): string {
  const base = path.basename(relative);
  if (base.startsWith(".env")) throw new Error("refusing to read an env file");
  const abs = path.resolve(repoRoot(), relative);
  if (!abs.startsWith(repoRoot() + path.sep)) throw new Error(`path escapes the repository: ${relative}`);
  return readFileSync(abs, "utf8");
}
