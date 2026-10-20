# Developer guide

How to build, run and test Akatsuki locally. The binding design is in [`ARCHITECTURE.md`](ARCHITECTURE.md);
this page is about the tooling.

## Toolchain

| Tool | Version | Notes |
|---|---|---|
| Node | 26.x | `engines` in the root `package.json`; `engine-strict` is on. Node runs the generator scripts (`*.ts`) directly through type stripping. |
| pnpm | 12.x (pinned `12.11.2` in `packageManager`) | pnpm 12 reads its settings from `pnpm-workspace.yaml` (`allowBuilds`, `saveExact`, ...), not `.npmrc`. |
| Rust | 1.98 (edition 2024) | `rust-toolchain.toml` |
| Docker | 29.x | Only for the local Convex harness, and optional (see below). |

There is no `just` on the dev machine: use the `Makefile` (`make help`) or the root `package.json` scripts.

## Workspace layout

`pnpm-workspace.yaml` lists `apps/web`, `apps/landing`, `apps/mobile`, `apps/api`, `packages/*` and `convex`.
The Rust crates (`apps/desktop`, `apps/sync`, `crates/*`) are a Cargo workspace (`Cargo.toml`).

| Command | What it does |
|---|---|
| `pnpm install` | Install everything. Only `esbuild` may run install scripts (`allowBuilds`). |
| `pnpm dev` / `make dev` | Every package's `dev` script through turbo. |
| `pnpm build` / `make build` | Builds, generators first (`tokens`, `shortcuts`). |
| `pnpm typecheck` | `tsc --noEmit` in every package. |
| `pnpm lint` | Biome over the whole TypeScript tree (one root `biome.json`). Packages have no `lint` script of their own and turbo has no `lint` task (D-352). |
| `pnpm format` | Biome formatting and safe fixes. |
| `pnpm test` | Vitest in every package. |
| `pnpm tokens:build` / `make tokens` | `packages/tokens/tokens.json` → `dist/tokens.css`, `dist/fonts.css`, `dist/tokens.ts`, `dist/tokens.js` and `crates/tokens/src/generated.rs`. |
| `pnpm shortcuts:build` / `make shortcuts` | `packages/shortcuts/shortcuts.json` → `crates/shortcuts/src/generated.rs`; fails on conflicting bindings. |
| `make rust-check`, `make rust-test`, `make rust-lint`, `make rust-fmt` | `cargo` over the workspace. |
| `make desktop`, `make sync` | Run the desktop app or the sync worker. |
| `make check` | typecheck, lint, test, `cargo check`, `cargo test`. |

Lint and format: [Biome](https://biomejs.dev) with one root `biome.json` that extends
`packages/config/biome.preset.json`. Shared TypeScript presets are in `packages/config`
(`tsconfig.base.json`, `tsconfig.node.json`, `tsconfig.react.json`, `tsconfig.convex.json`); extend them as
`@akatsuki/config/tsconfig.react.json`.

## Design tokens and shortcuts

- Edit `packages/tokens/tokens.json`, then `make tokens`. Web and landing import
  `@akatsuki/tokens/tokens.css` (and `@akatsuki/tokens/fonts.css` for the bundled Instrument Serif, Geist and
  Geist Mono, OFL-licensed, see `packages/tokens/fonts/README.md`). Mobile imports `@akatsuki/tokens`
  (typed constants plus `fg()`, `accent()` and `rgba()` helpers). Desktop uses the `akatsuki-tokens` crate,
  whose `src/generated.rs` is written by the generator and checked in. Light mode is the default; dark mode
  follows `prefers-color-scheme` unless the page sets `data-theme="light"`, and `data-theme="dark"` forces it.
- Edit `packages/shortcuts/shortcuts.json`, then `make shortcuts`. TypeScript reads the JSON through
  `@akatsuki/shortcuts` (`keysFor`, `formatKeys`, `createMatcher`, `paletteCommands`, `findConflicts`). Rust gets
  `SHORTCUTS` and `ids::*` constants in `crates/shortcuts/src/generated.rs`, including GPUI keystroke strings.
  Nothing else may hard-code a palette colour or a key binding.

## Convex

Production runs on **Convex Cloud**. It is not configured yet: the owner creates the project and the
production deployment, and the CI deploy key (`CONVEX_DEPLOY_KEY`, CI only, never on a server) will come from
the owner. Until then all development and testing uses the local self-hosted backend below.

The functions live in `convex/` (a workspace package, `@akatsuki/convex`, with `convex.json` →
`"functions": "."`). Files with two dots in the name (`*.test.ts`, `setup.helpers.ts`,
`vitest.config.ts`, `package.entry.ts`, `test.modules.ts`) are skipped by the Convex bundler on purpose.
`convex/_generated` is checked in so other packages can typecheck without a deployment.

### Local self-hosted backend (dev/test harness)

`infra/docker-compose.dev.yml` runs the self-hosted Convex backend and dashboard
([docs.convex.dev/self-hosting](https://docs.convex.dev/self-hosting)), pinned to one upstream revision,
with containers `akatsuki-convex-backend` and `akatsuki-convex-dashboard` bound to loopback:

| Port | Service | Variable |
|---|---|---|
| 3410 | Deployment API | `CONVEX_SELF_HOSTED_URL=http://127.0.0.1:3410` |
| 3411 | HTTP actions (the local `*.convex.site`; Better Auth lives at `/api/auth/*`) | `CONVEX_SITE_URL=http://127.0.0.1:3411` |
| 6891 | Dashboard (Docker only) | open `http://127.0.0.1:6891` and paste the admin key |

Ports can be changed with `AKATSUKI_CONVEX_PORT`, `AKATSUKI_CONVEX_SITE_PORT` and
`AKATSUKI_CONVEX_DASHBOARD_PORT`.

```bash
make convex-up       # start, generate the admin key, write .env.local, provision the dev deployment env
make convex-deploy   # push schema + functions (typechecked): pnpm convex:dev:deploy
make convex-dev      # watch convex/ and push on every change
make convex-status   # is it up?
make convex-down     # stop, keep data
make convex-reset    # stop and delete all local data (a new admin key is appended next time)
```

`make convex-up` (`scripts/convex-dev.mjs up`) does the following:

1. Starts the backend. It uses Docker Compose when the current user can reach the Docker daemon. Otherwise it
   downloads the `convex-local-backend` binary of the **same upstream revision** into `.convex-dev/native/`
   and runs it detached on the same ports (log: `.convex-dev/native/backend.log`). Force one with
   `AKATSUKI_CONVEX_RUNTIME=docker|native`. To use Docker without sudo, add yourself to the `docker` group.
2. Generates an admin key (`./generate_admin_key.sh` in the container, or `keygen admin-key` for the binary)
   and **appends** `CONVEX_SELF_HOSTED_URL` and `CONVEX_SELF_HOSTED_ADMIN_KEY` to `.env.local` without printing
   them. Later lines win in dotenv parsers. The script never reads `.env.local`; a marker in `.convex-dev/`
   records what it already wrote.
3. Sets local-only deployment env vars with `npx convex env set --from-file`: `BETTER_AUTH_SECRET` (random),
   `SITE_URL=http://localhost:5173`, `DEV_ORIGINS`, `EMAIL_TRANSPORT=console`, `AKATSUKI_BILLING=off`
   (everyone is entitled, as on a self-hosted install), and ES256 service identities for the worker and
   apps/api (`WORKER_ISSUER`, `API_ISSUER`, `WORKER_SERVICE_JWKS`, `API_SERVICE_JWKS`). Convex refuses a push
   while an env var used by `auth.config.ts` is unset, so these must exist. The matching private keys
   (`WORKER_SERVICE_PRIVATE_KEY`, `API_SERVICE_PRIVATE_KEY`, kids) plus `CONVEX_URL` and `CONVEX_SITE_URL` are
   appended to `.env.local` once, for running `apps/sync` and `apps/api` locally. The key material is kept in
   `.convex-dev/dev-env.json` (mode 0600, gitignored).

The deploy command is `pnpm convex:dev:deploy`, which runs
`convex deploy --env-file ../.env.local -y` in `convex/`: it typechecks, regenerates `convex/_generated`,
bundles, and pushes schema, indexes, functions, crons and both components (`betterAuth`, `rateLimiter`).

With `EMAIL_TRANSPORT=console`, sign-in links and codes are written to the function logs instead of being sent.
This works only because the harness's `CONVEX_SITE_URL` is loopback (`http://127.0.0.1:3411`). On any other
deployment `console` is refused, or, with `AKATSUKI_DEV_HARNESS=1`, logs the email with the address, link and
code redacted (ARCHITECTURE §14.6, D-350):

```bash
cd convex && npx convex logs --env-file ../.env.local
```

The harness was verified end to end on 2026-10-20: email OTP sign-in → Better Auth session → RS256 JWT
from `/api/auth/convex/token` (`iss` = the site URL, `aud` = `convex`, 15 minutes) → authenticated
`me.get`, with the profile created by the `user.onCreate` trigger (spike S2). An ES256 worker service JWT was
accepted by `worker.assignedAccounts`, and an apps/api token was rejected by it (spike S3).

### Convex tests

`pnpm --filter @akatsuki/convex test` runs [convex-test](https://docs.convex.dev/testing/convex-test) with
Vitest in the edge runtime, with the Better Auth and rate-limiter components registered. The tests cover the
schema (including a check that `convex/schema.ts` equals the contract block in ARCHITECTURE §3.2), auth
guards (user A cannot read or change user B's data, services cannot impersonate each other), and the ops
outbox (enqueue, replay, undo, claim, complete, failure rollback, label ordering, snooze, the desktop feed).
No test needs a real key.

### Production (Convex Cloud), once configured

1. The owner creates the project and production deployment and gives CI a `CONVEX_DEPLOY_KEY`.
2. Set the deployment env (`npx convex env set --prod ...`): every Convex variable in `.env.example`, with the
   real service JWKS from `infra/scripts/gen-service-keys.mjs`, `EMAIL_TRANSPORT` unset (SES) and
   `AKATSUKI_BILLING` unset.
3. CI deploys with `npx convex deploy` in `convex/`.

## Web app (`apps/web`)

`pnpm --filter @akatsuki/web dev` serves http://localhost:5173 (the harness's `SITE_URL` and a trusted origin)
against the local harness with no env file: the Convex URLs default to the ports above (D-260). Sign in with an
email code from the function logs (`cd convex && npx convex logs --env-file ../.env.local`). Without a running
sync worker, bodies and mailbox connect show the worker error copy. Tests: `pnpm --filter @akatsuki/web test`
(Vitest, mock backend), `test:e2e` (Playwright on the mock-mode production bundle under the production CSP) and
`AKATSUKI_E2E_LOCAL=1 pnpm --filter @akatsuki/web test:e2e` (real sign-in against the harness). Details:
`apps/web/README.md`.

## End-to-end (local, no Docker)

`make e2e` runs the whole mail path on this machine (`scripts/e2e-local.mjs`, D-382):

1. `scripts/convex-dev.mjs up` and `pnpm convex:dev:deploy` (skip with `--skip-deploy`), then
   `cargo build -p akatsuki-sync --features e2e-fakes --bins` (skip with `--skip-build`).
2. `akatsuki-fake-mail`: the fake IMAP and SMTP servers of `crates/providers` on random loopback ports with
   implicit TLS and throwaway self-signed certificates, an INBOX of invented mail (one hostile HTML message) and
   a loopback control API (`GET /state`, `POST /deliver`).
3. The real worker on a free port (default `127.0.0.1:18787`), started through `node --env-file=.env.local` so the
   service key reaches the worker only (the script never reads the file). Its symmetric keys for the harness are
   random and kept in `.convex-dev/e2e/worker-keys.json`; `MAIL_CA_FILE` trusts the fake certificates. Logs go to
   `.convex-dev/e2e/run-*/worker.log` (content-free JSON).
4. A fresh test user signs in by email code (read from `convex logs` for that address only), connects the fake
   mailbox through `POST /v1/connect/imap`, and the script checks: threads in Convex (`mail.listThreads`),
   sanitized HTML from `GET /v1/messages/{id}` (no scripts, handlers, forms, frames or `javascript:`, remote
   images through the signed proxy, tracker removed), NDJSON thread bodies, archive (`mail.apply` move to the
   `\Archive` folder) reaching the server and `mail.undo` moving it back, `PUT /v1/drafts` and `POST /v1/send`
   reaching the fake SMTP server (HTML re-sanitized, no Bcc header) and the Sent folder, a newly delivered message
   reaching Convex over IDLE, and account removal and purge.

`make e2e-web` then runs every spec in `apps/web/e2e/local` with Playwright against the same stack: `mail.spec.ts`
(sign-in, the IMAP onboarding form, the synced inbox, a sanitized body in the sandboxed frame, archive with `e`
checked on the fake server, undo with `z`, mailbox removal in Settings) and `auth.spec.ts` (sign-in, onboarding,
error copy, a theme change that survives an immediate reload, sign-out). Sign-ins on the loopback harness carry no
client IP, so Better Auth's per-IP limits do not apply to them (D-401). `make e2e-serve` starts the fake mailbox and the
worker and keeps them running (`.convex-dev/e2e/serve.json` has the worker URL and the fake mailbox state file).

## Secrets

Never open, print or commit `.env` (the owner's keys) or any `.env.*` file other than `.env.example`.
Values go into gitignored `.env.local` / `.env.credentials` files or the relevant deployment env; names are
documented in `.env.example`.
