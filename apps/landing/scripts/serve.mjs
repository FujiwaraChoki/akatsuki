// Serves the static export in out/ the way Vercel does: clean URLs (/pricing -> pricing.html),
// 404.html for misses, and the response headers from vercel.json (so the smoke test runs under the
// production Content-Security-Policy). Used by `pnpm start` and the Playwright smoke test. Local
// preview only, never a production server.
import { createReadStream, existsSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import { createGzip } from "node:zlib";

const root = path.resolve(import.meta.dirname, "../out");
const port = Number(process.env.PORT ?? 3101);

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".txt": "text/plain; charset=utf-8",
  ".xml": "application/xml",
  ".webmanifest": "application/manifest+json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".ttf": "font/ttf",
  ".woff2": "font/woff2",
  ".ico": "image/x-icon",
};

// vercel.json `headers`: sources use path-to-regexp syntax; ours only need "(...)" groups.
const vercel = JSON.parse(readFileSync(path.resolve(import.meta.dirname, "../vercel.json"), "utf8"));
const HEADER_RULES = vercel.headers.map((rule) => ({
  re: new RegExp(`^${rule.source}$`),
  headers: Object.fromEntries(rule.headers.map((h) => [h.key.toLowerCase(), h.value])),
}));

function headersFor(urlPath) {
  const out = {};
  for (const rule of HEADER_RULES) if (rule.re.test(urlPath)) Object.assign(out, rule.headers);
  // Plain http on localhost: HSTS and upgrade-insecure-requests would break the preview.
  delete out["strict-transport-security"];
  if (out["content-security-policy"]) {
    out["content-security-policy"] = out["content-security-policy"].replace(
      /;\s*upgrade-insecure-requests/,
      "",
    );
  }
  return out;
}

function resolve(urlPath) {
  const clean = decodeURIComponent(urlPath.split("?")[0] ?? "/");
  const abs = path.normalize(path.join(root, clean));
  if (!abs.startsWith(root)) return null;
  const candidates = [abs, `${abs}.html`, path.join(abs, "index.html")];
  for (const c of candidates) {
    if (existsSync(c) && statSync(c).isFile()) return c;
  }
  return null;
}

if (!existsSync(root)) {
  console.error("out/ does not exist: run `pnpm build` first");
  process.exit(1);
}

createServer((req, res) => {
  const urlPath = (req.url ?? "/").split("?")[0] ?? "/";
  const file = resolve(urlPath);
  if (!file) {
    res.writeHead(404, { "content-type": TYPES[".html"], ...headersFor(urlPath) });
    createReadStream(path.join(root, "404.html")).pipe(res);
    return;
  }
  const type = TYPES[path.extname(file)] ?? "application/octet-stream";
  // Vercel compresses text and fonts; do the same so local measurements are representative.
  const compress =
    /text|javascript|json|xml|font\/ttf/.test(type) && /gzip/.test(req.headers["accept-encoding"] ?? "");
  res.writeHead(200, {
    "content-type": type,
    "cache-control": "no-cache",
    ...(compress ? { "content-encoding": "gzip", vary: "accept-encoding" } : {}),
    ...headersFor(urlPath),
  });
  const stream = createReadStream(file);
  if (compress) stream.pipe(createGzip()).pipe(res);
  else stream.pipe(res);
}).listen(port, "127.0.0.1", () => {
  console.info(`serving ${root} on http://127.0.0.1:${port}`);
});
