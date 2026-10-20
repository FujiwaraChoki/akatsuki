import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import schema from "../schema";
import { makeTest, setEnv } from "./setup.helpers";

const here = dirname(fileURLToPath(import.meta.url));

/** The `defineSchema({...})` body with comments and all whitespace/trailing commas removed. */
function schemaBody(src: string): string {
  const start = src.indexOf("export default defineSchema({");
  if (start < 0) throw new Error("defineSchema not found");
  return src
    .slice(start)
    .replace(/\/\/[^\n]*/g, "")
    .replace(/\s+/g, "")
    .replace(/,([}\])])/g, "$1")
    .replace(/;$/, "");
}

describe("schema", () => {
  it("matches the contract in docs/ARCHITECTURE.md §3.2 exactly", () => {
    const doc = readFileSync(join(here, "../../docs/ARCHITECTURE.md"), "utf8");
    const section = doc.slice(doc.indexOf("### 3.2 Tables"), doc.indexOf("### 3.3 Write rules"));
    const block = section.slice(section.indexOf("```ts") + 5, section.lastIndexOf("```"));
    const code = readFileSync(join(here, "../schema.ts"), "utf8");
    expect(schemaBody(code)).toBe(schemaBody(block));
  });

  it("declares every contract table", () => {
    expect(Object.keys(schema.tables).sort()).toEqual(
      [
        "profiles",
        "devices",
        "mailAccounts",
        "accountSecrets",
        "syncState",
        "folderCursors",
        "accountRevs",
        "tombstones",
        "identities",
        "labels",
        "threads",
        "threadLabels",
        "threadSearch",
        "messages",
        "contacts",
        "ops",
        "drafts",
        "scheduledSends",
        "timers",
        "splits",
        "rules",
        "snippets",
        "senderPrefs",
        "readReceipts",
        "aiSettings",
        "aiUsage",
        "billing",
        "authRateLimits",
        "authHandoffs",
        "stripeEvents",
        "trialLedger",
      ].sort(),
    );
  });

  it("validates documents on insert", async () => {
    setEnv();
    const t = makeTest();
    await t.run(async (ctx) => {
      await ctx.db.insert("stripeEvents", { eventId: "evt_1", type: "invoice.paid", processedAt: 1 });
    });
    await expect(
      t.run(async (ctx) => {
        // @ts-expect-error missing required fields
        await ctx.db.insert("stripeEvents", { eventId: "evt_2" });
      }),
    ).rejects.toThrow();
  });
});
