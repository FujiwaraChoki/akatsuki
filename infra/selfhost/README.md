# Self-hosting Akatsuki

Akatsuki is **open source, free for personal use**, under the Akatsuki Source-Available License (`LICENSE`,
ASAL-1.0). You may run it for yourself and your household or friends at no charge. Running it for a business or
any organisation, for a fee or as a hosted service needs written permission from the copyright holder
(sami@samihindi.com).

This stack replaces Convex Cloud with the self-hosted Convex backend (FSL-1.1-Apache-2.0) and runs the sync
worker next to it. Billing is off: every user on your install is entitled to everything.

| Service | Container | Loopback port | Public URL (your reverse proxy) | Needed |
|---|---|---|---|---|
| Convex backend, deployment API + WebSocket | `akatsuki-selfhost-convex-backend` | 3210 | `AKATSUKI_CONVEX_URL` | yes |
| Convex backend, HTTP actions and Better Auth | same | 3211 | `AKATSUKI_CONVEX_SITE_URL` | yes |
| Sync worker | `akatsuki-selfhost-sync` | 8787 | `AKATSUKI_SYNC_URL` | yes |
| apps/api (managed AI with your own OpenRouter/OpenAI key) | `akatsuki-selfhost-api` | 8788 | your API URL | profile `ai`, optional: clients can call AI providers directly with their own keys |
| Web client (static) | `akatsuki-selfhost-web` | 8780 | your app URL | profile `web`, optional |
| Convex dashboard | `akatsuki-selfhost-convex-dashboard` | 6791 | keep private | profile `dashboard` |

Every port binds to `127.0.0.1`. Put a TLS reverse proxy (Caddy, nginx, Traefik) in front. The Convex
deployment URL carries a WebSocket, so the proxy must pass `Upgrade`. Do not log query strings: OAuth
callbacks carry codes in them.

## 1. Configure

```bash
cp infra/selfhost/selfhost.env.example infra/selfhost/.env       # public URLs, ports; no service secrets
openssl rand -hex 32                                             # paste as AKATSUKI_CONVEX_INSTANCE_SECRET
cp infra/sync.env.example infra/selfhost/sync.env && chmod 600 infra/selfhost/sync.env
cp infra/api.env.example  infra/selfhost/api.env  && chmod 600 infra/selfhost/api.env   # only for --profile ai
```

These files are gitignored. Never commit them.

## 2. Start Convex and push the functions

```bash
docker compose -f infra/selfhost/docker-compose.yml up -d --wait convex-backend
docker compose -f infra/selfhost/docker-compose.yml exec convex-backend ./generate_admin_key.sh
```

The second command prints an admin key. Keep it in your password manager; it can push code and read env.
Then, from `convex/`, with `CONVEX_SELF_HOSTED_URL=http://127.0.0.1:3210` and
`CONVEX_SELF_HOSTED_ADMIN_KEY` exported in your shell:

```bash
node ../infra/scripts/gen-service-keys.mjs --with-worker-secrets \
  --worker-issuer https://sync.example.com --api-issuer https://api.example.com
npx convex env set --from-file ../infra/secrets/<stamp>/convex.env.part   # issuers + public JWKS
npx convex env set SITE_URL https://app.example.com
npx convex env set BETTER_AUTH_SECRET "$(openssl rand -base64 32)"
npx convex env set AKATSUKI_BILLING off
npx convex env set EMAIL_TRANSPORT ses     # plus the AWS_SES_* names from .env.example, or sign in with OAuth only
npx convex deploy -y
```

Append `infra/secrets/<stamp>/sync.env.part` to `infra/selfhost/sync.env` (and `api.env.part` to
`api.env`). Fill in the remaining names in `sync.env`: `CORS_ORIGINS` (your app URL) and the mailbox OAuth
clients below. `CONVEX_URL`, `CONVEX_SITE_URL`, `PUBLIC_BASE_URL` and `BIND_ADDR` are set by the compose file.

## 3. Mailbox OAuth clients (your own)

Self-supplied OAuth clients work without Google verification while the consent screen is in testing mode
(add your accounts as test users). Redirect URIs:

| Provider | Purpose | Redirect URI |
|---|---|---|
| Google | mailbox (`GOOGLE_MAIL_CLIENT_*`, sync.env) | `https://sync.example.com/oauth/google/callback` |
| Microsoft | mailbox (`MICROSOFT_MAIL_CLIENT_*`, sync.env) | `https://sync.example.com/oauth/microsoft/callback` |
| Google | login (`GOOGLE_LOGIN_CLIENT_*`, Convex env) | `https://convex-site.example.com/api/auth/callback/google` |
| Microsoft | login (`MICROSOFT_LOGIN_CLIENT_*`, Convex env) | `https://convex-site.example.com/api/auth/callback/microsoft` |
| GitHub | login (`GITHUB_LOGIN_CLIENT_*`, Convex env) | `https://convex-site.example.com/api/auth/callback/github` |

IMAP accounts need no OAuth client. The worker connects only to public addresses on the standard mail ports
(IMAP 993/143, SMTP 465/587/25/2525). If your mail server is on your own network (a `192.168.x.x` address, a
`.lan` name that resolves privately) or uses another port, add `MAIL_ALLOW_PRIVATE_HOSTS=1` to `sync.env`; only do
this when everyone who can sign in to your install is trusted with your network (D-398).

## 4. Start the worker (and the optional services)

```bash
docker compose -f infra/selfhost/docker-compose.yml up -d --wait --build
docker compose -f infra/selfhost/docker-compose.yml --profile ai --profile web up -d --wait --build
```

`--build` builds `akatsuki-sync` (and `akatsuki-api`) from this checkout. Push notifications from Gmail and
Microsoft need public webhooks; the default `AKATSUKI_SYNC_PUSH=off` polls instead.

## Web client

Build `apps/web` with your URLs, then serve `apps/web/dist` with the `web` profile or any static host:

```bash
VITE_CONVEX_URL=https://convex.example.com VITE_CONVEX_SITE_URL=https://convex-site.example.com \
VITE_SYNC_URL=https://sync.example.com VITE_API_URL=https://api.example.com \
pnpm --filter ./apps/web build
```

Edit `img-src` in `web.nginx.conf` if you want to restrict remote images to your sync URL.

## Updating, backups

- `git pull`, then `npx convex deploy -y` and `docker compose ... up -d --wait --build`.
- Convex data lives in the volume `akatsuki-selfhost-convex-data`; back it up with
  `npx convex export --path backup.zip`. Message bodies are never stored there; they stay at your mail
  provider.
- `docker compose ... down` keeps data; `down --volumes` deletes it.
