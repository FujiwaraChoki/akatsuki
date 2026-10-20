# apps/landing (akatsuki.vc)

The marketing site, "First Light" (D-430): home, pricing, download and the waitlist, privacy, terms,
licence summary, security and changelog. Next.js App Router with `output: "export"`: the build is plain
files in `out/`, with no server runtime, no analytics, no cookies and no third-party requests (the smoke
test fails on any). There is no React on the client: every page is static HTML, and two small vanilla
TypeScript bundles (`src/client/`, about 20 kB gzipped on home, 4 kB elsewhere) add the working demo inbox,
the keyboard, the model picker, the command palette, the shortcuts sheet and the theme control. Everything
renders in its final state without JavaScript and with reduced motion.

- The logo is a red dot (`.logo-dot`, `--dawn`), nothing else. The favicon, Apple touch icon and Open Graph
  images are drawn from it at build time with `next/og`; the 192 and 512 px manifest icons (and the mobile
  app's icons) come from `scripts/brand-icons.mjs`, which reads the accent from `@akatsuki/tokens`.
- Colours come from `@akatsuki/tokens` (`tokens.css`, imported first in `layout.tsx`). The only colour
  literals are the landing-only additions in `src/app/landing-tokens.css` (`--dawn`, `--on-accent`, the night
  band); a test checks each one against `packages/tokens/tokens.json`.
- Fonts: `public/fonts/*.woff2` are Latin subsets of `packages/tokens/fonts/*.ttf` (about 67 kB for all six
  instead of 250 kB), made by `pnpm --filter @akatsuki/landing fonts` (fonttools via `uvx`). Re-run it when the
  token fonts change. The OG images read the TTFs from the token package directly.
- Key labels, the demo inbox's key bindings and the in-mock palette come from `@akatsuki/shortcuts`.
- `/privacy` renders `docs/PRIVACY.md` and `/security` renders `SECURITY.md` at build time; `/license` embeds
  `LICENSE`. Edit those files, not the pages.
- `scripts/finish.mjs` runs after `next build`: it bundles `src/client` with esbuild, strips Next's client
  runtime from every page, gives the generated images `.png` names, and fails the build if a page carries an
  inline script whose hash is not in the CSP in `vercel.json`.

## Commands

| Command | What it does |
|---|---|
| `pnpm --filter @akatsuki/landing dev` | Dev server on http://localhost:3100 (without the post-build step, so no client scripts) |
| `pnpm --filter @akatsuki/landing build` | Static export to `out/` (needs `@akatsuki/tokens` built; `turbo run build` does that) |
| `pnpm --filter @akatsuki/landing start` | Serves `out/` on http://127.0.0.1:3101 (`PORT=` to change) with the `vercel.json` headers (CSP included) and gzip |
| `pnpm --filter @akatsuki/landing typecheck` | `next typegen` + `tsc --noEmit` |
| `pnpm --filter @akatsuki/landing test` | Vitest: copy rules ("open source" only as "open source, free for personal use"; every page that names the licence says commercial use needs permission; periods only on the hero and finale), palette (no colour literals, landing tokens match the shared palette), CSP hash, privacy-document facts, Markdown rendering, the demo inbox's selectors, sitemap |
| `pnpm --filter @akatsuki/landing test:e2e` | Playwright smoke test against `out/` under the production CSP, desktop and phone Chromium: every page, links, SEO files and icons, the keyboard demo, the quiet keyboard, palette, theme, waitlist, reduced motion, phone layout and overflow (run `build` first, or `turbo run test:e2e --filter=@akatsuki/landing`; `LANDING_E2E_PORT=` sets the port, default 3117) |
| `pnpm --filter @akatsuki/landing fonts` | Re-subsets the brand fonts into `public/fonts` |
| `node apps/landing/scripts/brand-icons.mjs` | Redraws every raster icon from the red dot (needs ImageMagick 7; outputs are committed) |

Playwright is pinned to 1.63.0, whose Chromium (revision 1243) is already installed on the dev machine. On a
fresh machine run `pnpm --filter @akatsuki/landing exec playwright install chromium` once.

## Deploying to Vercel (not done yet; needs the owner)

`vercel.json` in this directory holds the build settings and response headers. Create the project once:

1. New Project → import the repository → **Root Directory: `apps/landing`**. Leave "Include files outside the
   root directory in the Build Step" **on**: the build reads `docs/PRIVACY.md`, `SECURITY.md`, `LICENSE`,
   `packages/tokens` and `packages/shortcuts`.
2. Framework preset: **Other** (`vercel.json` sets `"framework": null`). The site is a static export; Vercel
   serves `out/` with clean URLs (`/pricing` → `pricing.html`) and the custom `404.html`.
3. Install command `pnpm install --frozen-lockfile`, build command
   `cd ../.. && pnpm exec turbo run build --filter=@akatsuki/landing`, output directory `out` (all from
   `vercel.json`).
4. Node.js version: 26.x (the workspace `engines` field requires it). pnpm: the root `packageManager` pins
   `pnpm@12.11.2`; set the environment variable `ENABLE_EXPERIMENTAL_COREPACK=1` so Vercel uses it.
5. No environment variables are needed. Do not add analytics or Speed Insights.
6. Domains: `akatsuki.vc` (apex, DNS at Cloudflare already points at Vercel) and `www.akatsuki.vc` redirecting
   to the apex.

The headers in `vercel.json` set a strict Content-Security-Policy: `script-src 'self'` plus the SHA-256 hash of
the one inline script, the theme boot script in `src/lib/boot.ts` (no `'unsafe-inline'` for scripts; if you
change the boot script, update the hash; the build and a unit test both fail until you do),
`style-src 'self' 'unsafe-inline'` (each page's stylesheet is inlined for first paint), `img-src 'self' data:`
(the dawn's grain texture), `connect-src 'self'` and no third-party origin anywhere. Also HSTS,
`X-Frame-Options: DENY`, `nosniff`, a Referrer-Policy and a Permissions-Policy, long-lived caching for
`/_next/static/*` and a month for `/fonts/*`.
