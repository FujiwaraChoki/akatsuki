// Bundles the service into one ESM file (dist/server.mjs) so the runtime image needs no node_modules.
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
await build({
  entryPoints: [join(root, "src/main.ts")],
  outfile: join(root, "dist/server.mjs"),
  bundle: true,
  platform: "node",
  target: "node26",
  format: "esm",
  conditions: ["node", "import"],
  legalComments: "linked",
  sourcemap: false,
  logLevel: "warning",
  // CommonJS dependencies inside an ESM bundle need require().
  banner: {
    js: "import { createRequire as __cr } from 'node:module'; const require = __cr(import.meta.url);",
  },
});
console.info("api: wrote dist/server.mjs");
