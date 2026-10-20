# Third-party components

Akatsuki is licensed under the Akatsuki Source-Available License (ASAL-1.0, see [`LICENSE`](LICENSE)). The third-party components it uses are **not** covered by that licence. Each one stays under its own licence, listed below, and nothing in the ASAL-1.0 limits your rights under those licences.

This file lists the major components. The lockfiles (`pnpm-lock.yaml`, `Cargo.lock`) are the complete and authoritative list, including transitive dependencies. Licences below were read from the npm registry and crates.io on 2026-10-20 for the versions pinned in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §0.

When you add a dependency, add it here if it is major (a framework, a runtime library, anything vendored, any font or asset), and check that its licence allows use in a product that is free for personal use and commercially licensed. Copyleft licences (GPL, AGPL, LGPL for static linking, SSPL) need the owner's approval first.

## Backend platform

| Component | Version | Licence | How Akatsuki uses it |
|---|---|---|---|
| Convex backend (`get-convex/convex-backend`) | self-hosted Docker image | **FSL-1.1-Apache-2.0** (Functional Source License 1.1, Apache 2.0 future licence) | Production uses Convex Cloud, a hosted service under Convex's own terms. The self-hosted backend is used as a local development and test harness, and by self-hosters. The FSL forbids using it to offer a product that competes with Convex; each release becomes Apache-2.0 two years after it is published. If you run it, you must follow its licence. |
| `convex` (npm client and CLI) | 1.47.0 | Apache-2.0 | Client library, function runtime types, CLI |
| `convex` (Rust crate) | 0.10.4 | Apache-2.0 | Rust client for the sync worker and desktop |
| `convex-helpers` | 0.1.128 | Apache-2.0 | Custom query and mutation wrappers |
| `@convex-dev/better-auth` | 0.12.5 | Apache-2.0 | Better Auth component for Convex |
| `@convex-dev/rate-limiter` | 0.4.2 | Apache-2.0 | Rate limiting |
| `better-auth` | 1.6.33 | MIT | Authentication |
| `@better-auth/expo` | 1.6.33 | MIT | Authentication on mobile |

## AI

| Component | Version | Licence | Notes |
|---|---|---|---|
| `ai` (Vercel AI SDK) | 7.0.145 | Apache-2.0 | |
| `@ai-sdk/anthropic` | 4.0.83 | Apache-2.0 | |
| `@ai-sdk/openai` | 4.0.97 | Apache-2.0 | Also used for transcription |
| `@ai-sdk/google` | 4.0.100 | Apache-2.0 | |
| `@ai-sdk/gateway` | 4.0.118 | Apache-2.0 | |
| `@ai-sdk/openai-compatible` | 3.0.73 | Apache-2.0 | LM Studio, llama.cpp, vLLM and other OpenAI-style endpoints |
| `@openrouter/ai-sdk-provider` | 3.1.0 | Apache-2.0 | Published by OpenRouter |
| `ollama-ai-provider-v2` | 4.0.1 | Apache-2.0 | **Community provider**, not published by Vercel or Ollama (source: github.com/nordwestt/ollama-ai-provider-v2) |

AI models and APIs (OpenRouter, Anthropic, OpenAI, Google, Vercel AI Gateway, Ollama models) are services or model weights under their own terms and model licences. They are not distributed with Akatsuki. Users who bring their own provider or local model are responsible for following that provider's or model's terms.

## Servers and web

| Component | Version | Licence | Notes |
|---|---|---|---|
| `hono` | 4.13.13 | MIT | apps/api |
| `stripe` | 23.0.0 | MIT | apps/api billing |
| `jose` | 6.2.12 | MIT | JWT verification and minting |
| `aws4fetch` | latest at install | MIT | SES API signing from Convex |
| `react` | latest at install | MIT | web, landing, mobile |
| `@radix-ui/react-dialog`, `react-dropdown-menu`, `react-tooltip`, `react-visually-hidden` | 1.3.1, 2.2.1, 1.4.1, 1.2.13 | MIT | `packages/ui` dialogs, menus, tooltips, command palette |
| `@testing-library/react`, `user-event`, `dom`; `jsdom` | 16.3.3, 14.6.8, 10.4.2; 30.1.3 | MIT | `packages/ui` tests only |
| `vite` | latest at install | MIT | web |
| `next` | latest at install | MIT | landing |
| `typescript` | latest at install | Apache-2.0 | |
| `zod` | latest at install | MIT | |
| `turbo` | latest at install | MIT | monorepo task runner |

## Mobile

`apps/mobile` pins every version exactly (Expo SDK 58, D-330). Licences read from the npm registry on 2026-10-20.

| Component | Version | Licence | Notes |
|---|---|---|---|
| `expo` and the `expo-*` modules (router, secure-store, sqlite, notifications, font, haptics, crypto, web-browser, linking, localization, network, device, application, constants, splash-screen, status-bar, system-ui, dev-client, build-properties, document-picker, image-picker, file-system, audio) | 58.x (exact pins in `apps/mobile/package.json`) | MIT | |
| `react-native` | 0.88.0 | MIT | |
| `react`, `react-dom` | 19.3.0 | MIT | mobile pins the React version of its Expo SDK |
| `react-native-web` | 0.21.4 | MIT | web preview build only |
| `react-native-webview` | 14.0.1 | MIT | sandboxed message rendering |
| `react-native-gesture-handler` | 3.3.0 | MIT | swipe actions |
| `react-native-reanimated`, `react-native-worklets` | 4.7.1, 0.13.0 | MIT | |
| `react-native-safe-area-context`, `react-native-screens` | 5.10.1, 4.30.0 | MIT | |
| `@shopify/flash-list` | 2.3.2 | MIT | thread lists |
| `react-native-svg` | 15.16.0 | MIT | the shared stroke icons and the horizon (design spec §2.10, §2.11) |
| `expo-key-event` | 1.9.0 | MIT | hardware keyboard events (iPad); community module, github.com/tlow92/expo-key-event |

## Rust

| Crate | Version | Licence | Notes |
|---|---|---|---|
| `gpui` | 0.2.2 | Apache-2.0 | Desktop UI framework (from the Zed editor) |
| `async-imap` | 0.12.0 | MIT OR Apache-2.0 | IMAP client (`crates/providers::imap`); pulls `self_cell` (Apache-2.0 OR GPL-2.0-only, used under Apache-2.0) |
| `imap-proto` | 0.17.0 | MIT OR Apache-2.0 | IMAP response parser used by `async-imap` |
| `tokio-rustls` | 0.26.6 | MIT OR Apache-2.0 | TLS for IMAP and SMTP (rustls on aws-lc-rs) |
| `webpki-roots` | 1.0.10 | CDLA-Permissive-2.0 | Mozilla root certificates for IMAP/SMTP TLS |
| `roxmltree` | 0.20.0 | MIT OR Apache-2.0 | Mozilla autoconfig / ISPDB parsing |
| `rcgen` | 0.14.10 | MIT OR Apache-2.0 | Test-only: certificates for the fake IMAP/SMTP servers |
| `mail-parser` | 0.11.9 | Apache-2.0 OR MIT | |
| `ammonia` | 4.2.4 | MIT OR Apache-2.0 | HTML sanitiser |
| `jsonwebtoken` | 11.1.0 | MIT | |
| `chacha20poly1305` | 0.11.0 | Apache-2.0 OR MIT | Encryption of mailbox credentials at rest |
| `axum` | 0.8.9 | MIT | Sync worker HTTP server |
| `keyring` | 4.2.0 | MIT OR Apache-2.0 | OS keychain on desktop |
| `rusqlite` | 0.40.2 | MIT | Bundles SQLite, which is in the public domain |
| `tokio` | latest at build | MIT | |
| `reqwest` | latest at build | MIT OR Apache-2.0 | |
| `serde` | latest at build | MIT OR Apache-2.0 | |
| `jsonschema` | latest at build | MIT | Structured output validation in `crates/ai` |
| `sqlite-vec` | planned | MIT OR Apache-2.0 | On-device embeddings (desktop), not added yet |

## Fonts

| Font | Licence | Copyright |
|---|---|---|
| Instrument Serif | SIL Open Font License 1.1 (OFL-1.1) | Copyright 2022 The Instrument Serif Project Authors (github.com/Instrument/instrument-serif) |
| Geist | SIL Open Font License 1.1 (OFL-1.1) | Copyright (c) 2023 Vercel, in collaboration with basement.studio |
| Geist Mono | SIL Open Font License 1.1 (OFL-1.1) | Copyright 2024 The Geist Project Authors (github.com/vercel/geist-font) |

The OFL requires that the licence text travels with the font files. Wherever font files are vendored, keep an `OFL.txt` next to them. The OFL allows bundling fonts with software, including paid software, but the fonts may not be sold on their own.

## Icons and other assets

No third-party icon set or image asset is vendored yet. Record the source and licence here when one is added.

## Services (not distributed)

Akatsuki talks to these services. Their terms apply to the operator of each deployment, not to the code:

- Mail providers: Google (Gmail API, Cloud Pub/Sub) and Microsoft (Graph API), including the Google API Services User Data Policy (Limited Use) and Microsoft's equivalent terms.
- Convex Cloud, Stripe, Amazon SES, OpenRouter, OpenAI, Vercel.
