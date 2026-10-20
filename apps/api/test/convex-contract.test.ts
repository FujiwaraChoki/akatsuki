// The function names apps/api calls by string must exist in the Convex backend with the expected kind.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { SERVICE_FUNCTIONS, USER_ENTITLEMENT } from "../src/convex.ts";

const convexDir = join(import.meta.dirname, "../../../convex");

describe("Convex function names", () => {
  it.each(Object.entries(SERVICE_FUNCTIONS))(
    "%s exists as an apps/api service function",
    (_k, [kind, path]) => {
      const [mod, fn] = path.split(":") as [string, string];
      const src = readFileSync(join(convexDir, `${mod}.ts`), "utf8");
      const wrapper = kind === "query" ? "apiQuery" : "apiMutation";
      expect(src).toMatch(new RegExp(`export const ${fn} = ${wrapper}\\(`));
    },
  );

  it("ee.billing.entitlement is a user query", () => {
    const [mod, fn] = USER_ENTITLEMENT.split(":") as [string, string];
    expect(readFileSync(join(convexDir, `${mod}.ts`), "utf8")).toMatch(
      new RegExp(`export const ${fn} = userQuery\\(`),
    );
  });
});
