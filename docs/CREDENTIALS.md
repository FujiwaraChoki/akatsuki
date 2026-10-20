# Credentials and external setup

Names and status only. Values live in gitignored files: `.env` (owner's own keys, never read by tooling or agents), `.env.credentials` (keys created during setup, including `OPENAI_API_KEY`), `.env.local` (local dev values such as the self-hosted Convex admin key).

Account identifiers (DNS zone id, Stripe account and product ids, cloud project ids, DKIM tokens) are not kept in this file. They are in `docs/CREDENTIALS.local.md`, which is gitignored (`docs/*.local.md`) and exists only on the owner's machine (D-356).

| Service | What exists | Where the secret lives | Still needed |
|---|---|---|---|
| OpenRouter | API key | `.env` → `OPENROUTER_API_KEY` | — |
| Fal | API key (unused) | `.env` → `FAL_AI_API_KEY` | Nothing; Fal is not used |
| OpenAI (transcription) | API key | `.env.credentials` → `OPENAI_API_KEY` | — |
| Cloudflare | API token scoped to DNS Edit + Zone Read on `akatsuki.vc` only. Ops only: no component reads it. | `.env.credentials` → `CLOUDFLARE_API_TOKEN` | — |
| AWS SES (eu-north-1) | Domain identity `akatsuki.vc` (Easy DKIM 2048, custom MAIL FROM domain), DNS records added. Least-privilege IAM sender user (SendEmail/SendRawEmail on that identity, From `*@akatsuki.vc` only). Account has production access. | `.env.credentials` → `AWS_SES_REGION`, `AWS_SES_ACCESS_KEY_ID`, `AWS_SES_SECRET_ACCESS_KEY`, `SES_FROM_ADDRESS` | — (verified 2026-10-20: sending enabled, DKIM and MAIL FROM succeeded) |
| Stripe (sandbox) | Product **Akatsuki Pro**, price $20.00 USD monthly, lookup key `akatsuki_pro_monthly`. Trial (14 days) is set per Checkout session, not on the price. | `.env.credentials` → `STRIPE_SECRET_KEY`, `STRIPE_PUBLISHABLE_KEY` (sandbox) | Webhook secret after `api.akatsuki.vc` is live. Live mode needs business verification. |
| Google Cloud | Project **Akatsuki** (no organisation). Gmail API enabled. | — | Billing account link (needed for Pub/Sub push), Pub/Sub topic (`GMAIL_PUBSUB_TOPIC`) and push service account (`GMAIL_PUSH_SERVICE_ACCOUNT`, `GMAIL_PUSH_AUDIENCE`), OAuth consent screen (branding, domain, scopes, test users), OAuth clients for login (`GOOGLE_LOGIN_CLIENT_ID`/`_SECRET`) and for mail (`GOOGLE_MAIL_CLIENT_ID`/`_SECRET`; web + desktop), Search Console domain verification, later CASA assessment for restricted scopes |
| Microsoft Entra | — | — | App registration (multi-tenant + personal accounts), redirect URIs, client ids and secrets for login (`MICROSOFT_LOGIN_CLIENT_ID`/`_SECRET`) and mail (`MICROSOFT_MAIL_CLIENT_ID`/`_SECRET`), publisher verification |
| Convex Cloud | — | — | Project + prod deployment, deploy key (`CONVEX_DEPLOY_KEY`). On first deploy: check which client-IP header HTTP actions receive and set `AUTH_TRUSTED_PROXIES` if needed (D-401) |
| Vercel | Account signed in; apex `akatsuki.vc` + `www` already point at Vercel | — | Projects for `apps/landing` and `apps/web`, domains `akatsuki.vc`, `app.akatsuki.vc` |
| GitHub repository | — | — | Remote repository (CI has never run). Repository variable `AKATSUKI_PUBLISH_IMAGES` to publish images to GHCR; secret `CONVEX_DEPLOY_KEY` for a future Convex deploy job |
| Expo / EAS | — | — | Expo account and EAS project: `EAS_PROJECT_ID`, `EXPO_OWNER` (public build-time values; without them the app requests no push token) |
| Apple Developer / Google Play Console | — | — | Accounts for store builds; Apple team id and Android signing certificate for verified Universal Links / App Links (D-408) |
| Hetzner (`ssh hetzner`) | Shared server, Docker + host nginx | SSH key in `~/.ssh` | Deploy `akatsuki-sync`, `akatsuki-api` containers + nginx vhosts + TLS (only with owner's go-ahead) |

## DNS records on `akatsuki.vc` (Cloudflare)

Created for SES: three DKIM `CNAME`s (`<token>._domainkey`), the MAIL FROM `MX` and SPF `TXT` on the `mail` subdomain, and a `_dmarc` `TXT` at `p=none` (move to `quarantine` after a clean week). The exact values are in `docs/CREDENTIALS.local.md`.

Pre-existing and untouched: the apex and `www` records pointing at Vercel, the `_vercel` TXT, and four stale `NS` rows from the previous registrar at the apex.
