# Akatsuki — Build Brief (source of truth)

Every agent working in this repo reads this file first. If something here conflicts with your instructions, this file wins; if you must deviate, write the deviation into `docs/DECISIONS.md` with a reason.

## What we are building

**Akatsuki** (暁, "dawn") is a fast, keyboard-first email client in the spirit of Superhuman. It works with Gmail / Google Workspace, Outlook / Microsoft 365, and any IMAP/SMTP host. Domain: **akatsuki.vc**.

- Public wording: **"open source, free for personal use"** (owner, D-133). Always put the commercial-use caveat nearby: commercial use, including use by any organisation, needs written permission. The licence keeps its name, the Akatsuki Source-Available License (ASAL-1.0).
- Free to read, run and self-host for **non-commercial** use. **Commercial use is reserved to the owner** and needs written permission (see `LICENSE`).
- Paid hosted plan: **Akatsuki Pro, $20 / month**, 14-day trial, **no free hosted tier**.
- Owner / copyright holder: Sami Hindi (GitHub: FujiwaraChoki, email sami@samihindi.com). Do not guess pronouns for anyone.

## Decisions already made (do not reopen)

| # | Decision |
|---|---|
| 1 | Auth: **Convex + Better Auth** (`@convex-dev/better-auth`; pin exact tested versions). |
| 2 | Mail data: **hybrid**. Convex stores metadata (threads, message headers, snippets, labels, flags, state). **Message bodies and attachments are never stored in Convex**: fetched on demand through the sync worker, cached on the device. Hosted Convex never holds message text. |
| 3 | Sync worker: **Rust**, always-on, shares crates with desktop. |
| 4 | Desktop HTML email: sanitize in Rust, render the common 90% natively in GPUI; "Open original" shows the sanitized message in a sandboxed webview popup/separate window. |
| 5 | Build order: backend + web first, desktop spike in parallel, desktop alpha next. **We are building everything**: web, landing, Convex backend, sync worker, desktop, mobile, infra, docs. |
| 6 | License: custom licence (ASAL-1.0); open source, free for personal use, commercial use reserved (see below). |
| 7 | No free hosted tier. 14-day Pro trial. |
| 8 | Read receipts: opt-in only, with a clear warning in the UI (Apple Mail Privacy Protection makes them unreliable). |
| 9 | Mobile: **native mobile app before launch**. Stack chosen: **Expo (React Native) + TypeScript**, Convex React client + Better Auth Expo plugin. (GPUI does not target phones.) |
| 10 | Company / Google Cloud org / OAuth verification: not set up yet; the user wants it created in Chrome later. Code must work with self-supplied OAuth clients and not depend on verification. |
| 11 | Name locked: **Akatsuki**. Domain `akatsuki.vc` (DNS on Cloudflare, apex currently points at Vercel). |

## Hostnames (plan)

- `akatsuki.vc` — landing site (Next.js, Vercel)
- `app.akatsuki.vc` — web mail client (Vite SPA, Vercel)
- `docs.akatsuki.vc` — docs (can live in landing app under `/docs` first)
- `api.akatsuki.vc` — Convex custom domain / HTTP actions (optional later)
- `sync.akatsuki.vc` — Rust sync worker public HTTP/WebSocket (webhooks from Gmail Pub/Sub and Microsoft Graph land here)
- Transactional email (magic links, receipts, trial reminders): **AWS SES**, sender `Akatsuki <no-reply@akatsuki.vc>`.

## Design theme (from the user's existing Akatsuki todo app — "crimson dawn minimalism")

Source: reference clones live in `/tmp/claude-1000/-home-loki-Work-akatsuki/6d065de0-01ce-490f-a323-c7d9560709a5/scratchpad/ref/` (`akatsuki-web/DESIGN_SYSTEM.md`, `akatsuki-web/app/globals.css`, `akatsuki-ios/Akatsuki/Theme/ColorScheme.swift`). Read them. Summary:

| Token | Light | Dark |
|---|---|---|
| accent (crimson) | `#C1272D` | `#800000` (deep crimson) |
| background | `#FFFFFF` | `#0A0A0A` |
| foreground | `#171717` | `#EDEDED` |

- Foreground opacity scale: text 100 / secondary 80 / muted 60 / hover border 40 / border 20 / subtle border 10 %.
- Accent opacity scale: solid 100 / gradient core 70 / mid 50–60 / glow 30–40 / tint 15 / wash 8 %.
- Typeface: **Instrument Serif** (single weight 400; no bold; emphasise by size and colour). Font files exist in `ref/akatsuki-web/app/fonts/`. For dense mail UI (lists, metadata, code) pair it with **Geist** / **Geist Mono** (font files in `ref/Siberia/assets/fonts/`), because a serif at 13px in a 10,000-row list is not readable. Instrument Serif is for brand, headings, subjects of opened threads, empty states, onboarding and marketing.
- Labels: uppercase, wide tracking (0.3em on marketing, 0.12em in app chrome).
- Atmosphere: blurred crimson gradient orbs that float and morph slowly (landing, onboarding, empty states, command palette backdrop). Subtle crimson glow on primary actions. Honour `prefers-reduced-motion`.
- Single accent colour. Dark mode uses the *deep* crimson and `#0A0A0A`.
- `packages/tokens` is the single source: one JSON → CSS variables (web, landing), TypeScript constants (mobile), Rust constants (GPUI). Nothing else hard-codes the palette.

## Monorepo layout

```
akatsuki/
├─ apps/
│  ├─ desktop/       Rust · GPUI (gpui = "0.2.2" from crates.io, see ref/Siberia for a working setup)
│  ├─ web/           Vite + React + TypeScript SPA (the mail client)
│  ├─ landing/       Next.js (App Router) marketing, pricing, docs, legal
│  ├─ mobile/        Expo (React Native) + TypeScript
│  ├─ api/           Hono (Node) container on the Hetzner server: stateless AI + transcription + Stripe/Pro webhook endpoints (no message text is ever persisted or logged)
│  └─ sync/          Rust sync worker (binary crate)
├─ convex/           Convex functions: schema, auth, mail API, ops outbox, billing, AI (ee/)
│  └─ ee/            Pro-only code (managed AI, billing); same license, separate dir
├─ crates/
│  ├─ core/          mail domain model, sync engine, local store (SQLite)
│  ├─ providers/     gmail · graph · imap/smtp adapters behind one trait
│  ├─ mime/          parse, sanitize (HTML → safe tree), compose
│  ├─ search/        SQLite FTS5 index + query operator parser
│  ├─ ai/            thin Rust AI client for the desktop app (OpenAI-compatible + Anthropic native + Ollama), uses packages/prompts
│  └─ convex-api/    typed bindings to Convex functions used by Rust
├─ packages/
│  ├─ tokens/        design tokens → CSS / TS / Rust
│  ├─ shortcuts/     one key map (JSON) read by TS and Rust
│  ├─ ai/            Vercel AI SDK provider registry, model routing, feature functions (TS)
│  ├─ prompts/       prompt pack + JSON-schema outputs shared by TS and Rust (plain files)
│  ├─ ui/            shared React components (web + landing)
│  └─ config/        tsconfig / lint / format presets
├─ infra/            Dockerfiles, docker-compose (self-host + local dev), CI, deploy scripts
├─ docs/             BRIEF.md, ARCHITECTURE.md, DECISIONS.md, guides
├─ LICENSE  README.md  CONTRIBUTING.md  SECURITY.md
├─ Cargo.toml  package.json  pnpm-workspace.yaml  turbo.json
└─ .env.example  (never commit real secrets)
```

## Architecture contract (summary; `docs/ARCHITECTURE.md` is the detailed version)

- **Convex** is the control plane and the live view: accounts, labels, threads, message headers/snippets/flags, `ops` outbox, snoozes, reminders, scheduled sends, splits, rules, snippets, settings, AI results, billing state.
- **Sync worker (Rust)** holds provider connections (IMAP IDLE, Gmail watch via Pub/Sub, Graph webhooks), pulls changes into Convex, applies `ops` to providers, fires timers (snooze, reminder, scheduled send), serves message bodies and attachments on demand over an authenticated HTTP API, and is the **only** reader of provider OAuth tokens (encrypted at rest).
- **Clients** (web, desktop, mobile) subscribe to Convex live queries, apply optimistic updates, write mutations, and fetch bodies from the worker with the user's session token. Desktop keeps a SQLite cache + FTS5 index of bodies; web keeps IndexedDB.
- **One action travels:** client optimistic update → Convex mutation (updates thread + inserts `ops` row) → worker applies to provider → provider push comes back → worker reconciles → op marked done, or failed (client rolls back with a toast).
- Login identity and connected mail accounts are separate. Mail-account OAuth tokens never reach any client.
- Initial sync window: last 90 days, batch writes, lazy backfill. Never write bodies to Convex.
- Convex is the **only** datastore. No Postgres.

## Licensing text to implement (`LICENSE`)

Custom "Akatsuki Source-Available License" (ASAL-1.0), plain English, short:
1. Copyright © 2026 Sami Hindi. All rights reserved except as granted.
2. Grant: view, copy, modify and run the software for **personal, non-commercial** purposes, including self-hosting for yourself and household/friends at no charge.
3. **Commercial use is not granted.** Using it in or for a business, selling, sublicensing, offering it as a hosted service, bundling it into a paid product, or using it to provide a service for a fee requires prior written permission from the copyright holder. All commercial rights are reserved by the copyright holder. Contact: sami@samihindi.com.
4. Contributions: contributor grants the copyright holder a perpetual, worldwide, irrevocable licence (including to relicense commercially) to the contribution.
5. Third-party components keep their own licences (list in `THIRD_PARTY.md`); the Convex backend is FSL-1.1-Apache-2.0.
6. No warranty. Termination on breach. Trademark: "Akatsuki" and the logo are not licensed.
Add a header note in the README that this is **not** an OSI-approved licence (it reserves commercial use) and that a lawyer should review it before commercial launch.

## Toolchain on this machine (verified 2026-10-20)

Rust 1.98.1 (edition 2024 ok), Node 26.7, pnpm 12.11, bun 1.4, Docker 29.7, aws-cli 2.34 (configured, region eu-north-1), wrangler 4.147 (logged in to Cloudflare), gh (logged in as FujiwaraChoki). `just` is missing — use `package.json` scripts + a `Makefile`, no `just`. Linux = Omarchy / Hyprland (Wayland): the desktop app must build and run here.

## Reference code (read-only, user's own earlier work; reuse ideas and code freely)

In `/tmp/claude-1000/-home-loki-Work-akatsuki/6d065de0-01ce-490f-a323-c7d9560709a5/scratchpad/ref/`:
- `tegami/` — Electron + Next Gmail client: OAuth loopback+PKCE, history-id sync, Gmail parse/compose/labels, command palette, shortcuts, rich editor, virtualized list, optimistic actions. Mine it for UX and protocol details.
- `Siberia/` — Rust + GPUI (`gpui = "0.2.2"`) word processor: working GPUI setup, assets (fonts, SVG icons), Linux + macOS builds, text system usage.
- `multimark/` — Convex + React app (schema, functions layout).
- `akatsuki-web/`, `akatsuki-ios/` — theme source.

## Secrets rules

- Never print, log, commit or paste secret values. Put them in gitignored `.env.local` files; document names only in `.env.example`.
- AWS: create a least-privilege IAM user for SES sending; do not use root keys in app config.
- All provider tokens encrypted at rest (XChaCha20-Poly1305 or AES-GCM with a key from env).

## Quality bar

- Everything compiles, lints, typechecks and has tests that pass locally. No placeholder implementations presented as finished; if something cannot be completed (needs external credentials, a device, a paid account), implement everything up to that boundary, mark it clearly in `docs/STATUS.md`, and say what is missing.
- Email HTML is hostile input: sanitize before render everywhere; remote images proxied/blocked by default; no scripts, no forms, no `javascript:` URLs.
- Keyboard-first: every action reachable by key and by the command palette; shortcuts defined once in `packages/shortcuts`.
- Accessibility: focus states, reduced-motion, labelled controls.
- Plain, direct copy. No emoji as UI decoration.

## Secrets and env (HARD RULE)

- `/home/loki/Work/akatsuki/.env` holds the owner's **OpenRouter** and **Fal** API keys (names: `OPENROUTER_API_KEY`, `FAL_AI_API_KEY`). **Never open, read, cat, grep, source, print, diff or copy it. Never dump the process environment.** Do not scan it in secret audits. It is gitignored.
- Code reads keys at runtime from the environment (`OPENROUTER_API_KEY`, and `OPENAI_API_KEY` for transcription. The `.env` also contains `FAL_AI_API_KEY`, unused. Names were confirmed by the owner in chat). `.env.example` documents names only.
- Tests must not need real keys: mock providers, and use Ollama or recorded fixtures for any live check, and only when the owner has said a key may be used.

## AI layer (decided 2026-10-20)

- **Vercel AI SDK 7** (`ai@7.x`). Verified published providers: `@ai-sdk/anthropic`, `@ai-sdk/openai`, `@ai-sdk/google`, `@ai-sdk/gateway`, `@openrouter/ai-sdk-provider@3`, `ollama-ai-provider-v2@4` (community), `@ai-sdk/openai-compatible` (LM Studio, llama.cpp, vLLM, any OpenAI-style endpoint), `@ai-sdk/fal`.
- **Default model: Claude Sonnet 5.5** for most features (OpenRouter slug `anthropic/claude-sonnet-5.5`, direct id `claude-sonnet-5-5`). Use **Haiku 5.5** (`anthropic/claude-haiku-5.5`) for high-volume cheap classification (auto-triage, labelling). Use a heavier model (`anthropic/claude-opus-5.5` or `anthropic/claude-fable-5.1`) only where a feature measurably needs it. Task -> tier mapping is data in `packages/ai`, not scattered constants.
- **User choice, per feature:** the user picks provider, model, base URL and key for each AI feature (or one default for all). Providers: Akatsuki-managed (Pro), OpenRouter, Anthropic, OpenAI, Google, AI Gateway, Ollama (local), any OpenAI-compatible endpoint. Include "Test connection", and clear labelling of what leaves the device. Local models are first-class: nothing leaves the machine when Ollama is chosen.
- **Hosted (Pro) AI** runs in `apps/api` (stateless streaming endpoint). The client already holds the message text, sends only what the feature needs with the user's session token, the endpoint calls the model with the managed key and streams back. **Nothing is stored or logged**: no request bodies in logs, OpenRouter requests set data-collection deny / zero-data-retention routing (verify the exact provider-routing params in current OpenRouter docs), no training. AI outputs are cached on-device only unless the owner decides otherwise. Respect the Google API Services User Data Policy (Limited Use) and Microsoft equivalents: user-facing features only, explicit consent, no model training.
- **AI SDK is TypeScript-only.** Web, mobile and `apps/api` use it. The Rust desktop app uses `crates/ai` (reqwest, streaming, tool-less structured output) against the same prompt pack in `packages/prompts` and the same provider settings schema, so behaviour matches.
- **Fal is not used.** (`FAL_AI_API_KEY` exists in the owner's `.env`; ignore it, do not wire it in.)
- **Transcription / dictation:** OpenAI API through the AI SDK (`@ai-sdk/openai`, `experimental_transcribe` or the current equivalent; verify current model ids such as `gpt-4o-transcribe` / `whisper-1` in the installed package docs). User-selectable alternative: a local Whisper endpoint (OpenAI-compatible, e.g. whisper.cpp server or Ollama-adjacent) so audio can stay on the device. Key name: `OPENAI_API_KEY` (owner-created, lives in gitignored `.env.credentials`).
- **Embeddings / semantic search:** on-device only in v1 (desktop, sqlite-vec or equivalent, with a local or user-chosen embedding model). No embeddings in Convex.

## Hosting (decided 2026-10-20)

- **Convex: Convex Cloud** (hosted by Convex) for production. A local self-hosted Convex in Docker is allowed only as a dev/test harness so agents can run and verify without cloud credentials.
- **Everything else server-side runs on the owner's Hetzner server** (`ssh hetzner`, Ubuntu 24.04, 12 cores / 64 GB, Docker 28 + Compose v2, host nginx on 80/443). It is shared with many other live services (supoclip, skript, nyan, gitea, paperless, signoz, an existing mail server on 25/465/587/993, and more): **never stop, restart, edit or remove anything that is not Akatsuki's; use unique container names (`akatsuki-*`), a unique localhost port, and a new nginx vhost; check ports before binding.** Deployable units: `apps/sync` (Rust worker) and `apps/api` (Hono). TLS via the existing host nginx / certbot. Hostnames: `sync.akatsuki.vc`, `api.akatsuki.vc` (A records at Cloudflare, DNS-only).
- **Landing and web app: Vercel** (static / Next), `akatsuki.vc` and `app.akatsuki.vc`.
- **No remote action without the owner's go-ahead in the current conversation**: deploying to Hetzner, changing DNS beyond the records needed here, purchasing anything.
