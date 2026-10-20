# Deploying Akatsuki

Step-by-step production setup. Nothing here may be run against a remote system without the owner's go-ahead
in the current conversation (BRIEF "Hosting"). The binding design is [`ARCHITECTURE.md`](ARCHITECTURE.md);
decisions are in [`DECISIONS.md`](DECISIONS.md) (infra: D-370 to D-379). Self-hosting for personal,
non-commercial use is a separate guide: [`infra/selfhost/README.md`](../infra/selfhost/README.md).

| Piece | Where | Hostname | Source |
|---|---|---|---|
| Convex (control plane, Better Auth) | Convex Cloud | `<deployment>.convex.cloud`, `<deployment>.convex.site` | `convex/` |
| Landing site (static Next.js export) | Vercel | `akatsuki.vc` | `apps/landing` |
| Web client (Vite SPA) | Vercel | `app.akatsuki.vc` | `apps/web` |
| Sync worker | Hetzner, container `akatsuki-sync`, `127.0.0.1:47310` | `sync.akatsuki.vc` | `apps/sync`, `infra/hetzner` |
| API service (managed AI, dictation, Stripe) | Hetzner, container `akatsuki-api`, `127.0.0.1:47311` | `api.akatsuki.vc` | `apps/api`, `infra/hetzner` |
| Transactional email | AWS SES (eu-north-1), sent by Convex | `no-reply@akatsuki.vc` | `convex/internal` |

Order for a first launch: service keys → Convex → SES → OAuth clients → Hetzner → DNS → Vercel → Stripe.
DNS for `sync`/`api` must exist before the Hetzner step can issue certificates.

## 1. Service keys

The worker and apps/api call Convex as themselves with ES256 JWTs (ARCHITECTURE §6.6, D-007). Create both key
pairs and the worker's symmetric keys on the owner's machine:

```bash
node infra/scripts/gen-service-keys.mjs --with-worker-secrets
```

It writes `infra/secrets/<timestamp>/` (gitignored, directory 0700, files 0600) and prints only key ids and
public thumbprints:

| File | Goes to |
|---|---|
| `convex.env.part` | Convex production env (public halves: `WORKER_ISSUER`, `API_ISSUER`, `WORKER_SERVICE_JWKS`, `API_SERVICE_JWKS`) |
| `sync.env.part` | `/opt/akatsuki/sync.env` (`WORKER_*` identity, `TOKEN_ENC_KEYS`, `STATE_KEY`, `SPOOL_KEY`, `LINK_SIGNING_KEY`, `IMG_PROXY_KEY`, `READ_RECEIPT_KEY`) |
| `api.env.part` | `/opt/akatsuki/api.env` (`API_*` identity, `WORKER_ISSUER`, `WORKER_SERVICE_JWKS`) |
| `service-keys.json` | the owner's password manager, then delete the local copy |

Rotation: `gen-service-keys.mjs --previous infra/secrets/<old>` publishes old and new public keys; set the new
JWKS in Convex, switch the services to the new private key and kid, redeploy, then publish a JWKS without the
old key. `TOKEN_ENC_KEYS` rotation is ARCHITECTURE §14.1 (`akatsuki-sync rotate-secrets`).

## 2. Convex Cloud

1. In the Convex dashboard create the project `akatsuki` and its production deployment. Note the two URLs:
   `CONVEX_URL = https://<deployment>.convex.cloud` and `CONVEX_SITE_URL = https://<deployment>.convex.site`.
   `CONVEX_SITE_URL` is the Better Auth JWT issuer that the worker and apps/api check (D-029): decide on any
   Convex custom domain **before** launch, because it changes `iss`.
2. Create a production deploy key (Settings → Deploy keys). It is **CI-only** (D-017): store it as the GitHub
   Actions secret `CONVEX_DEPLOY_KEY` and nowhere else; no server ever holds it.
3. Set the deployment env. From `convex/`, with the deploy key in the shell environment only:

   ```bash
   export CONVEX_DEPLOY_KEY=...            # typed or pasted, never written to a file in the repo
   npx convex env set --from-file ../infra/secrets/<timestamp>/convex.env.part
   npx convex env set SITE_URL https://app.akatsuki.vc
   openssl rand -base64 32 | npx convex env set BETTER_AUTH_SECRET
   npx convex env set SES_FROM_ADDRESS 'Akatsuki <no-reply@akatsuki.vc>'
   npx convex env set AWS_SES_REGION eu-north-1
   npx convex env set AWS_SES_ACCESS_KEY_ID          # prompts; value from the akatsuki-ses-sender IAM user
   npx convex env set AWS_SES_SECRET_ACCESS_KEY      # prompts
   npx convex env set GOOGLE_LOGIN_CLIENT_ID ...     # and _SECRET; likewise MICROSOFT_LOGIN_*, GITHUB_LOGIN_*
   ```

   | Variable | Production value |
   |---|---|
   | `SITE_URL` | `https://app.akatsuki.vc` |
   | `BETTER_AUTH_SECRET` | 32 random bytes, base64 |
   | `WORKER_ISSUER`, `API_ISSUER` | `https://sync.akatsuki.vc`, `https://api.akatsuki.vc` (from `convex.env.part`) |
   | `WORKER_SERVICE_JWKS`, `API_SERVICE_JWKS` | from `convex.env.part` |
   | `GOOGLE_LOGIN_CLIENT_ID/SECRET`, `MICROSOFT_LOGIN_CLIENT_ID/SECRET`, `GITHUB_LOGIN_CLIENT_ID/SECRET` | login-only OAuth clients (section 7); each provider is off while unset |
   | `AWS_SES_REGION`, `AWS_SES_ACCESS_KEY_ID`, `AWS_SES_SECRET_ACCESS_KEY`, `SES_FROM_ADDRESS` | section 8 |
   | `EMAIL_TRANSPORT` | unset (means `ses`); never `console` in production |
   | `AKATSUKI_BILLING` | unset (billing on) |
   | `AKATSUKI_MANAGED_AI` | unset on the hosted deployment |
   | `DEV_ORIGINS`, `AKATSUKI_DEV_HARNESS` | unset |

4. Deploy the functions: `npx convex deploy -y` from `convex/` (typechecks, pushes schema, indexes, functions,
   crons and the `betterAuth` and `rateLimiter` components). Convex refuses the push while an env var used by
   `auth.config.ts` is unset, so step 3 comes first. A CI deploy job can run the same command with the
   `CONVEX_DEPLOY_KEY` secret; it is not added yet.

## 3. Hetzner (sync.akatsuki.vc, api.akatsuki.vc)

The server (`ssh hetzner`, Ubuntu 24.04, Docker + Compose v2, host nginx on 80/443) runs many unrelated live
services. Akatsuki adds exactly:

- `/opt/akatsuki/` — compose file, release file, source trees for image builds, env files, ACME webroot,
  the spool directory;
- `/etc/nginx/sites-available/{sync,api}.akatsuki.vc.conf` and their `sites-enabled` symlinks;
- certificates for the two names under `/etc/letsencrypt` (certbot);
- containers `akatsuki-sync` and `akatsuki-api` (compose project `akatsuki`), network `akatsuki-net`, volume
  `akatsuki-sync-spool`, images `akatsuki-sync:<tag>` and `akatsuki-api:<tag>`.

`scripts/deploy-hetzner.sh` does all of it. It is a **dry run by default** (prints the plan, contacts nothing),
reloads nginx only after `nginx -t` passed (and restores the previous vhost when it fails), never restarts
nginx or any other service, and never prunes Docker.

### Ports

`127.0.0.1:47310` (sync) and `127.0.0.1:47311` (api), loopback only. They must be checked free on the server
before the first deploy; `--preflight` does it (`ss -ltnp`) and refuses to continue when anything else holds
them. If one is taken, change it in `infra/hetzner/docker-compose.yml`, the matching `upstream` block in the
vhost, `SYNC_PORT`/`API_PORT` in the script, and this page, in one change.

### Steps

1. **Env files on the server**, never in git, mode 0600, owner root. Start from the names in
   `infra/sync.env.example` and `infra/api.env.example`, add the `.part` files from section 1, the Convex URLs
   from section 2, the mailbox OAuth clients (section 7), the Gmail Pub/Sub settings, `OPENROUTER_API_KEY`,
   `OPENAI_API_KEY` and the Stripe values (section 6). Either write them on the server
   (`sudo install -m 0600 /dev/null /opt/akatsuki/sync.env && sudoedit /opt/akatsuki/sync.env`) or keep them
   in a private local directory outside the repo and pass `--upload-env DIR` (copied byte for byte, never
   read or printed by the script). `BIND_ADDR`, `API_INTERNAL_URL`, `PUBLIC_BASE_URL` and `SPOOL_DIR` are
   fixed by the compose file.
2. **Plan**: `scripts/deploy-hetzner.sh` and read it.
3. **Preflight** (read-only): `scripts/deploy-hetzner.sh --preflight`. Checks the ports, the container,
   network and vhost names (anything existing must carry Akatsuki's compose label or the
   `# managed-by: akatsuki` marker), that no other vhost already serves `sync.akatsuki.vc` or
   `api.akatsuki.vc` (`nginx -T`), the env-file modes, existing certificates and free disk space.
4. **Deploy**: `AKATSUKI_CERT_EMAIL=sami@samihindi.com scripts/deploy-hetzner.sh --apply`. In order:
   creates the directories (the spool as `65532:65532`, 0700); rsyncs the working tree (tracked and
   unignored files only; `.env*`, keys and build output are filtered and the list is checked) to
   `/opt/akatsuki/src/<tag>/`; builds both images on the server with their allow-list `.dockerignore`;
   writes `/opt/akatsuki/release.env` (keeping `release.prev.env`); `docker compose -p akatsuki up -d --wait`;
   checks both `/healthz` on loopback; installs each vhost (section "TLS" for the first time); checks
   `https://{sync,api}.akatsuki.vc/healthz`.
5. **Rollback**: `scripts/deploy-hetzner.sh --apply --rollback` restores the previous release's images.
6. **Logs**: `docker logs akatsuki-sync` (content-free JSON, rotated at 5 × 10 MB), nginx
   `/var/log/nginx/akatsuki-{sync,api}.access.log` (custom format without query strings, client IPs or
   capability tokens; rotated by the host's nginx logrotate).

Images are built on the server because GHCR publishing is off by default. To pull instead, set the repository
variable `AKATSUKI_PUBLISH_IMAGES=true`, push a `v*` tag (`.github/workflows/release-images.yml` publishes
`ghcr.io/<owner>/akatsuki-{sync,api}`), and put those references in `/opt/akatsuki/release.env`.

### TLS

Certificates are Let's Encrypt through certbot, one per name. Two methods; the script supports both:

- **Webroot (default, `AKATSUKI_CERT_METHOD=webroot`)**. Needs nothing from the existing setup. On a first
  deploy the script installs a port-80-only bootstrap vhost (`infra/hetzner/nginx/acme-bootstrap/`) that
  serves `/.well-known/acme-challenge/` from `/opt/akatsuki/acme`, reloads, runs
  `certbot certonly --webroot -w /opt/akatsuki/acme -d <name> --deploy-hook 'systemctl reload nginx'`, then
  replaces it with the full vhost. Renewal uses the same webroot (the full vhost keeps the ACME location on
  port 80) through certbot's systemd timer.
- **The server's existing nginx authenticator (`AKATSUKI_CERT_METHOD=nginx`)**, if the other sites were set up
  with `certbot --nginx`. The script runs `certbot certonly --nginx -d <name>`: `certonly` uses the nginx
  plugin only to answer the challenge and does not edit any vhost.

Check which one the server uses with `sudo grep -h authenticator /etc/letsencrypt/renewal/*.conf`. Never run
`certbot --nginx` without `certonly`: it would rewrite our vhost files.

The vhosts: TLS 1.2/1.3 with Mozilla intermediate ciphers, HTTP/2 (`listen 443 ssl http2`, the syntax nginx 1.24
on Ubuntu 24.04 understands; switch to `http2 on;` after an upgrade to ≥ 1.25.1), HSTS one year, per-IP rate
limits (the per-user limits of §13.4 live in the services), body limits per route (sync: 2 MiB JSON, 25 MB
uploads streamed; api: 64 KiB default, 256 KiB AI, 25 MB dictation streamed, 1 MiB Stripe), no request or
response bodies on nginx's disk (`client_body_buffer_size` covers each JSON limit, `proxy_request_buffering
off` for uploads, `proxy_max_temp_file_size 0`), buffering off for the AASP stream (`/v1/ai/*`) and the NDJSON
and attachment routes, `/metrics` and `/internal/` denied, and an error log at `crit` (nginx's error lines
contain full request lines). Run `infra/hetzner/nginx/test-config.sh [nginx]` locally to `nginx -t` them; CI
does it with Ubuntu's nginx 1.24.

## 4. Cloudflare DNS (zone `akatsuki.vc`)

| Name | Type | Content | Proxy | Why |
|---|---|---|---|---|
| `sync` | A | `65.21.141.237` | **DNS only** | Webhooks (Gmail Pub/Sub OIDC JWTs, Graph validation within 10 s) and long NDJSON and attachment streams. A proxied record would terminate TLS at Cloudflare, so message bodies and attachments would be decrypted by a third party; Cloudflare's 100 s origin timeout would cut long streams; and the http-01 challenge and per-IP rate limits need the real client. |
| `api` | A | `65.21.141.237` | **DNS only** | AI requests carry message text and stream answers for up to 5 minutes (AASP over SSE): same TLS and 100 s timeout reasons. The Stripe webhook signature is unaffected either way. |
| `@` (`akatsuki.vc`) | A | Vercel's value from the project's Domains page (currently `76.76.21.21`) | DNS only | Already points at Vercel. Vercel issues the certificate and needs to see the requests. |
| `www` | CNAME | `cname.vercel-dns.com` | DNS only | Redirected to the apex in Vercel. |
| `app` | CNAME | `cname.vercel-dns.com` | DNS only | The web client. |
| SES records | CNAME / MX / TXT | section 8 | DNS only | |

Trade-off of DNS-only: the server's IP is public (it already is, for the existing mail server) and Cloudflare's
DDoS protection does not front these two names; nginx rate limits are the first line. The BRIEF ("Hosting")
already fixes `sync`/`api` as DNS-only. No other records change.

## 5. Vercel

Two projects in the owner's Vercel team, both connected to the GitHub repository:

| Project | Root Directory | Domain | Settings come from |
|---|---|---|---|
| `akatsuki-landing` | `apps/landing` | `akatsuki.vc` (+ `www.akatsuki.vc` → redirect to apex) | `apps/landing/vercel.json`: no framework preset (plain static files), install `pnpm install --frozen-lockfile`, build `cd ../.. && pnpm exec turbo run build --filter=@akatsuki/landing`, output `out` (Next.js static export) |
| `akatsuki-web` | `apps/web` | `app.akatsuki.vc` | `apps/web/vercel.json`: framework `vite`, same install, build `... --filter=./apps/web`, output `dist`, SPA rewrite to `/index.html` |

Project settings to set by hand:

- "Include files outside the Root Directory in the Build Step": on (workspace packages).
- Node.js version: 26.x (the workspace's `engines`). If Vercel does not offer Node 26 yet, build in GitHub
  Actions and upload with `vercel deploy --prebuilt` instead.
- Env `ENABLE_EXPERIMENTAL_COREPACK=1` so Vercel uses the `packageManager` pin (`pnpm@12.11.2`).
- Web env (Production): `VITE_CONVEX_URL`, `VITE_CONVEX_SITE_URL`, `VITE_SYNC_URL=https://sync.akatsuki.vc`,
  `VITE_API_URL=https://api.akatsuki.vc`. Landing env: `NEXT_PUBLIC_APP_URL=https://app.akatsuki.vc`. These are public values; no secret belongs in a Vercel
  project.

Both `vercel.json` files set a Content-Security-Policy and the usual security headers. The web CSP allows
images from `https://sync.akatsuki.vc` (the image proxy) and `connect-src https: wss:` plus loopback, because
BYO AI calls go from the browser straight to the user's chosen provider or local Ollama (D-009); the microphone
is allowed for dictation.

## 6. Stripe

1. Product "Akatsuki Pro", recurring price $20/month with lookup key `akatsuki_pro_monthly`
   (`STRIPE_PRICE_LOOKUP_KEY`). The 14-day trial is set per Checkout session by apps/api (§13.1), not on the
   price.
2. Webhook endpoint `https://api.akatsuki.vc/hooks/stripe`, API version as the `stripe@23.0.0` SDK pins it,
   events: `checkout.session.completed`, `customer.subscription.created`, `customer.subscription.updated`,
   `customer.subscription.deleted`, `customer.subscription.paused`, `customer.subscription.resumed`,
   `customer.subscription.trial_will_end`, `invoice.paid`, `invoice.payment_failed` (§13.2).
3. Put the signing secret (`whsec_...`) in `/opt/akatsuki/api.env` as `STRIPE_WEBHOOK_SECRET` and a restricted
   secret key as `STRIPE_SECRET_KEY` (Customers, Checkout Sessions, Billing Portal, Subscriptions: write;
   Prices: read).
4. Customer portal: enable cancel and payment-method update; return URL `https://app.akatsuki.vc/billing`.
5. Test with the sandbox first; live mode needs Stripe's business verification.

## 7. OAuth clients and redirect URIs

Login and mailbox access use **separate** clients (§6.1): login clients live in Convex env and ask for
`openid email profile` only; mailbox clients live only in the worker's env.

| Provider | Client | Env (where) | Redirect URI |
|---|---|---|---|
| Google | login | `GOOGLE_LOGIN_CLIENT_ID/SECRET` (Convex) | `https://<deployment>.convex.site/api/auth/callback/google` |
| Microsoft (Entra, `common` tenant) | login | `MICROSOFT_LOGIN_CLIENT_ID/SECRET` (Convex) | `https://<deployment>.convex.site/api/auth/callback/microsoft` |
| GitHub | login | `GITHUB_LOGIN_CLIENT_ID/SECRET` (Convex) | `https://<deployment>.convex.site/api/auth/callback/github` |
| Google | mailbox (`gmail.modify`) | `GOOGLE_MAIL_CLIENT_ID/SECRET` (sync.env) | `https://sync.akatsuki.vc/oauth/google/callback` |
| Microsoft | mailbox (`offline_access openid email User.Read Mail.ReadWrite Mail.Send`) | `MICROSOFT_MAIL_CLIENT_ID/SECRET` (sync.env) | `https://sync.akatsuki.vc/oauth/microsoft/callback` |

For local development add the local harness's site URL (`http://127.0.0.1:3411/api/auth/callback/<provider>`)
and `http://127.0.0.1:8787/oauth/<provider>/callback` to separate development clients, not to the production
ones. Desktop (`http://127.0.0.1:*`) and mobile (`akatsuki://`) return addresses are Better Auth
`callbackURL`s and worker `returnTo` values, not OAuth redirect URIs, so they are not registered with the
providers.

Push endpoints for the mailbox side:

- Gmail: a Pub/Sub topic (`GMAIL_PUBSUB_TOPIC`) with `gmail-api-push@system.gserviceaccount.com` as
  publisher, and a push subscription to `https://sync.akatsuki.vc/hooks/gmail` with OIDC authentication by a
  service account (`GMAIL_PUSH_SERVICE_ACCOUNT`) and audience `https://sync.akatsuki.vc/hooks/gmail`
  (`GMAIL_PUSH_AUDIENCE`). Pub/Sub needs billing on the Google Cloud project.
- Graph: the worker creates subscriptions itself with `notificationUrl https://sync.akatsuki.vc/hooks/graph`
  and `lifecycleNotificationUrl https://sync.akatsuki.vc/hooks/graph/lifecycle`; nothing to register.

## 8. AWS SES (transactional email, sent by Convex only)

1. In SES (region `eu-north-1`), create the domain identity `akatsuki.vc` with Easy DKIM (RSA 2048). Add the
   three DKIM CNAME records at Cloudflare, DNS only.
2. Custom MAIL FROM domain `bounce.akatsuki.vc`: MX `10 feedback-smtp.eu-north-1.amazonses.com` and TXT
   `v=spf1 include:amazonses.com -all` on `bounce`. This keeps the apex SPF untouched for the existing mail
   server.
3. DMARC: TXT on `_dmarc` (start with `v=DMARC1; p=none; rua=mailto:<owner address>`, tighten later). If a
   DMARC record already exists, keep it and only check it is aligned.
4. Request production access (leave the sandbox) for transactional mail.
5. IAM user `akatsuki-ses-sender`, programmatic access only, policy allowing `ses:SendEmail` on the
   `akatsuki.vc` identity with condition `ses:FromAddress = no-reply@akatsuki.vc`. Its keys go only into
   the Convex env (`AWS_SES_ACCESS_KEY_ID`, `AWS_SES_SECRET_ACCESS_KEY`), never into a file in the repo.
6. Configuration set with bounce and complaint notifications is optional for launch.

## 9. CI

`.github/workflows/ci.yml` needs no secrets. `release-images.yml` uses only `GITHUB_TOKEN` and runs only when
the repository variable `AKATSUKI_PUBLISH_IMAGES` is `true`. The only CI secret the plan needs is
`CONVEX_DEPLOY_KEY`, for a Convex deploy job once the owner adds one.
