// Unit tests for the view-models in src/vm and the token mapping in src/theme: plain TypeScript, no React Native
// runtime, so they run in Node like every other package.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["test/**/*.test.ts"],
  },
});
