# Akatsuki

**Akatsuki is open source, free for personal use.** You can read, run and self-host it for personal, non-commercial use. Commercial use of any kind, including use by any organisation, needs written permission from the copyright holder (sami@samihindi.com). The licence, the Akatsuki Source-Available License (ASAL-1.0) in [`LICENSE`](LICENSE), is not an OSI-approved licence, because it reserves commercial use. It has not yet been reviewed by a lawyer, and it must be reviewed before any commercial launch.

---

Akatsuki (暁, "dawn") is a fast, keyboard-first email client. It works with Gmail and Google Workspace, Outlook and Microsoft 365, and any IMAP/SMTP host, on the web, desktop and phone.

Email should take less of your day. Akatsuki keeps your hands on the keyboard, keeps the list fast at any size, and keeps your mail where it already lives: with your provider, and on your devices.

Website: [akatsuki.vc](https://akatsuki.vc)

## Status

**Pre-alpha. Nothing is ready to use yet.** The architecture is decided and the components are being built. The table in [`docs/STATUS.md`](docs/STATUS.md) shows, for each component, whether it is not started, a skeleton, working, or blocked on credentials. Everything below describes what Akatsuki is designed to do, not what you can use today.

## What it does

- **Every mailbox in one place.** Gmail, Outlook and IMAP accounts side by side, in one unified inbox or one at a time.
- **Keyboard first.** Every action has a shortcut and is in the command palette. One key map is shared by every app.
- **Fast lists.** The desktop app keeps a full local copy of your mail metadata, so lists open in under a millisecond and work offline.
- **Instant actions.** Archive, label, star and move happen on screen at once and sync in the background. If the provider refuses, the thread comes back and you are told why.
- **Split inbox.** Sort incoming mail into splits with simple rules, optionally helped by AI triage.
- **Snooze, reminders, send later and undo send.** Timers run on the server, so a closed laptop still sends on time.
- **Search with operators.** `from:`, `to:`, `has:attachment`, `before:`, `is:unread` and more, with your provider's search as a fallback for older mail.
- **Safe by default.** Email HTML is treated as hostile: it is sanitised before it is shown, scripts never run, and remote images are blocked until you allow them, then loaded through a proxy that hides your IP address.
- **Read receipts only if you ask.** Off by default, with a clear note on how unreliable they are.

## AI

AI in Akatsuki is optional, and you decide where it runs.

- **Bring any provider.** OpenRouter, Anthropic, OpenAI, Google, Vercel AI Gateway, any OpenAI-compatible endpoint (LM Studio, llama.cpp, vLLM), or **Ollama on your own machine**. You can pick a provider and model per feature, or one for everything.
- **Local means local.** With Ollama or another endpoint on your machine, nothing leaves it, triage and dictation included. A model server on your local network keeps data on that network.
- **Pro includes managed AI.** No keys to manage. Requests go through Akatsuki's stateless AI service with a monthly usage allowance. When the allowance runs out you can switch to your own key or a local model.
- **Features:** thread summaries, three instant replies, write in your voice, rewrite (shorter, friendlier, formal, fix), action items and deadlines, natural-language search, auto-triage, and dictation.

Privacy guarantees, enforced in code:

1. No AI feature runs until you have given consent. The consent text names the provider and what leaves the device.
2. Only what the feature needs is sent: one thread, or for triage just headers and snippets. Triage never sends message bodies.
3. The managed service never stores or logs your requests or the responses. It keeps usage counts only.
4. Managed text features (summaries, replies, rewrite, triage, search and the rest) are routed through OpenRouter with zero data retention and data collection denied. Your mail is never used to train models.
   Managed dictation is the exception: your audio goes to OpenAI's transcription API under OpenAI's standard API data policy. That policy excludes it from model training, but OpenAI may keep it for a limited time for abuse monitoring. Akatsuki has not confirmed zero data retention with OpenAI. To keep audio on your machine, choose a local Whisper endpoint for dictation.
5. AI results stay on your device. The only exceptions are triage categories, and your writing-style profile if you choose to sync it.
6. Your own API keys stay on the device you typed them into. They never reach Akatsuki's servers.
7. Every AI action shows where its data goes, for that feature and provider: "Stays on this device" (a local model on this machine), "Stays on your local network" (a model server on your LAN, named by host), "Leaves this device" (your own cloud provider, named by host, with what is sent), "Sent to Akatsuki for processing, not stored" (managed text features), or, for managed dictation, "Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring".

## How it fits together

- **Convex** is the control plane: accounts, labels, thread metadata, the action queue, timers, settings and billing. It never stores message bodies or attachments. It does keep message headers (sender, recipients, subject, date) and a short snippet of each message (up to 200 characters), so lists and search work without the worker.
- **The sync worker** (Rust) is the only component that talks to mail providers and the only holder of mailbox credentials, which are encrypted at rest. It serves message bodies on demand and never stores them.
- **The API service** (Hono) is stateless. It runs managed AI, transcription and billing webhooks.
- **Clients** (web, desktop, mobile) read live data from Convex, fetch bodies from the worker, and cache bodies on the device.

The full contract is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), and the reasons behind it are in [`docs/DECISIONS.md`](docs/DECISIONS.md).

## Repository map

```
akatsuki/
├─ apps/
│  ├─ web/           Mail client for the browser (Vite, React, TypeScript)
│  ├─ desktop/       Desktop app (Rust, GPUI)
│  ├─ mobile/        Phone app (Expo, React Native, TypeScript)
│  ├─ landing/       Marketing site, pricing, docs, legal (Next.js)
│  ├─ api/           Stateless AI, transcription and billing service (Hono, Node)
│  └─ sync/          Sync worker (Rust)
├─ convex/           Convex schema and functions
│  └─ ee/            Pro-only code (billing, managed AI); same licence
├─ crates/
│  ├─ core/          Mail domain model, sync engine, local store
│  ├─ providers/     Gmail, Microsoft Graph and IMAP/SMTP adapters
│  ├─ mime/          Parse, sanitise and compose messages
│  ├─ search/        Query parser and SQLite full-text index
│  ├─ ai/            AI client for the desktop app
│  ├─ convex-api/    Typed Convex bindings for Rust
│  ├─ tokens/        Design tokens as Rust constants (generated)
│  └─ shortcuts/     The key map as Rust constants (generated)
├─ packages/
│  ├─ mail-model/    Shared mail logic: label keys, actions, rules, search grammar
│  ├─ wire/          Wire types, error codes, AI stream parser
│  ├─ ai/            Provider registry and model routing (Vercel AI SDK)
│  ├─ prompts/       Prompt pack shared by TypeScript and Rust
│  ├─ tokens/        Design tokens for CSS, TypeScript and Rust
│  ├─ shortcuts/     The one key map
│  ├─ ui/            Shared React components
│  └─ config/        TypeScript, lint and format presets
├─ infra/            Dockerfiles, Compose files, CI, deploy scripts
└─ docs/             Brief, architecture, decisions, guides, status
```

[`docs/STATUS.md`](docs/STATUS.md) shows how far each entry has got.

## Quickstart

Setup, local development and self-hosting instructions are in [`docs/DEV.md`](docs/DEV.md).

Secrets never go in git. Copy `.env.example` to `.env.local` and fill in your own values; `.env.example` documents the variable names only.

Self-hosting needs your own OAuth clients for Google and Microsoft. Akatsuki does not depend on its own verified OAuth apps.

## Pricing

| Plan | Price | What you get |
|---|---|---|
| Self-hosted | Free, for personal non-commercial use | Everything in this repository, run on your own hardware. Bring your own AI provider or run models locally. |
| Akatsuki Pro | $20 / month | Hosted sync for all your accounts, every app, and managed AI included. 14-day free trial; a card is needed to start it. |

There is no free hosted plan. Using Akatsuki for a business, self-hosted or otherwise, needs a commercial licence: write to sami@samihindi.com.

## Licence in brief

| You may | You may not | Ask first |
|---|---|---|
| Read, run and change the code for yourself | Use it in or for a business | Any commercial use |
| Self-host it for yourself, your household and friends, free of charge | Sell it, or sell access to it | Use by any organisation, including non-profits |
| Publish your changes as source, free, under the same licence | Offer it as a hosted service | Using it for business email |
| Compile it and share the builds with anyone, free, under the same licence | Bundle it into a paid product, or use it to provide a paid service | |
| Contribute back | | |
| | Use the Akatsuki name or logo for your copy | |

The full terms are in [`LICENSE`](LICENSE). Third-party components keep their own licences; see [`THIRD_PARTY.md`](THIRD_PARTY.md). The Convex backend is licensed FSL-1.1-Apache-2.0.

## Contributing and security

- How to contribute, and the rights you grant when you do: [`CONTRIBUTING.md`](CONTRIBUTING.md).
- How to report a vulnerability privately: [`SECURITY.md`](SECURITY.md).
- How we treat each other: [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

## Copyright

Copyright (c) 2026 Sami Hindi. "Akatsuki" and the Akatsuki logo are not licensed for use by others.
