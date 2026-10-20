import path from "node:path";
import type { NextConfig } from "next";

// Static export: the whole site is plain files in `out/`, served by Vercel (see vercel.json and README.md).
// No server runtime, no image optimizer, no middleware, no analytics.
const config: NextConfig = {
  output: "export",
  reactStrictMode: true,
  poweredByHeader: false,
  images: { unoptimized: true },
  // Inline the (small) stylesheet into each page so first paint does not wait for a CSS request.
  experimental: { inlineCss: true },
  // Workspace packages ship TypeScript sources and JSON.
  transpilePackages: ["@akatsuki/shortcuts", "@akatsuki/tokens"],
  turbopack: { root: path.join(import.meta.dirname, "../..") },
  outputFileTracingRoot: path.join(import.meta.dirname, "../.."),
};

export default config;
