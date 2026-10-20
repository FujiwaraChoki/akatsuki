// apps/api reads exactly the environment names documented for it: the apps/api row of ARCHITECTURE §18,
// infra/api.env.example and the apps/api part of the root .env.example (names only, never values).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { API_ENV_NAMES, DEFAULT_PRICE_LOOKUP_KEY, loadConfig } from "../src/config.ts";

const repo = join(import.meta.dirname, "../../..");
const read = (rel: string) => readFileSync(join(repo, rel), "utf8");
/** `NAME=` lines of an env example file (only the committed *.example files are ever read). */
const exampleNames = (text: string) =>
  text
    .split("\n")
    .map((l) => /^([A-Z][A-Z0-9_]*)=/.exec(l)?.[1])
    .filter((n): n is string => Boolean(n));

describe("environment names (ARCHITECTURE §18)", () => {
  it("config.ts reads exactly the apps/api row of §18", () => {
    const arch = read("docs/ARCHITECTURE.md");
    // The "Environment variable names" table (§14.2 has an apps/api row too).
    const table = arch.slice(arch.indexOf("**Environment variable names**"));
    const row = table.split("\n").find((l) => l.startsWith("| apps/api |"));
    expect(row, "§18 has an apps/api row").toBeDefined();
    const documented = [...row!.matchAll(/`([A-Z][A-Z0-9_]*)`/g)].map((m) => m[1]);
    expect([...API_ENV_NAMES].sort()).toEqual([...new Set(documented)].sort());
  });

  it("infra/api.env.example lists exactly those names", () => {
    expect(exampleNames(read("infra/api.env.example")).sort()).toEqual([...API_ENV_NAMES].sort());
  });

  it("the root .env.example documents every one of them", () => {
    const names = new Set(exampleNames(read(".env.example")));
    for (const n of API_ENV_NAMES) expect(names.has(n), n).toBe(true);
  });

  it("ignores undocumented variables and keeps Stripe keys out of the defaults", () => {
    const c = loadConfig({
      CONVEX_URL: "https://x.convex.cloud",
      CONVEX_SITE_URL: "https://x.convex.site",
      API_SERVICE_PRIVATE_KEY: "k",
      API_SERVICE_KID: "kid",
      STRIPE_PRICE_ID: "price_should_be_ignored",
      FAL_AI_API_KEY: "ignored",
    });
    expect(JSON.stringify(c)).not.toContain("price_should_be_ignored");
    expect(JSON.stringify(c)).not.toContain("ignored");
    expect(c.stripeSecretKey).toBeUndefined();
    expect(c.stripeWebhookSecret).toBeUndefined();
    expect(c.stripePriceLookupKey).toBe(DEFAULT_PRICE_LOOKUP_KEY);
    expect(DEFAULT_PRICE_LOOKUP_KEY).toBe("akatsuki_pro_monthly");
  });
});
