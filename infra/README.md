# infra

Deployment and local-run configuration. Nothing here is applied to a remote host without the owner's
go-ahead in the current conversation (BRIEF "Hosting"). Production steps: [`docs/DEPLOY.md`](../docs/DEPLOY.md).

| Path | What |
|---|---|
| `docker-compose.dev.yml` | Local Convex backend harness (below) |
| `hetzner/docker-compose.yml` | Production `akatsuki-sync` (`127.0.0.1:47310`) and `akatsuki-api` (`127.0.0.1:47311`) on the shared Hetzner host: read-only roots, non-root, no capabilities, healthchecks, rotated json-file logs, network `akatsuki-net`, spool volume `akatsuki-sync-spool`, env files `/opt/akatsuki/{sync,api}.env` |
| `hetzner/nginx/{sync,api}.akatsuki.vc.conf` | Host nginx vhosts (TLS, HTTP/2, HSTS, rate limits, body limits, SSE/streaming without buffering, no query strings in logs) |
| `hetzner/nginx/acme-bootstrap/` | Port-80-only vhosts used once to issue the first certificates |
| `hetzner/nginx/test-config.sh` | `nginx -t` on the vhosts in a temp prefix with self-signed stand-in certificates |
| `sync.env.example`, `api.env.example` | Names (never values) of the two services' env files |
| `scripts/gen-service-keys.mjs` | ES256 service identities (§6.6) and the worker's symmetric keys, written 0600 to the gitignored `infra/secrets/` |
| `selfhost/` | Self-host stack for personal, non-commercial use with the self-hosted Convex backend ([guide](selfhost/README.md)) |

The deploy script is `scripts/deploy-hetzner.sh` (dry run unless `--apply`). CI is `.github/workflows/ci.yml`.

## Local Convex backend (`docker-compose.dev.yml`)

A dev/test harness only: production uses Convex Cloud. The compose file runs the self-hosted Convex backend
and dashboard, pinned to upstream revision `72ce13c379fe6dbdfc305bc8ad699c5663c68c2a`
(release `precompiled-2026-10-10-72ce13c`), bound to loopback:

| Container | Host port | Purpose |
|---|---|---|
| `akatsuki-convex-backend` | 3410 | deployment API (`CONVEX_SELF_HOSTED_URL`) |
| `akatsuki-convex-backend` | 3411 | HTTP actions, Better Auth at `/api/auth/*` (`CONVEX_SITE_URL`) |
| `akatsuki-convex-dashboard` | 6891 | dashboard |

Data lives in the named volume `akatsuki-convex-data`. Use `make convex-up`, `make convex-down` and
`make convex-reset` rather than raw `docker compose`: the wrapper (`scripts/convex-dev.mjs`) also generates the
admin key, writes it to `.env.local` without printing it, provisions the local-only deployment env, and falls
back to the same revision's native binary when Docker is not usable. Details: [`docs/DEV.md`](../docs/DEV.md).

Raw commands, if needed:

```bash
docker compose -f infra/docker-compose.dev.yml up -d --wait
docker compose -f infra/docker-compose.dev.yml exec convex-backend ./generate_admin_key.sh   # prints the key
docker compose -f infra/docker-compose.dev.yml down            # keep data
docker compose -f infra/docker-compose.dev.yml down --volumes  # delete data
```
