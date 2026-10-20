// Every error this package raises carries a §15 registry code (D-396): no ad-hoc codes like "managed".
import { readdirSync, readFileSync } from "node:fs";
import { ERROR_CODES } from "@akatsuki/wire/errors";
import { describe, expect, it } from "vitest";

describe("AiFeatureError codes", () => {
  it("are all in the registry", () => {
    const dir = new URL("../src/", import.meta.url);
    const found: string[] = [];
    for (const f of readdirSync(dir).filter((n) => n.endsWith(".ts"))) {
      const src = readFileSync(new URL(f, dir), "utf8");
      for (const m of src.matchAll(/new AiFeatureError\(\s*"([a-z_]+)"/g)) found.push(m[1]!);
    }
    expect(found.length).toBeGreaterThan(5);
    for (const code of found) expect(Object.keys(ERROR_CODES), code).toContain(code);
  });
});

describe("AI SDK warning filter", () => {
  it("drops only the known v2 compatibility notice", async () => {
    const { installAiWarningFilter } = await import("../src/warnings.ts");
    const lines: string[] = [];
    installAiWarningFilter((l) => lines.push(l));
    const logger = globalThis.AI_SDK_LOG_WARNINGS;
    if (typeof logger !== "function") throw new Error("filter not installed");
    logger({
      warnings: [
        { type: "compatibility", feature: "specificationVersion" },
        { type: "unsupported", feature: "temperature" },
      ],
      provider: "openrouter",
      model: "m",
    } as never);
    expect(lines).toEqual(["AI SDK warning (openrouter / m): unsupported temperature"]);
    installAiWarningFilter();
  });
});
