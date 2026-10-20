// JSON Schema export of the settings model (used by scripts/build.ts and the up-to-date test).
import { z } from "zod";
import { zAiSettings } from "./settings.ts";

export function settingsJsonSchema(): Record<string, unknown> {
  const schema = z.toJSONSchema(zAiSettings, {
    target: "draft-7",
    unrepresentable: "any",
    io: "input",
  }) as Record<string, unknown>;
  return {
    ...schema,
    $id: "https://akatsuki.vc/schema/ai-settings.schema.json",
    title: "Akatsuki AI settings (Convex aiSettings, §12.2)",
  };
}
