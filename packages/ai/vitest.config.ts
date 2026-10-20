import { defineConfig } from "vitest/config";

export default defineConfig({
  // The known AI SDK compatibility notice of the OpenRouter provider is filtered, like in apps/api (D-396).
  test: { setupFiles: ["./test/setup.ts"] },
});
