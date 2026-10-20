import "@akatsuki/tokens/tokens.css";
import "@akatsuki/tokens/fonts.css";
import "@akatsuki/ui/styles.css";
import "./styles/app.css";
import "./styles/features.css";
import { color } from "@akatsuki/tokens";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App.tsx";
import { createConvexBackend } from "./backend/convex.tsx";
import type { Backend } from "./backend/types.ts";
import { originOf, readEnv } from "./env.ts";

declare const __AKATSUKI_MOCK__: boolean;

/** The tab icon: the logo, a red dot, drawn from the tokens (no hard-coded palette). */
function setFavicon() {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><circle cx="16" cy="16" r="14" fill="${color.light.accent}"/></svg>`;
  const link = document.createElement("link");
  link.rel = "icon";
  link.type = "image/svg+xml";
  link.href = `data:image/svg+xml,${encodeURIComponent(svg)}`;
  document.head.appendChild(link);
  // The in-app mark is the same dot, in the light accent in both themes (the dark accent is too dark
  // on the dark background to read as the logo).
  document.documentElement.style.setProperty("--ak-dot", color.light.accent);
}

function ConfigError() {
  return (
    <main className="ak-fatal">
      <h1 className="ak-heading ak-heading--heading">This build is not configured.</h1>
      <p>
        Set VITE_CONVEX_URL and VITE_CONVEX_SITE_URL (and VITE_SYNC_URL) when building the web app. See
        docs/DEV.md.
      </p>
    </main>
  );
}

async function start() {
  setFavicon();
  const env = readEnv();
  const root = createRoot(document.getElementById("root")!);
  let backend: Backend | null = null;
  let syncOrigin = originOf(env.syncUrl);
  if (__AKATSUKI_MOCK__ && env.backend === "mock") {
    const { createMockBackend } = await import("./backend/mock/index.tsx");
    backend = createMockBackend();
    syncOrigin = window.location.origin;
  } else if (env.convexUrl && env.convexSiteUrl) backend = createConvexBackend(env);

  root.render(
    <StrictMode>{backend ? <App backend={backend} syncOrigin={syncOrigin} /> : <ConfigError />}</StrictMode>,
  );
}

void start();
