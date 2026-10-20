import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "edge-runtime",
    server: { deps: { inline: ["convex-test"] } },
    include: ["test/**/*.test.ts"],
    // Node 26 warns on every worker that touches the (unused) Web Storage globals; the tests do not need them.
    execArgv: ["--no-experimental-webstorage"],
  },
});
