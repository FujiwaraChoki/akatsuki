#!/usr/bin/env bash
# Builds apps/web against the hosted backend and uploads dist/ to the Vercel project akatsuki-app
# (app.akatsuki.vc). Same approach as scripts/deploy-landing.sh: Vercel's builders top out at Node 24 and
# this repo pins Node 26, so the build runs here and the static output is uploaded with vercel.json's headers,
# CSP and SPA rewrite.
#
#   scripts/deploy-web.sh            # preview deployment
#   scripts/deploy-web.sh --prod     # production (app.akatsuki.vc)
#
# The VITE_* values are public (they end up in the bundle). Build from a clean checkout: Vite also reads
# apps/web/.env*.local files, and the process environment below only wins for the names it sets.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scope="${VERCEL_SCOPE:-fujiwarachokis-projects}"
project="${VERCEL_PROJECT:-akatsuki-app}"
vercel_cli="vercel@64.7.0"

target=()
if [[ "${1:-}" == "--prod" ]]; then
  target=(--prod)
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--prod]" >&2
  exit 2
fi

export VITE_CONVEX_URL="${VITE_CONVEX_URL:-https://healthy-stoat-314.convex.cloud}"
export VITE_CONVEX_SITE_URL="${VITE_CONVEX_SITE_URL:-https://healthy-stoat-314.convex.site}"
export VITE_SYNC_URL="${VITE_SYNC_URL:-https://sync.akatsuki.vc}"
export VITE_API_URL="${VITE_API_URL:-https://api.akatsuki.vc}"
export VITE_LOGIN_PROVIDERS="${VITE_LOGIN_PROVIDERS:-}"
export VITE_AKATSUKI_BACKEND=""

pnpm --dir "$repo_root" exec turbo run build --filter=./apps/web --force

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$repo_root/apps/web/dist" "$stage/dist"
node -e '
  const fs = require("node:fs");
  const [src, dest] = process.argv.slice(1);
  const config = JSON.parse(fs.readFileSync(src, "utf8"));
  Object.assign(config, { framework: null, installCommand: "", buildCommand: "", outputDirectory: "dist" });
  fs.writeFileSync(dest, JSON.stringify(config, null, 2));
' "$repo_root/apps/web/vercel.json" "$stage/vercel.json"

cd "$stage"
npx -y "$vercel_cli" deploy "${target[@]}" --yes --scope "$scope" --project "$project"
