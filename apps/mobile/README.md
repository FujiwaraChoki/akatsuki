# Akatsuki mobile

The iOS, iPadOS and Android app: Expo SDK 58 (React Native 0.88, React 19.3), Expo Router, Convex live queries with
Better Auth (`@better-auth/expo` + `@convex-dev/better-auth`), message bodies from the sync worker. Contract:
`docs/ARCHITECTURE.md` §6.4 (auth), §16.2 (lists and caches), §14.3 (sandboxed rendering), §14.2 (push privacy).
Decisions D-330 to D-339.

## What is in it

- Sign-in: magic link (`akatsuki://auth/callback`), email code, Google / Microsoft when the server has them.
- Split inbox (Inbox, each split, Other) and every mailbox view, swipe right to archive, left to snooze, far left to
  trash, with haptics; instant optimistic actions and instant Undo (10 s, also `z`); durable outbox in SQLite.
- Thread view in a WebView with JavaScript off, the §14.3 CSP, links opened outside the app, remote images blocked
  until allowed, inline images and attachments through the worker.
- Compose, reply, reply all, forward with 2 s autosave to the provider draft, Send with undo, Send later.
- Snooze and "Remind me if no reply" pickers in the user's time zone.
- Search with the shared grammar (chips, errors), Convex results, then "Search the full mailbox" via the worker.
- Settings: plan status only (the app sells nothing, D-333), mailboxes (Gmail, Outlook, IMAP), notifications
  (`private` by default; `full` explains that sender and subject pass through Expo, Apple and Google), theme,
  keymap, undo-send delay, remote images, on-device body cache, AI providers (Akatsuki AI, BYO key kept only in
  SecureStore, Ollama or an OpenAI-compatible server on the LAN, each with its "where data goes" label and Test
  connection), devices, account deletion.
- AI under an open conversation (D-408): Summarize and Suggest replies (menu, `Shift+S` / `Shift+R`, palette),
  through the same route as web: Akatsuki AI via apps/api, or the user's own provider straight from the device with
  the key from SecureStore. A suggested reply opens the composer with it. Results stay on the screen. Not on mobile
  yet: Write with AI, rewrite, dictation, Search with AI and triage.
- Hardware keyboards (iPad first): the shared key map from `packages/shortcuts`, `⌘K` command palette with every
  implemented action plus theme, density, settings pages, mailboxes and sign out.

## Run it

Everything public comes from `EXPO_PUBLIC_*` variables at build time (names in `.env.example`; never put secrets
there). Against the local Convex harness (`make convex-up`, `docs/DEV.md`), from a phone or emulator on the same
network use the machine's LAN address:

```bash
cd apps/mobile
EXPO_PUBLIC_CONVEX_URL=http://192.168.1.10:3410 \
EXPO_PUBLIC_CONVEX_SITE_URL=http://192.168.1.10:3411 \
EXPO_PUBLIC_SYNC_URL=http://192.168.1.10:8787 \
EXPO_PUBLIC_API_URL=http://192.168.1.10:8788 \
pnpm start            # Metro for a development build (expo-dev-client)
pnpm android          # build and install a development build locally (Android SDK required)
pnpm ios              # macOS + Xcode only
```

The app uses native modules that Expo Go does not ship (`expo-key-event`), so it runs as a development build.
Without the variables the app shows which ones are missing instead of starting.

## Checks

```bash
pnpm --filter @akatsuki/mobile typecheck
pnpm --filter @akatsuki/mobile test          # view-models (src/vm), theme mapping, instant undo
pnpm exec biome check apps/mobile            # from the repo root
pnpm --filter @akatsuki/mobile export:android
pnpm --filter @akatsuki/mobile export:web    # web preview of the same app (development aid, not apps/web)
```

## Local iOS simulator build (macOS)

```sh
cd apps/mobile && npx expo prebuild --platform ios --clean
xcodebuild -workspace ios/Akatsuki.xcworkspace -scheme Akatsuki -configuration Debug \
  -destination 'generic/platform=iOS Simulator' CODE_SIGN_IDENTITY=- DEVELOPMENT_TEAM= build
```

Sign ad hoc (`CODE_SIGN_IDENTITY=-`), not `CODE_SIGNING_ALLOWED=NO`: an unsigned build has no keychain entitlement,
so SecureStore throws as soon as the auth client starts.

## Builds (EAS)

`eas.json` has `development`, `development-simulator`, `preview` and `production` profiles. Nothing has been built in
the cloud yet. Before the first build the owner needs to:

1. `eas init` with the Expo account (sets the project id; export it as `EAS_PROJECT_ID`, owner as `EXPO_OWNER`).
   Push tokens need the project id.
2. Set `EXPO_PUBLIC_CONVEX_URL` and `EXPO_PUBLIC_CONVEX_SITE_URL` (production Convex Cloud) as EAS environment
   variables; `EXPO_PUBLIC_SYNC_URL` and `EXPO_PUBLIC_API_URL` are in `eas.json`.
3. iOS: an Apple Developer account, the bundle id `vc.akatsuki.app`, an APNs key for Expo push, and a Mac or EAS
   Build. Android: a Google Play developer account and an FCM v1 service account for Expo push.
