# @akatsuki/web — app.akatsuki.vc

The Akatsuki web mail client: a Vite + React 19 SPA on Convex (live queries, optimistic mutations), Better Auth
(`@convex-dev/better-auth`, crossDomain) and the sync worker (bodies, mailbox connect). Contract:
ARCHITECTURE §6.2 (auth), §5 (ops and undo), §11 (search), §14.3 (body rendering), §15 (errors), §16.1–16.2
(caches and optimistic updates), §10 (compose, send, undo, timers), §12 (AI), §13 (billing). Decisions D-260 to
D-269 (part 1) and D-280 to D-298 (part 2).

Part 1: sign-in, onboarding, the mail shell, lists, threads, search, keyboard, palette, optimistic actions with
undo. Part 2 plugs into `src/app/extensions.ts` (registered by `src/features/index.ts`): compose with autosave,
undo-send and send later, attachments, snippets, contacts, identities and signatures; reminders and follow-ups;
sender actions (unsubscribe, block); settings (accounts, identities, mail, appearance, shortcuts, notifications,
split inbox, rules, snippets, senders, AI, billing, devices and sessions, your data); AI (summaries, suggested
replies, write in my voice, rewrite, extract actions, search with AI, dictation, triage labels).

## Run it

```bash
pnpm --filter @akatsuki/web dev          # http://localhost:5173, against the local Convex harness
pnpm --filter @akatsuki/web build        # production bundle in dist/ (needs VITE_* at build time)
pnpm --filter @akatsuki/web build:mock   # the same bundle on the in-memory backend (dist-mock/)
```

`pnpm dev` needs no env file: in development the Convex URLs default to the local harness from docs/DEV.md
(`http://127.0.0.1:3410`, site `http://127.0.0.1:3411`) and the worker to `http://127.0.0.1:8787`. Start the
harness with `make convex-up && make convex-deploy`. Sign in with an email code: the harness logs it
(`cd convex && npx convex logs --env-file ../.env.local`). Port 5173 matters: it is the harness's `SITE_URL`
and a trusted origin.

Vite reads only `VITE_*` names, from the process environment or from env files in `apps/web/` (never the
repo root, whose `.env` holds the owner's keys).

| Variable | Meaning |
|---|---|
| `VITE_CONVEX_URL` | `https://<deployment>.convex.cloud` |
| `VITE_CONVEX_SITE_URL` | `https://<deployment>.convex.site` (Better Auth, JWT issuer) |
| `VITE_SYNC_URL` | `https://sync.akatsuki.vc` (bodies, connect, provider search; also the frame's image origin) |
| `VITE_API_URL` | `https://api.akatsuki.vc` (part 2: managed AI, billing) |
| `VITE_LOGIN_PROVIDERS` | Social buttons to show, e.g. `google,microsoft`; must match the deployment's `*_LOGIN_CLIENT_ID`s (D-261) |

## Layout

| Path | What |
|---|---|
| `src/main.tsx` | Picks the backend (Convex, or the mock in `mock`/`test` mode), sets the favicon from the tokens, renders. |
| `src/app/` | `App` (providers, auth gate, onboarding gate, routes), `router.tsx` (history API), `routes.ts` (URL scheme, pure), `extensions.ts` (part-2 slots). |
| `src/backend/` | `types.ts` (the data contract the UI uses), `convex.tsx` (production), `mock/` (in-memory, deterministic synthetic mailbox). |
| `src/auth/SignIn.tsx` | Email code, magic link, social buttons. |
| `src/onboarding/` | Connect Gmail / Outlook (worker OAuth) / IMAP with host autodetect (`imapDiscovery.ts`). |
| `src/mail/` | `MailApp` (shell, keyboard, palette, actions), `Sidebar`, `ThreadList` (virtualized), `ThreadView`, `MessageFrame` + `bodyHtml.ts` (sandboxed rendering), `bodyCache.ts`, `useMailActions.ts` (optimistic apply, undo, outbox, failure toasts), `undoSnapshots.ts`, `SearchBar` + `searchSuggest.ts`, `TimePicker` + `snoozeTimes.ts`, `ListPicker`, `HelpDialog`, `Banners`, `EmptyStates`. |
| `src/lib/` | ids (UUIDv7, deviceId), theme, outbox, IndexedDB key-value store, formatting. |
| `src/features/index.ts` | Registers the part-2 extensions once at start-up. |
| `src/compose/` | `Composer` (overlay), `RecipientField` (contacts autocomplete), `model.ts` (local draft, reply/forward builders, `DraftContent`), `saver.ts` (autosave, §10.1), `sendFlow.ts` (send, undo, send later, follow-ups), `store.ts` (open draft, pending sends), `text.ts` (text ↔ HTML, addresses, snippet variables), `extension.tsx` (keys, palette, scheduled messages, reply bar, `/compose`). |
| `src/reminders/` | Reminder picker (Shift H, natural-language times, note, "only if nobody replies") and the open thread's active reminder. |
| `src/senders/` | Unsubscribe (Ctrl/⌘ U), block sender (D-286), read receipts under your own messages. |
| `src/ai/` | `useAi` (settings, status, egress, managed vs BYO `runAi`), `module.ts` (lazy AI chunk), `keyStore.ts` (BYO keys, D-285), `cache.ts`, `dictation.ts`, `thread.ts` (message text for AI and quotes), `extension.tsx` (thread AI panel, search with AI, BYO triage), `Egress.tsx`, `TriageChip.tsx`. |
| `src/settings/` | `SettingsApp` (lazy chunk), `pages.ts`, `sections/*`, `ruleModel.ts` (rule language editor model), `aiTest.ts` (Test connection). |
| `src/styles/app.css`, `src/styles/features.css` | Token variables only (tested). |

## How it behaves

- **Keyboard first.** Every key comes from `packages/shortcuts` (superhuman or gmail keymap from `prefs.keymap`)
  through one matcher. Every action with a handler is also in the command palette (`Ctrl/⌘ K`), plus accounts,
  labels, splits, theme, density and sign-out. `?` lists all shortcuts.
- **Instant actions.** `mail.apply` runs with `applyOptimisticUpdate` from `@akatsuki/mail-model`; a gesture is one
  `clientOpId` per account and action (archive differs per provider), chunked at 500 threads. Undo (`z` or the
  toast, 10 s) calls `mail.undo` with an optimistic update that also puts removed rows back (D-265); a snooze is
  undone with `mail.unsnooze`. Calls go through the durable IndexedDB outbox and are replayed on start (§16.1).
  Op failures from `ops.failures` show once per opId with Retry and Dismiss (§5.7).
- **Bodies.** Streamed once per thread from `GET /v1/threads/{id}/bodies`, cached in memory and IndexedDB
  (250 MB LRU, keyed by message and checked against the newest `sanitizerVersion`; memory only when
  `cacheBodiesOnDevice` is off). The next two conversations are prefetched (query and bodies).
- **Rendering (§14.3).** `<iframe sandbox="allow-popups allow-popups-to-escape-sandbox" srcdoc>` with the meta CSP
  first; never scripts or same-origin. Remote images stay deferred until "Show images" (or the sender or global
  preference), and only `/v1/img/` URLs on the worker origin load. `cid:` parts are fetched with the token and
  injected as `data:image/*`. A defence-in-depth pass strips anything executable. The frame height is measured
  off-frame in an inert shadow root (D-267).
- **Search.** The §11.1 grammar with suggestions (operators, contacts, labels, splits), parse errors inline,
  `nowAt` rounded to the minute; "Search the full mailbox" runs provider search on the worker.
- **No layout shift.** Fixed row heights from the density tokens, fixed panes, skeletons with the final size,
  the theme applied before first paint (`public/theme-boot.js`).

## Part 2 in practice

- **Compose** (`c`, `r`, `a`, `f`; reply from the list or an open thread). Recipients autocomplete from
  `contacts.search`; paste lists; `;shortcut` + Tab or Ctrl/⌘ ; inserts a snippet with `{{first_name}}`-style
  variables. Every edit is saved on the device at once and to the provider draft every 2 s; a conflict with
  another device offers to load that copy or keep this one. Attachments upload to the worker's encrypted spool
  (25 MB per message), by picker, drag and drop or paste. Ctrl/⌘ Enter sends with Undo (`z`), Ctrl/⌘ Shift L
  sends later; "Show scheduled messages" in the palette changes or cancels them.
- **AI** is off until consent. Settings → AI adds providers (Akatsuki AI on Pro, OpenRouter, Anthropic, OpenAI,
  Google, AI Gateway, Ollama, any OpenAI-compatible endpoint), keys stay in this browser (memory, or encrypted
  with "Remember on this browser"), and each feature routes to a provider with the egress label under it.
  Managed requests go to apps/api (`VITE_API_URL`); BYO requests go from the browser to the provider.
- **Billing**: Settings → Billing and the checkout onboarding step open Stripe through apps/api.

## Tests

```bash
pnpm --filter @akatsuki/web test        # Vitest + Testing Library on jsdom (mock backend; no network)
pnpm --filter @akatsuki/web test:e2e    # Playwright: mock-mode production bundle under the production CSP
AKATSUKI_E2E_LOCAL=1 pnpm --filter @akatsuki/web test:e2e   # against the local Convex harness
```

Part 2 adds `test/compose.test.ts` (text, drafts, autosave, send and undo, natural-language times),
`test/ai.test.ts` (key store, managed vs BYO path, status, egress, cache, Test connection),
`test/settings.test.ts` (rule model, blocks, unsubscribe plan, billing copy, export) and
`test/features.test.tsx` (the app: compose and undo with `z`, reply with snippets, autosave, reminders, settings,
AI summary), plus `e2e/mock/features.spec.ts` (compose, undo-send, send later, settings, AI, billing).

The local suite signs in with a real email code (read from the function logs through the documented
`convex logs --env-file ../.env.local`), goes through onboarding, checks the worker error copy, round-trips a
preference and signs out. It needs the harness up and deployed and port 5173 free.

`e2e/local/mail.spec.ts` runs only under `make e2e-web` (repository root, `scripts/e2e-local.mjs --web`), which
starts a real sync worker and fake IMAP/SMTP servers and passes `AKATSUKI_E2E_SYNC_URL` and
`AKATSUKI_E2E_FAKE_MAIL_STATE`: sign-in, IMAP onboarding, the synced inbox, a sanitized body, archive with `e`
(checked on the fake server), undo with `z`, mailbox removal. Without those variables it is skipped.
