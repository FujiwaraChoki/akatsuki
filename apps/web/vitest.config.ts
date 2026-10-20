import { defineConfig } from "vitest/config";

export default defineConfig({
  define: { __AKATSUKI_MOCK__: "true" },
  test: {
    environment: "jsdom",
    include: ["test/**/*.test.{ts,tsx}"],
    setupFiles: ["test/setup.ts"],
    testTimeout: 20000,
  },
});
