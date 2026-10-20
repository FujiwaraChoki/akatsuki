import { defineConfig, devices } from "@playwright/test";

// Two suites:
// - default: the production bundle built in `mock` mode (in-memory backend, no network), served by
//   `vite preview` with the vercel.json headers (the real CSP), so the smoke tests run under production rules.
// - AKATSUKI_E2E_LOCAL=1: the real bundle against the LOCAL Convex harness (docs/DEV.md, `make convex-up` and
//   `make convex-deploy` first). Served on http://localhost:5173, the harness's SITE_URL / DEV_ORIGINS.
const local = process.env.AKATSUKI_E2E_LOCAL === "1";
const PORT = local ? 5173 : Number(process.env.AKATSUKI_WEB_E2E_PORT ?? 4317);
const baseURL = local ? `http://localhost:${PORT}` : `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: local ? "e2e/local" : "e2e/mock",
  fullyParallel: !local,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "github" : "list",
  timeout: local ? 60_000 : 30_000,
  use: { baseURL, trace: "retain-on-failure" },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } },
  ],
  webServer: local
    ? {
        command: `vite build --outDir dist-local && vite preview --outDir dist-local --port ${PORT} --strictPort`,
        env: {
          VITE_CONVEX_URL: "http://127.0.0.1:3410",
          VITE_CONVEX_SITE_URL: "http://127.0.0.1:3411",
          // scripts/e2e-local.mjs --web runs a real worker on a free port and passes its URL.
          VITE_SYNC_URL: process.env.AKATSUKI_E2E_SYNC_URL ?? "http://127.0.0.1:8787",
        },
        url: baseURL,
        reuseExistingServer: false,
        timeout: 120_000,
      }
    : {
        command: `vite build --mode mock --outDir dist-mock && vite preview --outDir dist-mock --port ${PORT} --host 127.0.0.1 --strictPort`,
        url: baseURL,
        reuseExistingServer: false,
        timeout: 120_000,
      },
});
