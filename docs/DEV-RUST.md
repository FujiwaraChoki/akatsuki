# Rust development

How to build, test and run the Rust side of Akatsuki: the Cargo workspace, the sync worker
(`apps/sync`) and the GPUI desktop app (`apps/desktop`). The contract is
[`ARCHITECTURE.md`](ARCHITECTURE.md); decisions are in [`DECISIONS.md`](DECISIONS.md).

## Toolchain

- Rust **1.98.1**, pinned in `rust-toolchain.toml` (rustup installs it with `rustfmt` and `clippy`
  on first use). Edition 2024, resolver 3.
- `cargo-deny` for the licence gate: `cargo install cargo-deny --locked`.
- No `just`: use the Cargo aliases below or the root `Makefile`.

## System packages

The workspace is pure Rust except for GPUI's windowing libraries. `rusqlite` bundles SQLite (with
FTS5), TLS is rustls, and `aws-lc-sys` builds with the C compiler alone (no CMake, no NASM on
Linux).

**Arch / Omarchy** (verified on this machine, Hyprland on Wayland):

```sh
sudo pacman -S --needed base-devel clang pkgconf \
  libxcb libxkbcommon libxkbcommon-x11 wayland fontconfig freetype2 \
  vulkan-icd-loader
```

plus the Vulkan driver for your GPU (`vulkan-radeon`, `vulkan-intel` or `nvidia-utils`).

**Debian / Ubuntu**:

```sh
sudo apt install build-essential clang pkg-config \
  libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libfontconfig-dev libfreetype-dev libvulkan1 mesa-vulkan-drivers
```

What needs what:

| Library | When | Why |
|---|---|---|
| `libxcb`, `libxcb-xkb`, `libxkbcommon`, `libxkbcommon-x11` | link time | GPUI keyboard handling (X11 and Wayland) |
| `wayland` (`libwayland-client`) | run time (dlopen) | GPUI's Wayland backend |
| `vulkan-icd-loader` + a driver | run time | GPUI renders through Blade on Vulkan |
| `fontconfig`, `freetype2` | run time (dlopen) | system font discovery (bundled fonts work without it) |
| `clang`, `pkgconf` | build time | C build scripts and library discovery |

The sync worker needs none of these; it builds in the stock `rust` Docker image.

macOS builds GPUI's Metal shaders at run time (`runtime_shaders`, as in ref/Siberia), so Xcode's
offline Metal compiler is not needed.

## Everyday commands

```sh
cargo build --workspace                                   # everything, including the desktop app
cargo t                                                   # alias: cargo test --workspace
cargo lint                                                # alias: clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo deny check                                          # licences, bans, sources, advisories (deny.toml)
cargo sync                                                # alias: run the sync worker
cargo desktop                                             # alias: run the desktop app
AKATSUKI_EXIT_AFTER_MS=3000 cargo desktop                 # open the window, quit after 3 s (smoke test)
cargo desktop -- --demo                                   # the demo mailbox: invented mail, no server
```

CI runs `cargo fmt --all --check`, `cargo lint`, `cargo t` and `cargo deny check` (all four checks).

## Workspace layout

| Crate | Package | What it holds |
|---|---|---|
| `crates/core` | `akatsuki-core` | Domain model (label keys, `apply_ops`, rule matching), Convex shapes (`convex::{schema,rows,worker,client}`), worker/apps-api wire types, AASP, the error-code registry, the account-actor state machine (`sync`), user-JWT verification and service-JWT minting (`auth`), and with the `store` feature the desktop SQLite mirror (§16.3) |
| `crates/providers` | `akatsuki-providers` | `MailProvider`, `TokenSource`, `PushSource`, `MetaSink` traits (§9.1), provider error disposition (§5.5), label tables, IMAP thread ids, `FetchedThread` → `ThreadUpsert`, and the adapters: `gmail` (Gmail API), `graph` (Microsoft Graph), `imap` (IMAP + SMTP, IDLE). Feature `fake-servers` exposes the in-process fake IMAP/SMTP servers (`imap::fake`) for the local end-to-end harness |
| `crates/mime` | `akatsuki-mime` | RFC 5322/MIME parsing (`mail-parser`), the HTML sanitizer (ammonia → safe tree → HTML), CSS allow-list, quote detection, `compose` (`mail-builder`), `MessageBody` |
| `crates/search` | `akatsuki-search` | Query grammar (§11.1) and the SQLite FTS5 index |
| `crates/convex-api` | `akatsuki-convex-api` | Typed `WorkerApi` / `ClientApi` over the official `convex` client, with a `MockTransport` |
| `crates/ai` | `akatsuki-ai` | Streaming AI client: OpenAI-compatible, Anthropic native, managed (apps/api); prompt pack, routing, resolution, transcript |
| `crates/tokens` | `akatsuki-tokens` | Design tokens; `generated.rs` is written by `packages/tokens` |
| `crates/shortcuts` | `akatsuki-shortcuts` | Key map; `generated.rs` is written by `packages/shortcuts` |
| `apps/sync` | `akatsuki-sync` | The sync worker binary (lib + bin): HTTP API, account supervisor and actors, `ConvexSink`, OAuth connect, spool, image proxy, push hooks, `rotate-secrets` |
| `apps/desktop` | `akatsuki-desktop` | The GPUI desktop app: sign-in, mirror feed, lists, native reader, palette, compose, search, BYO AI (see "Desktop app" below) |

Dependency direction: `core` ← `search` ← `providers`; `core` ← `mime`, `convex-api`, `ai`; apps on
top. `core` depends on no other workspace crate (D-090).

## Conventions

- Every third-party version is declared once in `[workspace.dependencies]` as an exact (`=`) pin,
  the version in `Cargo.lock` (D-396); a bump is a reviewed edit of that line. Lints are shared
  through `[workspace.lints]`.
- Convex-facing structs use `#[serde(rename_all = "camelCase")]`, enums `snake_case`, and
  `#[skip_serializing_none]` because Convex rejects `null` for optional fields. `v.int64()` and
  `v.bytes()` travel as `ConvexInt64` / `ConvexBytes`, which encode to Convex's tagged JSON
  (`{"$integer": …}`, `{"$bytes": …}`), so arguments round-trip without loss (spike S9).
- Secrets: configuration types implement `Debug` without values; logs carry route templates,
  status, latency and `sha256(userId)[..12]` only. Never print a key, token, body, subject or
  address. Never read `.env` files from Rust code or tests; the worker reads the process
  environment, and local values go in the gitignored `.env.local`.
- Errors: HTTP errors are the §15 envelope built from `akatsuki_core::wire::ErrorCode`.
- Tests never need real keys or a network: providers are mocked with `wiremock`, Convex with
  `MockTransport`, JWKS with an in-memory source.

## Generated files

- `crates/tokens/src/generated.rs` and `crates/shortcuts/src/generated.rs` come from
  `pnpm --filter @akatsuki/tokens build` and `pnpm --filter @akatsuki/shortcuts build`. Do not edit
  them by hand; the crate shells (`lib.rs`) add helpers and tests.
- `crates/ai` embeds `packages/ai/routing.json` and the prompt pack (`packages/prompts`:
  `manifest.json`, `shared/untrusted.md`, `features/<id>/*`) at build time (`build.rs`), so a
  packaged desktop build needs no checkout (`PromptPack::embedded()`). There is no fallback copy:
  building `akatsuki-ai` without those files fails (D-341).

## Shared fixtures

The Rust twins run the shared fixture files from the TypeScript packages:

| Fixtures | Rust test |
|---|---|
| `packages/ai/fixtures/resolve/*.json`, `fixtures/settings/*.json`, `fixtures/base-url/cases.json`, `fixtures/egress/cases.json` | `crates/ai` `settings::tests` |
| `packages/prompts/fixtures/render/*.json` (system and user prompts, byte-exact), `fixtures/template/cases.json`, `pack.json` | `crates/ai` `prompts::tests` |
| `packages/prompts/shared/fixtures/transcript/*.json`, `triage-items/*.json`, `whitespace.json` | `crates/ai` `transcript::tests` |
| `packages/mail-model/fixtures/query.json` | `crates/search` (skipped until the file exists) |

Label-key, apply-ops, rules, wire and AASP fixtures get runners once `packages/mail-model` and
`packages/wire` publish them.

## Pins and quirks

- `libc` is pinned to `=0.2.189`: `gpui 0.2.2` depends on `xattr 0.2`, which uses
  `libc::ENOATTR`, removed on Linux in libc 0.2.190. Same pin as ref/Siberia. Lift it when GPUI
  updates.
- On Wayland GPUI ignores `TitlebarOptions::title`; the desktop sets the title with
  `window.set_window_title`. The Wayland app id is `vc.akatsuki.desktop`.
- Accepted advisories are listed in `deny.toml` `[advisories] ignore`, each with its reason (D-396):
  unmaintained crates deep in the GPUI tree, and RUSTSEC-2023-0071 (RSA timing, `rsa` via
  `jsonwebtoken`'s `rust_crypto` backend; Akatsuki only verifies RS256 signatures with public keys,
  which that advisory does not affect). `cargo deny check` passes, so any new advisory fails it.
  Review the list whenever GPUI or jsonwebtoken is bumped.
- `gpui 0.2.2` → `stacksafe 0.1.4` → `stacksafe-macro` → `proc-macro-error2 2.0.1` makes every cargo
  run print "the following packages contain code that will be rejected by a future version of
  Rust" (D-357). It does not fail the build today. Lift it when GPUI moves off stacksafe /
  proc-macro-error2, and check `cargo report future-incompatibilities` before bumping
  `rust-toolchain.toml`: a toolchain that turns the warning into an error breaks the desktop build.

## Sync worker

```sh
BIND_ADDR=127.0.0.1:8787 cargo sync                       # serve
curl -s localhost:8787/healthz                            # {"ok":true}
cargo run -p akatsuki-sync -- healthcheck                 # exit 0 when healthy (used by the image)
cargo run -p akatsuki-sync -- rotate-secrets              # re-seal account secrets under the active TOKEN_ENC_KEYS key
```

With `CONVEX_URL` and the worker service key set, the worker connects to Convex as
`worker:<instance>` (ES256 service JWT, re-minted every 5 minutes), subscribes to
`worker.assignedAccounts` and `worker.pendingOpAccounts`, and runs one account actor per mailbox
(`src/engine`; decisions D-230 to D-248). Without them it serves `/healthz` and answers the other
routes with `503 provider_unavailable`. Uploads are spooled encrypted under `SPOOL_DIR` (default
`/var/lib/akatsuki/spool`). Engine and handler tests run against `FakeProvider`, an in-memory
`WorkerApi` or `MockTransport`, and `wiremock`; they need no keys and no network.

IMAP and SMTP trust the Mozilla roots (`webpki-roots`) only; `MAIL_CA_FILE` adds the certificates of a
PEM file, for self-hosted mail servers with a private CA (D-381).

Configuration is read from the variables listed in ARCHITECTURE §18 ("Worker"). Missing
production variables are logged by name at start-up; routes that need them refuse requests.
Logs are JSON when stderr is not a terminal; `RUST_LOG` sets the level.

Docker (multi-stage, distroless, non-root), from the repository root:

```sh
docker build -f apps/sync/Dockerfile -t akatsuki-sync .
```

The build context is an allow-list (`apps/sync/Dockerfile.dockerignore`), so `.env` files never
enter it.

## Local end-to-end run (no Docker)

`make e2e` (`scripts/e2e-local.mjs`, D-382) runs the real worker against the local Convex harness and the fake
IMAP + SMTP servers of `crates/providers` (`akatsuki-fake-mail`, built with
`cargo build -p akatsuki-sync --features e2e-fakes --bins`): email-code sign-in, `POST /v1/connect/imap`, initial
sync, sanitized bodies, archive and undo, draft and send, IDLE delivery, account removal. `make e2e-web` adds the
web app's Playwright suite (`apps/web/e2e/local/mail.spec.ts`) against the same stack; `make e2e-serve` keeps the
worker and the fake mailbox running for manual use. Details in `docs/DEV.md`.

## Desktop app

`apps/desktop` (ARCHITECTURE §6.3, §16.3; decisions D-310 to D-321). Modules:

| Module | What |
|---|---|
| `auth` | Loopback sign-in (social, magic link) with the crossDomain one-time token, email OTP, JWT minting and refresh, sign-out |
| `backend` | tokio side: session, Convex connection, per-account `sync.changes` feed into the mirror, optimistic actions with the durable outbox, undo, bodies, search indexing, compose, BYO AI; events to the UI |
| `keys`, `commands` | Rust twin of the web key matcher over `crates/shortcuts`; the command palette list |
| `vm/*` | View models without a window: lists, cursor, selection, splits, intents, action plans per account, compose drafts, dates |
| `render/*` | Local sanitizing of raw bodies, the native layout model, "Open original" files |
| `ui/*` | GPUI views; `ui/input.rs` is the text field adapted from ref/Siberia |
| `demo` | The demo mailbox: an in-process stand-in for Convex and the worker (D-316) |

The SQLite mirror is `akatsuki-core`'s `store` feature (D-310); its tests run in `cargo t` (the desktop
enables the feature) or with `cargo test -p akatsuki-core --features store`.

Run it:

```sh
cargo desktop -- --demo                                   # no server needed
AKATSUKI_CONVEX_URL=http://127.0.0.1:3410 \
AKATSUKI_CONVEX_SITE_URL=http://127.0.0.1:3411 cargo desktop  # against the local harness (make convex-up)
```

With the local harness (`EMAIL_TRANSPORT=console`) the sign-in code and magic link are printed in the
function logs (`cd convex && npx convex logs --env-file ../.env.local`). The session token is stored in the
OS keychain (Secret Service on Linux: GNOME Keyring, KeePassXC or similar must be running). Data lives in
`$XDG_DATA_HOME/akatsuki` (`mirror.sqlite`, `search.sqlite`), settings in `$XDG_CONFIG_HOME/akatsuki/desktop.json`;
`AKATSUKI_DATA_DIR` moves both.

Live tests against the local harness (ignored by default):

```sh
(cd convex && npx convex logs --env-file ../.env.local > /tmp/convex.log) &
AKATSUKI_LIVE_SITE=http://127.0.0.1:3411 AKATSUKI_LIVE_CONVEX=http://127.0.0.1:3410 \
AKATSUKI_LIVE_OTP_LOG=/tmp/convex.log cargo test -p akatsuki-desktop -- --ignored
```

They sign in with an emailed code, mint a JWT, call the desktop's Convex functions, follow a magic link
through the loopback (crossDomain one-time token) and sign out.

Screens without a keyboard: `AKATSUKI_SCRIPT="go.split:split:other;nav.open"` runs command ids at start-up
(`search=<q>` types a search). A private headless compositor keeps smoke runs off your screen, e.g.
`XDG_RUNTIME_DIR=<private dir> WLR_BACKENDS=headless sway -c <config>`, then run the app and `grim` with that
`XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY`.
