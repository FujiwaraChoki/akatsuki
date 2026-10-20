#!/usr/bin/env bash
# Builds apps/landing and uploads its static export to the Vercel project akatsuki-landing (akatsuki.vc).
#
# Vercel's builders top out at Node 24 and this repo pins Node 26 with engine-strict, so the site is built
# here and uploaded as plain files: install and build commands are empty, and the headers, CSP, clean URLs
# and output directory come from apps/landing/vercel.json.
#
#   scripts/deploy-landing.sh            # preview deployment
#   scripts/deploy-landing.sh --prod     # production (akatsuki.vc)
#
# Needs a logged-in Vercel CLI (`npx vercel login`) or VERCEL_TOKEN in the environment.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
scope="${VERCEL_SCOPE:-fujiwarachokis-projects}"
project="${VERCEL_PROJECT:-akatsuki-landing}"
vercel_cli="vercel@64.7.0"

target=()
if [[ "${1:-}" == "--prod" ]]; then
  target=(--prod)
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--prod]" >&2
  exit 2
fi

pnpm --dir "$repo_root" exec turbo run build --filter=@akatsuki/landing

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$repo_root/apps/landing/out" "$stage/out"
node -e '
  const fs = require("node:fs");
  const [src, dest] = process.argv.slice(1);
  const config = JSON.parse(fs.readFileSync(src, "utf8"));
  Object.assign(config, { framework: null, installCommand: "", buildCommand: "", outputDirectory: "out" });
  fs.writeFileSync(dest, JSON.stringify(config, null, 2));
' "$repo_root/apps/landing/vercel.json" "$stage/vercel.json"

cd "$stage"
npx -y "$vercel_cli" deploy "${target[@]}" --yes --scope "$scope" --project "$project"
