# @akatsuki/ai

The TypeScript AI layer (`docs/ARCHITECTURE.md` §12) on the Vercel AI SDK 7 (`ai@7.0.145`). Used by the web
and mobile clients (BYO keys and local models, directly from the device) and by `apps/api` (the managed,
Pro path). The Rust desktop uses `crates/ai` against the same `routing.json`, settings schema, prompt pack
and fixtures.

| Module | What it does |
|---|---|
| `routing.json`, `src/routing.ts` | Feature → tier table and tier → model per provider kind. The only place model ids live. Also the escalation candidates (D-120) and list prices in micro-dollars. |
| `src/settings.ts` | zod schema of the synced `aiSettings` (profiles, routes, consent). Keys are never part of it: a profile names where its key lives (`keyOnDevices`), the device keychain holds it under `profileId`. `egressFor(profile, feature)` / `egressText(profile, feature)` give the "where the data goes" label (`device` on loopback, `network` on RFC 1918 / `.local`, `provider`, `akatsuki`, and `akatsuki_openai` for managed dictation; `fixtures/egress/cases.json`, shared with `crates/ai`). `schema/ai-settings.schema.json` is generated from it. |
| `src/resolve.ts` | `resolveFeature(feature, settings)`: route → profile → model, exactly §12.3. |
| `src/providers.ts` | `createAkatsukiRegistry` (AI SDK `createProviderRegistry`, one provider per profile, models addressed `profileId:modelId`) over `@ai-sdk/anthropic`, `@ai-sdk/openai`, `@ai-sdk/google`, `@ai-sdk/gateway`, `@openrouter/ai-sdk-provider`, `ollama-ai-provider-v2` and `@ai-sdk/openai-compatible`. `createManagedModel` for apps/api. |
| `src/run.ts` | `streamFeature`: validates vars, renders the prompt pack, calls `streamText` (with `Output.object` for object features) and yields AASP events (§12.6). |
| `src/features.ts` | `runFeature(feature, vars, ctx)` (BYO or managed) and the named functions `summarize`, `instantReply`, `writeInVoice` (feature `compose`), `rewrite`, `extractActions`, `triage`, `nlSearchToQuery` (feature `nl_search`), `voiceProfile`. |
| `src/transcribe.ts` | Dictation: OpenAI transcription (`gpt-4o-transcribe`, `gpt-4o-mini-transcribe`, `whisper-1`) or a local Whisper server with an OpenAI-compatible `/v1/audio/transcriptions` endpoint. |
| `src/transcript.ts` | `renderTranscript` and `renderTriageItems` (spec in `packages/prompts`), with one whitespace set shared with Rust (Unicode `White_Space` plus U+FEFF). |
| `src/aasp.ts`, `src/sse.ts` | Re-exports of `@akatsuki/wire/aasp` (AASP event types, SSE encoder, `AaspLineParser`, `parseAaspStream`; D-251), kept so this package's API is unchanged. |
| `src/pricing.ts` | Micro-dollar costs from `routing.json` (Sonnet 5.5 $2/$10, Haiku 5.5 $0.10/$0.50, Opus 5.5 $4/$20, Fable 5.1 $10/$50 per MTok, gpt-4o-transcribe $0.006/min). |

## Models

| Tier | Features | Managed / OpenRouter / Gateway | Anthropic direct |
|---|---|---|---|
| `fast` | `triage`, `nl_search` | `anthropic/claude-haiku-5.5` | `claude-haiku-5-5` |
| `default` | everything else | `anthropic/claude-sonnet-5.5` | `claude-sonnet-5-5` |
| `heavy` | none in v1 (escalation only) | `anthropic/claude-opus-5.5` | `claude-opus-5-5` |
| `transcription` | `dictation` | `gpt-4o-transcribe` | — |

OpenAI, Google, Ollama and OpenAI-compatible profiles have no table models: the user picks one
(`ai_model_required` otherwise). Escalation (`escalateOnInvalidOutput`, opt-in per call) retries one invalid
structured answer on the first heavy candidate (Opus 5.5; Fable 5.1 is the second candidate, BYO only,
because it is not available under zero data retention).

## Privacy in code

- OpenRouter requests (managed and BYO) always carry `provider: { zdr: true, data_collection: "deny",
  allow_fallbacks: true, require_parameters: true }`. A model without a ZDR endpoint fails instead of falling
  back to one that keeps data.
- `runFeature` refuses to run without `consent.grantedAt`. Keys are passed in by the caller per call
  (`keyFor(profileId)`); nothing reads provider keys from the environment on clients.
- Anthropic calls from a browser set `anthropic-dangerous-direct-browser-access: true` (BYO keys only).
- No AI SDK telemetry is registered. Errors carry fixed messages, never provider response text.

Client-side error codes thrown as `AiFeatureError` before any request: `consent_required`, `ai_off`,
`ai_model_required`, `key_missing`, `unsupported` (D-121).

## Tests

`pnpm --filter @akatsuki/ai test`: AI SDK mock models (`MockLanguageModelV4`, `MockTranscriptionModelV4`) and a
stubbed `fetch` for the real provider adapters. No keys, no network. `pnpm --filter @akatsuki/ai build`
regenerates `schema/ai-settings.schema.json`.
