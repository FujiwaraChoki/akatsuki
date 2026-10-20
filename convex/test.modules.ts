// Function modules for convex-test. Lives at the functions root (convex.json "functions": ".");
// the double dot keeps the Convex bundler from treating it as a function module.
/// <reference types="vite/client" />
export const modules = import.meta.glob([
  "./**/*.*s",
  "!./node_modules/**",
  "!./test/**",
  "!./**/*.test.ts",
  "!./vitest.config.ts",
  "!./test.modules.ts",
]);
