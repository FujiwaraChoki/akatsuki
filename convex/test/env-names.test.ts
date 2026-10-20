// @vitest-environment node
// ARCHITECTURE §18 and the root .env.example list the same environment names (D-355), and every name the Convex
// functions read is in the Convex row (review round 2, D-404: SHARD_COUNT was read but not listed).
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const repo = join(import.meta.dirname, "../..");
const read = (rel: string) => readFileSync(join(repo, rel), "utf8");

const table = (() => {
  const arch = read("docs/ARCHITECTURE.md");
  const start = arch.indexOf("**Environment variable names**");
  return arch.slice(start, arch.indexOf("\n\n", arch.indexOf("| Component |", start)));
})();
const names = (text: string) =>
  new Set([...text.matchAll(/`([A-Z][A-Z0-9_]*)(?:\/SECRET)?`/g)].map((m) => m[1]!));
const rowNames = (prefix: string) => {
  const row = table.split("\n").find((l) => l.startsWith(prefix));
  if (!row) throw new Error(`no §18 row ${prefix}`);
  const out = names(row);
  // `GOOGLE_LOGIN_CLIENT_ID/SECRET` lists two names.
  for (const m of row.matchAll(/`([A-Z][A-Z0-9_]*)_ID\/SECRET`/g)) {
    out.add(`${m[1]}_ID`);
    out.add(`${m[1]}_SECRET`);
  }
  return out;
};
const exampleNames = new Set(
  read(".env.example")
    .split("\n")
    .map((l) => /^([A-Z][A-Z0-9_]*)=/.exec(l)?.[1])
    .filter((n): n is string => Boolean(n)),
);

describe("environment names (§18, D-355)", () => {
  it("the table and .env.example list the same names", () => {
    const documented = new Set<string>();
    for (const line of table.split("\n").filter((l) => l.startsWith("| ") && !l.startsWith("| Component")))
      for (const n of rowNames(line.slice(0, line.indexOf("|", 2) + 1))) documented.add(n);
    // Convex provides CONVEX_SITE_URL itself; the CLI-only deploy key lives in CI.
    const missingFromExample = [...documented].filter((n) => !exampleNames.has(n));
    const missingFromTable = [...exampleNames].filter((n) => !documented.has(n) && n !== "CONVEX_DEPLOY_KEY");
    expect(documented.size).toBeGreaterThan(80);
    expect(missingFromExample).toEqual([]);
    expect(missingFromTable).toEqual([]);
  });

  it("every name the Convex functions read is in the Convex row", () => {
    const convexRow = rowNames("| Convex deployment env");
    const files = ["", "lib", "ee"].flatMap((dir) =>
      readdirSync(join(repo, "convex", dir))
        .filter((f) => f.endsWith(".ts") && !f.includes(".test.") && !f.endsWith(".d.ts"))
        .map((f) => read(join("convex", dir, f))),
    );
    const read_ = new Set<string>();
    for (const src of files)
      for (const m of src.matchAll(/\b(?:process\.env|env)\.([A-Z][A-Z0-9_]+)/g)) read_.add(m[1]!);
    const allowed = new Set([...convexRow, "CONVEX_SITE_URL", "NODE_ENV"]);
    expect([...read_].filter((n) => !allowed.has(n)).sort()).toEqual([]);
    expect(read_.has("SHARD_COUNT")).toBe(true);
  });
});
