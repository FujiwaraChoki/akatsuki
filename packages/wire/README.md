# @akatsuki/wire

The wire contract between clients (web, mobile), the sync worker (`apps/sync`, Rust) and `apps/api`
(ARCHITECTURE §1.7, §7.3, §8, §12.6, §15). Serde twins live in `crates/core::wire`; shared fixtures in
`fixtures/` run on both sides.

```ts
import { createWorkerClient, createApiClient, setAccessTokenSource, splitQuoted, WireError } from "@akatsuki/wire";
import { api, type Id, vThreadAction } from "@akatsuki/wire/convex";
```

## Entry points

| Import | What | Resolution |
|---|---|---|
| `@akatsuki/wire` | Types, errors, AASP, offsets, tokens, HTTP clients. Needs only `convex/values` types. | Bundler and NodeNext (packages/ai, apps/api) |
| `@akatsuki/wire/aasp`, `@akatsuki/wire/errors` | Just those modules (packages/ai and packages/ui import these). | both |
| `@akatsuki/wire/convex` | `api`, `Doc`, `Id`, every validator of `convex/lib/validators.ts`, row-view types (`ThreadRow`, `LabelView`, …). Pulls the Convex sources in for types. | Bundler only (web, mobile) |

Sources use explicit `.ts` import extensions; consumers need `"allowImportingTsExtensions": true`. The main
entry uses only erasable TypeScript, so Node runs it directly (apps/api dev mode).

## Error codes (`errors.ts`, §15)

- `ERROR_CODES[code]` → `{ http, retryable, clientOnly, meaning, message }`; `ErrorCode`, `ALL_ERROR_CODES`,
  `isErrorCode`, `httpStatusFor` (codes without a status map to 400, as in Rust).
- Additions to the contract table: `not_implemented` (501, D-091) and the client-only AI states
  `consent_required`, `ai_off`, `key_missing` (D-121), next to `ai_model_required`.
- `errorBody(code, {message?, retryAfterMs?, version?})` builds the HTTP envelope; `parseErrorPayload(json)` reads
  an envelope, a bare payload or Convex error data; `toErrorPayload(thrown)` normalises anything (WireError,
  `ConvexError`, `AiFeatureError`, Error → `internal`).
- `WireError` carries `code`, `retryable`, `retryAfterMs`, `version` (for `draft_version_conflict`) and the HTTP
  `status`. User-facing copy is in `@akatsuki/ui` (`errorCopy`, `describeOpFailure`), never here.
- `fixtures/error-codes.json` is the registry snapshot (order, status, retryable, client-only).

## Worker and apps/api types (`worker.ts`, `api.ts`)

Exactly §7.3 and §8: `ConnectStart`, `ImapConnect`, `MessageBody`, `AttachmentMeta`, `DraftContent`, `DraftPut`,
`DraftSaved`, `SendRequest`, `SendAccepted`, `SearchResponse`, `PrefetchRequest`, `LinkResponse`,
`UploadResponse`, `AiFeatureRequest`, `TranscribeResponse`, `QuotaResponse`, `TriageInput`, … Optional fields are
omitted, never `null`, except where the type says `| null`. `Id<"messages">` here is the same type as
`convex/_generated/dataModel`'s.

### Quoted offsets are UTF-8 bytes (D-251)

`MessageBody.quoted.htmlStart` / `textStart` are **UTF-8 byte offsets** (the worker is Rust). JavaScript strings
are UTF-16, so convert before slicing, or every non-ASCII character before the quote shifts the cut:

```ts
const { visible, quoted } = splitQuoted(body.html, body.quoted.htmlStart); // safe: bad offsets keep all visible
const i = utf8OffsetToUtf16(body.text, body.quoted.textStart);              // throws RangeError on a bad offset
```

`utf16ToUtf8Offset` and `utf8ByteLength` go the other way. Fixtures: `fixtures/offsets.json` and
`fixtures/worker/message-body.non-ascii.json`.

## Access tokens (`token.ts`, §6.2 step 4)

```ts
const authClient = createAuthClient({ /* … convexClient() … */ });
setAccessTokenSource(authClient);          // wraps authClient.convex.token()
const jwt = await getAccessToken();        // memory cache, refreshed 60 s before exp, one request in flight
clearAccessToken();                        // on sign-out
```

`createAccessTokenSource(fetcher, { refreshBeforeMs, now })` makes an independent source (desktop-style
function fetchers, tests). Signed out → `WireError("unauthenticated")`. Tokens are never persisted.

## HTTP clients (`clients.ts`, `http.ts`)

```ts
const worker = createWorkerClient({ baseUrl: import.meta.env.VITE_SYNC_URL });   // uses getAccessToken()
const body = await worker.getMessage(messageId);
for await (const b of worker.threadBodies(threadId)) render(b);                   // NDJSON, newest first
await worker.putDraft(clientDraftId, { baseVersion, draft, deviceId });          // 409 → WireError draft_version_conflict
const apiClient = createApiClient({ baseUrl: import.meta.env.VITE_API_URL });
for await (const e of apiClient.streamFeature("summarize", { promptVersion, vars })) …; // AASP events
```

Every request sends `Authorization: Bearer <jwt>`, retries once with a fresh token after a 401, and throws
`WireError` for §15 bodies (status kept, `Retry-After` → `retryAfterMs`). A network failure is
`WireError{code:"unreachable", retryable:true}`. `readNdjson` and `createHttpClient` are exported for other
routes. Raw message bytes (`getRawMessage`) are desktop-only.

## AASP (`aasp.ts`, §12.6)

`AaspEvent` (`meta | delta | partial | done | error`), `encodeAaspEvent`, `aaspReadableStream` (producer side,
used by packages/ai and apps/api), `AaspLineParser` and `parseAaspStream` (consumer side). Parser rules match
`crates/core::wire::aasp`: all line endings (also `\r\n` split across chunks), comments, multi-line data,
unknown events and non-JSON data are skipped, an unterminated last frame is flushed.

## Shared fixtures

| Fixture | TypeScript | Rust |
|---|---|---|
| `fixtures/aasp/<name>.txt` + `.json` | `test/aasp.test.ts` (whole and byte by byte) | `crates/core::wire::shared_fixtures` |
| `fixtures/worker/*.json` (`{type, value}`) | `test/worker-fixtures.test.ts` (keys tied to the TS types, JSON round trip, no stray `null`) | `crates/core::wire::shared_fixtures` (serde round trip) |
| `fixtures/offsets.json` | `test/offsets.test.ts` | n/a (Rust strings are UTF-8) |
| `fixtures/error-codes.json` | `test/errors.test.ts` | not yet (follow-up: assert `ErrorCode::ALL` against it) |

`make fixtures` runs both sides.
