#!/usr/bin/env bash
# Subsets the brand fonts (packages/tokens/fonts/*.ttf, the workspace's only copy) to Latin + Latin-1
# and writes small woff2 files into public/fonts, which are committed. Run it when the token fonts
# change: `pnpm --filter @akatsuki/landing fonts`. Uses fonttools via uvx (pinned), no global installs.
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
src="$here/../../packages/tokens/fonts"
out="$here/public/fonts"
mkdir -p "$out"
# Basic Latin, Latin-1, general punctuation, arrows and the few symbols the UI uses (⌘ ⇧ ↵ ↑ ↓ ← → ↻ · — … ’ “ ” € •).
unicodes="U+0000-00FF,U+0131,U+0152-0153,U+02C6,U+02DA,U+02DC,U+2000-206F,U+20AC,U+2122,U+2190-2193,U+21B5,U+21BB,U+21E7,U+2318,U+2325,U+2303,U+2713,U+00D7"
for f in InstrumentSerif-Regular InstrumentSerif-Italic Geist-Regular Geist-Medium GeistMono-Regular GeistMono-Medium; do
  uvx --from 'fonttools[woff]==4.60.1' pyftsubset "$src/$f.ttf" \
    --unicodes="$unicodes" \
    --layout-features='kern,liga,calt,tnum,case,ss01' \
    --flavor=woff2 --no-hinting --desubroutinize \
    --output-file="$out/$f.woff2"
done
ls -la "$out"
