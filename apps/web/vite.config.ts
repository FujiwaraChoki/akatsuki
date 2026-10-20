// Vite config for app.akatsuki.vc (ARCHITECTURE §6.2, D-260).
//
// Env: Vite reads only `VITE_*` variables, from the process environment and from env files in this
// directory (apps/web). It deliberately does NOT point `envDir` at the repo root, whose `.env` holds the
// owner's keys and must never be read. Against the local Convex harness the URLs default to the documented
// ports (docs/DEV.md: 3410 / 3411), so `pnpm dev` works with no env file at all.
//
// Modes: `development` / `production` use Convex + Better Auth; `mock` (`pnpm build:mock`, Playwright) and
// `test` (Vitest) swap in the in-memory backend in `src/backend/mock`, which production builds tree-shake away.
import { readFileSync } from "node:fs";
import { defineConfig, type Plugin } from "vite";

/**
 * Preloads the two faces every first paint uses (Geist Regular for the UI, Instrument Serif for headings) as
 * WOFF2; the other weights load on demand through @akatsuki/tokens/fonts.css (D-409).
 */
export const PRELOAD_FONTS = /(?:^|\/)(?:Geist-Regular|InstrumentSerif-Regular)-[\w-]+\.woff2$/;
function preloadFonts(): Plugin {
  return {
    name: "akatsuki-preload-fonts",
    apply: "build",
    transformIndexHtml: {
      order: "post",
      handler(_html, ctx) {
        return Object.keys(ctx.bundle ?? {})
          .filter((f) => PRELOAD_FONTS.test(f))
          .sort()
          .map((f) => ({
            tag: "link",
            attrs: { rel: "preload", href: `/${f}`, as: "font", type: "font/woff2", crossorigin: "" },
            injectTo: "head" as const,
          }));
      },
    },
  };
}

interface VercelHeaders {
  headers: { source: string; headers: { key: string; value: string }[] }[];
}

/** The production headers from vercel.json (CSP and friends), applied to `vite preview` so tests run under them. */
function productionHeaders(): Record<string, string> {
  const config = JSON.parse(readFileSync(new URL("./vercel.json", import.meta.url), "utf8")) as VercelHeaders;
  const all = config.headers.find((h) => h.source === "/(.*)");
  const headers = Object.fromEntries((all?.headers ?? []).map((h) => [h.key, h.value]));
  // The local Convex harness speaks plain ws:// on loopback (production Convex is wss://, already allowed).
  const csp = headers["Content-Security-Policy"];
  if (csp)
    headers["Content-Security-Policy"] = csp.replace(
      /connect-src ([^;]*)/,
      "connect-src $1 ws://127.0.0.1:* ws://localhost:*",
    );
  return headers;
}

export default defineConfig(({ mode }) => ({
  plugins: [preloadFonts()],
  define: {
    __AKATSUKI_MOCK__: JSON.stringify(mode === "mock" || mode === "test"),
  },
  build: {
    target: "es2023",
    sourcemap: true,
    // One small entry; the mock backend and part-2 features are separate chunks.
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      onwarn(warning, warn) {
        // Radix ships "use client" directives for React Server Components; meaningless in a SPA bundle.
        if (warning.code === "MODULE_LEVEL_DIRECTIVE") return;
        warn(warning);
      },
    },
  },
  server: {
    // Better Auth's SITE_URL and DEV_ORIGINS on the local harness are http://localhost:5173 (scripts/convex-dev.mjs).
    port: 5173,
    strictPort: true,
  },
  preview: {
    port: Number(process.env.AKATSUKI_WEB_PORT ?? 4173),
    strictPort: true,
    headers: productionHeaders(),
  },
}));
