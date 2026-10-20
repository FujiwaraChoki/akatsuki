# apps/api (akatsuki-api)

Stateless Hono service on Node for `https://api.akatsuki.vc` (`docs/ARCHITECTURE.md` §8, §12, §13): managed
AI (Pro), transcription, Stripe checkout, portal and webhook, and the worker-only triage endpoint. It never
persists or logs request or response content; it keeps only usage counts, in Convex.

## Routes

| Route | Auth | Notes |
|---|---|---|
| `GET /healthz` | none | `{"ok":true}` |
| `POST /v1/ai/{feature}` | user JWT | prompt-pack features only; `{promptVersion, vars}` ≤ 256 KB; vars validated against `input.schema.json` (422), version checked (409), `ee.service.aiPrecheck` (402/429), then an AASP `text/event-stream`. Usage is recorded with `ee.service.aiRecordUsage` when the stream ends (also for client aborts: input estimate). |
| `POST /v1/ai/transcribe` | user JWT | multipart `audio` (webm/ogg/m4a/mp4/wav/mp3, ≤ 25 MB, ≤ 10 min), `language?` (BCP-47), `prompt?` (≤ 500 chars) → `{text, durationSec, language}` via `gpt-4o-transcribe`. 120 min/day per user (UTC day) through the `transcriptionDay` bucket in `ee.service` (D-343): `429 rate_limited` with the reset time. Audio goes to OpenAI under its standard data policy (D-128). |
| `GET /v1/ai/quota` | user JWT | `ee.billing.entitlement` with the user's JWT → `{period, usedMicros, limitMicros, resetsAt}` |
| `POST /v1/billing/checkout`, `/portal`, `/cancel` | user JWT | §13.1; checkout is limited to 10/h per user |
| `POST /hooks/stripe` | Stripe signature | §13.2: verifies the signature on the raw body, re-fetches the customer's subscription and applies the snapshot through `ee.service.applyStripeSubscription`. Redeliveries are answered from a per-process dedupe cache, `ee.service.recordStripeEvent` is the durable one (D-346) |
| `POST /internal/v1/triage` | worker service JWT (`aud: akatsuki-api`) | ≤ 25 header-and-snippet items, Haiku tier; host nginx also denies `/internal/` publicly |

User JWTs: RS256, `iss = CONVEX_SITE_URL`, `aud = "convex"`, 30 s leeway, JWKS from
`${CONVEX_SITE_URL}/api/auth/convex/jwks` (1 h cache). Calls to Convex use apps/api's own ES256 service
token (`iss API_ISSUER`, `aud akatsuki-service`, `sub api`, 10 min, re-minted every 5 min). Every response
carries `Cache-Control: no-store`; CORS is an exact-origin allow-list (`CORS_ORIGINS`, default
`https://app.akatsuki.vc` and the local Vite dev server); only bearer tokens are accepted, never cookies.

## Logging

One JSON line per request: `t, method, route` (the route template, never the path or query), `status, ms,
user` (`sha256(userId)[:12]`), `model, inTok, outTok, audioSec, code, ua`. Nothing else, enforced by
`test/api.test.ts` ("log redaction").

## Run

```bash
pnpm --filter @akatsuki/api test        # vitest, no network, no keys
pnpm --filter @akatsuki/api build       # esbuild → dist/server.mjs (single file, no node_modules needed)
pnpm --filter @akatsuki/api dev         # node --watch src/main.ts, env from your shell
docker build -f apps/api/Dockerfile -t akatsuki-api .   # from the repository root
```

Environment names: `infra/api.env.example`, exactly the apps/api row of ARCHITECTURE §18 (`test/config.test.ts`).
Checkout uses the price with lookup key `STRIPE_PRICE_LOOKUP_KEY` (default `akatsuki_pro_monthly`) and a 14-day
trial (`trial_period_days`) for users who never had one. Without `OPENROUTER_API_KEY` the AI routes answer 503; without
`OPENAI_API_KEY` transcription does; without Stripe keys the billing routes do; `AKATSUKI_BILLING=off` removes
them (404).
