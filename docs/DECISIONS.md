# Decisions log

Each entry records what was decided, the alternatives, and why. Entries marked **BRIEF deviation** change or refine something in `docs/BRIEF.md`; the BRIEF requires them to be written here. Newer entries go at the bottom. Do not edit a past decision. To change one, add a new entry that supersedes it.

The first batch (D-001 to D-052) was decided on 2026-10-20 by the tech lead. Three independent architecture proposals were reviewed:

- **MVP**: fewest moving parts.
- **SYNC**: sync correctness and failure handling.
- **CLIENT**: client performance and developer experience.

The resulting contract is `docs/ARCHITECTURE.md`. Section numbers (§) refer to that file.

Entries D-001 to D-033 settle points the three proposals disputed. D-034 to D-052 settle points that were not disputed but that the contract had to fix.

---

## Contested points

### D-001 Timers: Convex decides when, the worker performs the provider effect (**BRIEF deviation**)
- **BRIEF**: "Sync worker … fires timers (snooze, reminder, scheduled send)".
- **Decision**: every deadline (snooze, reminder, undo-send window, send-later, entitlement edges, purge) is a Convex scheduled function. Its effect is a Convex state change plus an `ops` row, and the worker executes that op against the provider (§10).
- **Why**:
  - A Convex scheduled function runs exactly once, and only if the mutation that scheduled it committed, so the timer is transactional with the state change that set it.
  - A worker restart, redeploy or crash can no longer lose or double-fire a snooze or a send.
  - The worker stays stateless.
  - Self-hosters get the same behaviour.
  - All three proposals agreed on this.
- **Graft from SYNC**: a `timers.sweep` cron fires anything more than 60 s overdue whose scheduled function is missing. Every fire mutation is guarded by row state, so a duplicate fire does nothing.
- The worker still drives provider-side timers itself: watch renewal, IDLE refresh and polling.

### D-002 Undo-send and send-later text waits in the provider's Drafts folder
- **Options**:
  - (a) Provider Drafts folder (MVP, SYNC).
  - (b) An encrypted spool on the worker's disk for the undo window, with provider drafts only for sends more than 120 s ahead (CLIENT).
  - (c) The client holds the text.
- **Decision**: (a) for both undo-send and send-later (§10).
- **Why**:
  - Compose autosave already writes a provider draft, so the text is already at the provider.
  - Drafts survive a worker crash or disk loss.
  - They keep message text out of Convex (BRIEF decision 2).
  - They avoid a second at-rest store of bodies on the shared Hetzner host.
  - The trade-off is that the pending message is briefly visible in Gmail's or Outlook's Drafts folder. This is documented in the UI and acceptable.
- **Option (c) rejected**: a closed laptop would never send.
- The only worker disk store is the encrypted 24 h **upload** spool for attachments (§10.4).

### D-003 Auto-triage: managed path on the server, BYO on clients
- **Options**:
  - Client-only triage (MVP).
  - The worker calls apps/api for managed users (SYNC, CLIENT).
  - BYO users triage on whichever client is online, under a lease (CLIENT).
- **Decision**: users whose triage route is `akatsuki` (managed, Pro) are triaged by the worker through `POST /internal/v1/triage`. BYO and local users are triaged by their online clients with `ai.claimTriage` and a lease (§12.5).
- **Why**:
  - Splits should be correct for mail that arrives while no client is open, so that the inbox is sorted when the user opens it. That needs a server-side path.
  - BYO keys must never reach a server (BRIEF), so BYO triage can only run on a client.
- **Input**: headers and snippet only. Bodies are never sent; SYNC proposed sending the first 2,000 characters, which is rejected.

### D-004 Op vocabulary: label-key deltas plus a few named actions
- **Options**:
  - A single `modify` kind for everything (MVP).
  - Label-key `modify` plus `trash`, `untrash`, `delete_forever`, `move` and `unsubscribe` (SYNC).
  - A semantic enum such as `archive`, `star` or `markRead` (CLIENT).
- **Decision**: SYNC's vocabulary (§3.1 `vThreadAction`). UIs never build actions by hand: `packages/mail-model/src/actions.ts` exposes CLIENT-style helpers (`archive()`, `star()` and so on) that produce the canonical actions per provider (§5.1).
- **Why**:
  - One pure `applyOps` over label keys gives a single semantics for optimistic UI, server state and rollback.
  - A small number of named kinds covers real provider differences: folder moves, permanent delete, and RFC 8058 unsubscribe.
  - A semantic enum would need per-action code in every layer.

### D-005 Backfill horizon: 90-day window, then 1 year, full history opt-in
- **Options**: backfill the full mailbox metadata (MVP; CLIENT with a 250k cap) or a 1-year default (SYNC).
- **Decision**:
  - 90-day window first, as the BRIEF requires.
  - Lazy backfill down to `backfillHorizonAt`, which defaults to connect − 365 d.
  - Users can choose "everything" (capped at 250k messages per account).
  - Older mail is reached through provider search, which upserts hits on demand (§9.3).
- **Why**:
  - Initial sync uses 6 to 10 times less Gmail quota and time.
  - Convex stays small (~60 MB instead of ~400 MB).
  - Search stays complete through the provider fallback.

### D-006 Rate limiting uses `@convex-dev/rate-limiter`
- **Options**: hand-written fixed windows on the billing row (MVP) or the component (SYNC, CLIENT).
- **Decision**: the component (`0.4.2`) for `mail.apply`, `search.threads`, `labels.*` and the managed-AI request rate. The monthly AI **budget** is the `aiUsage` table (§13.4).
- **Why**: hand-written windows on the billing row put frequently written counters on a document that every entitled query reads, which invalidates subscriptions. The component is tested and shards its own counters.

### D-007 Service identity: self-minted ES256 JWTs registered as Convex `customJwt` providers, one issuer per service
- **Options**: one shared service issuer (MVP), separate issuers (SYNC, CLIENT), shared secrets passed as arguments, or deploy keys.
- **Decision**:
  - The worker uses `iss https://sync.akatsuki.vc` and apps/api uses `iss https://api.akatsuki.vc`.
  - Both use `aud "akatsuki-service"`, ES256, 10-minute tokens, and data-URI JWKS in Convex env (§6.6).
  - Deploy keys are used only in CI.
- **Why**:
  - Identity flows through `ctx.auth`, so secrets never appear in function arguments or logs.
  - Separate issuers give least privilege: apps/api cannot call `worker.*` functions.
  - A deploy key can push code and read env, so it never sits on a running server.
- **Verified**: the plugin's `parseAuthConfig` only inspects `applicationID: "convex"` (§0 fact 2). Spike S3 covers the runtime check.

### D-008 Bodies: the worker sanitizes for web and mobile, desktop sanitizes locally, and image URLs are capability URLs
- **Decision**:
  - `GET /v1/messages/{id}` returns sanitized HTML from `crates/mime::sanitize`. Desktop fetches `?format=raw` and runs the same crate (§14.3).
  - **Change from MVP**: inline `cid:` parts are not served as long-lived capability URLs. Sanitized HTML carries `data-akatsuki-cid`, and the client fetches the part with its bearer token and injects a `data:` URL.
  - Remote images become HMAC-signed proxy URLs with no expiry; rotating the key revokes them.
  - Body responses are `no-store`, and clients cache bodies explicitly (D-041).
- **Why**:
  - Cached sanitized HTML never contains expiring URLs.
  - No URL can fetch private mail content without a bearer token.
  - The proxy is still not open, because only URLs the worker signed work.

### D-009 BYO AI keys on web: IndexedDB with a non-extractable WebCrypto key, opt-in; calls go directly from the browser to the provider
- **Decision**: all three proposals agreed, with SYNC's refinement that the key is kept for the session only unless the user chooses "Remember on this browser" (§12.2).
- **Why**: this is the best protection a browser offers. The UI says so and recommends desktop or managed AI for the strongest protection. Keys never reach any server (BRIEF).

### D-010 No attachment metadata in Convex
- **Options**: `hasAttachments` only (MVP, SYNC) or an `attachments[]` array of names and sizes per message (CLIENT).
- **Decision**:
  - `messages.hasAttachments` and `threads.hasAttachments` only, best effort: Gmail uses the `Content-Type` plus a `has:attachment` id sweep, Graph uses its `hasAttachments` field, and IMAP uses `BODYSTRUCTURE`.
  - The exact list arrives with the body.
  - `filename:` search goes to the provider or desktop FTS.
- **Why**: Gmail `format=metadata` does not return MIME parts. Getting filenames would need `format=full`, which downloads body data, costing quota and putting bodies in transit for a list view.

### D-011 Rollback: recompute from provider-confirmed state, not stored inverse snapshots
- **Options**:
  - A stored inverse patch plus a `pendingOps` counter (MVP).
  - Recompute `labelKeys = applyOps(syncedLabelKeys, pendingOps)` (SYNC).
  - Re-pull provider truth (CLIENT).
- **Decision**: SYNC's recompute model (§5.1, §5.3).
- **Graft from CLIENT**: on a permanent failure the worker re-reads the thread from the provider and writes it in the same `completeOps` call.
- **Why**:
  - A failed op rolls back by construction, even when later ops on the same thread are still pending.
  - Inverse snapshots go wrong exactly in that case.
  - The `pendingOps>0 → ignore provider` counter in MVP can mask genuine remote changes.

### D-012 One user JWT (15 min, `aud: "convex"`) for Convex, the worker and apps/api
- **Decision**: all three proposals agreed. The worker and apps/api verify the JWT against the Better Auth JWKS, then check ownership and entitlement against Convex (§6.5).
- **Why**: it keeps one token cache per client, and the 15-minute revocation lag is acceptable.

### D-013 The Stripe webhook lands in apps/api
- **Options**: apps/api, which forwards to Convex through service functions (all three proposals), or a Convex HTTP action.
- **Decision**: apps/api (§13.2).
- **Why**:
  - The Stripe secret key and webhook secret then live only on the Hetzner host.
  - Convex HTTP stays limited to `/api/auth/*`.
  - The webhook re-fetches the subscription snapshot, which needs the Stripe SDK and secret anyway.

### D-014 An op is `done` when the provider confirms, without waiting for the push echo (**BRIEF refinement**)
- **BRIEF**: "provider push comes back → worker reconciles → op marked done".
- **Decision**: `worker.completeOps` marks the op `done` and writes the provider's post-op thread state in one transaction (§5.3). The later echo is an idempotent, version-guarded upsert.
- **Why**: echoes can be delayed by minutes, merged, or never come (polling mode, IMAP flags on other connections). Waiting for them would leave ops stuck with no added safety, because the post-op re-read already reconciles.

### D-015 Sends are at-most-once, with automatic verification before asking the user
- **Options**:
  - Exactly-once by searching Sent before a retry (MVP, CLIENT).
  - At-most-once with a user-visible `unknown` state (SYNC).
- **Decision**: both, combined (§5.6):
  - `find_sent(rfcMessageId)` runs before every attempt.
  - `markSendStarted` is called just before the provider call.
  - When the outcome is ambiguous, the op becomes `verifyOnly`. It never re-sends, and re-checks Sent at about +30 s, +2 min and +10 min.
  - Only after those checks does the send become `unknown` for the user to decide.
- **Why**: a double send is worse than asking. Provider search indexing can lag, so a single check could miss a message that was sent.

### D-016 Desktop sign-in: loopback using Better Auth's own flows plus the crossDomain one-time token; no Local Install
- **Options**:
  - Loopback plus `oneTimeToken` minted by the web app (MVP). This reuses the **web session**, verified from source, so signing out on the web would sign out the desktop.
  - Device authorization, RFC 8628 (SYNC). This needs a `deviceCode` table and therefore a Local Install.
  - Custom `/auth/native/*` HTTP actions and a `nativeAuthCodes` table (CLIENT). This is custom auth code.
- **Decision**: the desktop starts `sign-in/social` (with `disableRedirect`) or `sign-in/magic-link` itself, with `callbackURL = http://127.0.0.1:<port>/cb?n=<nonce>`. Better Auth creates a **new session**, crossDomain appends `ott`, and the desktop exchanges it at `/api/auth/cross-domain/one-time-token/verify` (§6.3). Email OTP is the fallback.
- **Why**:
  - Each desktop gets its own revocable session.
  - The default component install is kept.
  - There is no custom auth code.
  - Every step uses stock endpoints, verified in the installed sources.
- Spike S1 confirms the flow end to end.

### D-017 Deploy keys are used only in CI
- **Decision**: see D-007. `CONVEX_DEPLOY_KEY` exists only in CI secrets.

### D-018 Hosted mode allows one Akatsuki user per mailbox
- **Decision**: `mailAccounts.by_provider_account` is unique in practice. `worker.createAccount` throws `already_connected` when another user already owns the mailbox. The same user reconnecting replaces the secret.
- **Why**: two users syncing one mailbox would run duplicate watches and ops. Gmail Pub/Sub pushes are keyed by email address, so a push would fan out to two users, and remote changes would bounce between them. Mailbox sharing is out of scope for v1.

### D-019 Lists use a `threadLabels` membership table; counts are capped index reads, not counters
- **Options**:
  - Boolean or index fields on threads plus `threadLabels` (MVP).
  - `threadLabels` with virtual `split:*` keys and no counters (SYNC).
  - A `threadBoxes` table with 8 indexes plus `boxCounts` counters (CLIENT).
- **Decision**: SYNC's `threadLabels` with 5 indexes (§3.2). `mail.unreadCounts` reads at most 100 index rows per key, and the UI shows "99+".
- **Why**:
  - Every list is one index range, unified or per account, and unread-only or not.
  - Write amplification is about half of `threadBoxes`.
  - There are no counter documents, which would be hot during sync bursts (OCC conflicts) and could drift.
  - A keyboard-first inbox does not need exact counts above 99.

### D-020 Split search into a `threadSearch` table
- **Options**: search text on `threads` (MVP), a separate `threadSearch` table (SYNC), or a search index on `messages` (CLIENT).
- **Decision**: `threadSearch` (§3.2).
- **Why**: Convex bills database bandwidth by the documents read. List renders must not read 1.5 KB of search text per row.

### D-021 New packages: `packages/mail-model` and `packages/wire` (**BRIEF layout deviation**)
- **Options**: `packages/search-query` (SYNC); `packages/query-grammar`, `packages/mail-model` and `packages/clients` (CLIENT); a parser inside `convex/lib` (MVP).
- **Decision**: two new packages.
  - `packages/mail-model` holds pure domain logic: label keys, `applyOps`, action helpers, rule matching, thread aggregates and the search grammar, with fixtures.
  - `packages/wire` holds wire types for the worker and apps/api, the error codes, the AASP parser and the token helper.
- **Why**:
  - Convex mutations, web and mobile optimistic updates, and the Rust twins all need the same pure functions and fixtures. Putting them in `convex/lib` would make clients import server code.
  - Wire types are shared by three TypeScript apps.
  - Two packages are the minimum that keeps those dependency directions clean. OpenAPI generation is optional, not required.

### D-022 Triage results are stored in Convex (enum and boolean only)
- **Decision**: `threads.triage = {category, needsReply, source, model, promptVersion, at}`.
- **Why**: triage drives splits, which Convex evaluates. It is metadata, not message text.

### D-023 Entitlement is never carried in JWT claims
- **Options**: `ent` / `entExp` claims through Better Auth `additionalFields` (CLIENT), or Convex lookups.
- **Decision**: rejected the claims. The worker and apps/api check entitlement against Convex: the worker through `assignedAccounts` and `userContext`, apps/api through `aiPrecheck`.
- **Why**: claims would need a Local Install (D-034), be up to 15 minutes stale, and duplicate the source of truth.

### D-024 The worker and apps/api call ordinary public Convex functions under their service identities
- **Decision**: agreed by all three proposals (§4.1).
- **Why**: the Rust and Node clients can only call public functions. The `worker*` and `api*` wrappers check the issuer.

### D-025 Desktop keeps a full metadata mirror through a per-account `rev` feed
- **Options**: a mirror through `sync.changes` with `accountRevs` and tombstones (CLIENT), or paginated live queries only.
- **Decision**: the mirror (§16.3). `rev` and its indexes are in the schema from day one.
- **Why**:
  - Offline use and sub-millisecond lists are the reason the desktop exists.
  - Adding `rev` later would mean backfilling every row.
  - Convex serializability makes the counter a correct change feed, which timestamps are not.
- **Cost**: writes to one account serialize on one small document, which is acceptable because the actor already serializes provider work per account.

### D-026 Splits are materialized when a thread is written, with batched reassignment when a split changes
- **Decision**: agreed by all three proposals (§3.4).
- **Rule language**: CLIENT's structured `vRule`, not SYNC's search-grammar strings. Structured rules are safe to evaluate inside a mutation, in both TypeScript and Rust.

### D-027 Encrypted provider credentials are stored in Convex (`accountSecrets`); the key exists only on the worker
- **Decision**: all three proposals agreed. This is forced by "Convex is the only datastore" (BRIEF).
- **Details**: secrets are split from `mailAccounts` so that token refreshes never invalidate client subscriptions. A version field gives compare-and-set for rotating refresh tokens (§14.1).

### D-028 AI results in Convex are limited to triage and an opt-in voice profile (**BRIEF clarification**)
- **BRIEF**: the architecture summary lists "AI results" in Convex, while the AI section says "AI outputs are cached on-device only unless the owner decides otherwise".
- **Decision**: follow the AI section, which is more specific and more recent.
  - Convex holds only `threads.triage`.
  - Convex holds `aiSettings.voiceProfile` only when the user turns on `consent.syncVoiceProfile`. The profile is a style description that the prompt forbids from quoting mail.
  - Summaries, replies, action items and every other output stay on the device.
- The owner may revisit this.

### D-029 `api.akatsuki.vc` belongs to apps/api; the auth issuer stays `*.convex.site` (**BRIEF inconsistency resolved**)
- **BRIEF**: "Hostnames" says `api.akatsuki.vc` is a Convex custom domain (optional), while "Hosting (decided 2026-10-20)" puts apps/api there.
- **Decision**: the later Hosting section wins.
- **Consequence**: if a Convex custom domain is ever added (for example `auth.akatsuki.vc`), it changes the JWT `iss`, which every verifier checks. That change must be decided **before launch**.

### D-030 The worker builds MIME for every client
- **Decision**: agreed by all three proposals. `crates/mime::compose` is the only MIME builder, and web, mobile and desktop all send `DraftContent` JSON (§7.3).

### D-031 BYO triage runs on whichever of the user's clients is online, under a lease
- **Decision**: see D-003. Desktop always triages; web and mobile triage only while in the foreground.

### D-032 Opposing actions coalesce in the worker, not at enqueue
- **Option rejected**: marking an older opposing op `superseded` when the newer one is enqueued (CLIENT). This is wrong when the older op was a no-op against provider state. For example, starring an already-starred thread and then unstarring it would cancel both ops and leave the thread starred.
- **Decision**: the worker merges consecutive `modify` ops on a thread into a net delta at claim time. A net-zero delta completes as `moot` without calling the provider (§5.4). `mail.undo` still cancels a `queued` op outright, which is correct because cancelling restores exactly the pre-op visible state.

### D-033 Enum casing: string enum values are `snake_case`; fields are `camelCase`
- **Context**: the proposals mixed conventions.
- **Decision**: §1.1.
- **Why**: one rule, and it maps directly to serde's `rename_all` attributes on both sides.

---

## Points the proposals did not dispute but that needed a decision

### D-034 Better Auth uses the default component, with no Local Install
- **Decision**: none of the chosen plugins (`crossDomain`, `expo`, `magicLink`, `emailOTP`, `convex`, and `bearer`/`jwt`/`oneTimeToken` included by them) needs tables beyond the default component schema. This was verified from source.
- **Bootstrap**: user rows are created by the component's `user.onCreate` trigger, with `me.bootstrap` as an idempotent fallback.

### D-035 Pin `better-auth@1.6.33` (**BRIEF decision 1 implementation**)
- **Facts**: `@convex-dev/better-auth@0.12.5` declares `better-auth >=1.6.11 <1.7.0` in the npm registry (read on 2026-10-20). npm `latest` is 1.7.11, and the newest 1.6 release is 1.6.33.
- **Rejected**:
  - MVP's 1.7.11, because it is outside the peer range.
  - SYNC's 1.6.15, an older patch release with no reason to prefer it.
- **Rule**: `@better-auth/expo` uses the same version.

### D-036 Ops are ordered FIFO per thread and per label, not per account
- **Context**: SYNC proposed strict FIFO per account, where the claim stops at the first op that is not yet available. One op backing off for 1.5 hours would then block every other action on the account.
- **Decision**: FIFO is enforced per `threadId` and per `labelId`. A thread op that adds a label whose `label_create` has not finished waits for it (§5.4).
- **Why**: undo correctness (archive, then undo) only needs ordering per thread.

### D-037 Snooze is mirrored at the provider through an `Akatsuki/Snoozed` label or folder
- **Context**: CLIENT archived a snoozed thread at the provider, and MVP removed its inbox label.
- **Decision**: SYNC's `sys:snoozed`.
- **Why**:
  - Gmail and Outlook show the thread in an explicit "Akatsuki/Snoozed" location instead of losing it in the archive.
  - When the user moves it back to the inbox in another client, sync sees that and wakes the thread early.

### D-038 The managed AI budget is $8 a month on Pro and $1 during the trial; 30 requests per minute; 1,000 per day
- **Context**: MVP proposed $6, CLIENT $8 with $1 for the trial, and SYNC daily token caps.
- **Decision**: CLIENT's numbers, re-checked against current Anthropic list prices (Sonnet 5.5 at $2/$10 and Haiku 5.5 at $0.10/$0.50 per million tokens). Heavy use comes to about $8 a month and typical use to $2 to $4 (§17).
- **Why the trial cap**: the trial cap limits abuse of trial accounts.

### D-039 Gmail scope is `gmail.modify`, not `https://mail.google.com/`
- **Decision**: `gmail.modify` covers reading, labelling, drafts and sending. Permanent delete is therefore `unsupported` on Gmail, and the UI hides it through `caps.deleteForever`. Trash auto-empties after 30 days.
- **Why**: a narrower restricted scope makes the eventual Google verification and CASA review easier. Only Gmail **IMAP** needs `mail.google.com`, and we use the Gmail API.

### D-040 The AI stream protocol is SSE ("AASP")
- **Context**: MVP used text/plain for text features and JSON for objects. SYNC used text/plain plus NDJSON.
- **Decision**: CLIENT's SSE protocol (§12.6), used for both the managed path and the in-process BYO path.
- **Why**: one parser in TypeScript and Rust; partial objects and errors can arrive mid-stream; usage arrives at the end.

### D-041 Body responses are `no-store`, and device caches are explicit
- **Context**: MVP and CLIENT proposed `private, max-age` caching.
- **Decision**: SYNC's `no-store` (§7.1).
- **Why**: sign-out wipes and `prefs.cacheBodiesOnDevice` only work if mail text never sits in the browser HTTP cache.

### D-042 Prompt templates support only `{{var}}`; the prompt pack carries no sampling parameters
- **Context**: SYNC proposed a mustache subset with `each` and `if`.
- **Decision**: `{{var}}` only, with lists pre-rendered by shared, fixture-tested renderers (§12.4). `temperature` is removed because Sonnet, Opus and Haiku 5.5 reject non-default sampling (claude-api reference, 2026-10-06 cache).
- **Rust structured output**: uses `output_config.format`, never a forced tool, because forced `tool_choice` returns a 400 on these models.

### D-043 Every new account must be entitled to connect
- **Decision**: the onboarding order is checkout, then connect. `worker.createAccount` and `/v1/connect/*` return `not_entitled` without `hostedSync`.
- **Why**: there is no free hosted tier (BRIEF decision 7).

### D-044 One worker process in v1, with per-account leases
- **Decision**: `SHARD_COUNT=1`. The `shard` field and the account-level leases (SYNC) keep a rolling deploy correct and make later sharding a configuration change.

### D-045 Dynamic Better Auth JWKS for MVP
- **Decision**: user-token keys live in the component's `jwks` table. Convex, the worker and apps/api fetch `/api/auth/convex/jwks`.
- **Later**: a static `JWKS` env (CLIENT's recommendation) is an optimization once its generation procedure is scripted (spike S4).

### D-046 Filter rules are included in v1
- **Context**: MVP deferred rules, while SYNC and CLIENT included them.
- **Decision**: include the `rules` table. Rules use the same `vRule` as splits and apply only to **newly arrived inbound** mail inside `worker.upsertThreads`, producing ops (§4.4).
- **Why**: the BRIEF lists rules in the Convex control plane. Reusing the split condition language keeps the cost small.

### D-047 Transactional email is sent only from Convex
- **Decision**: magic links, OTP, trial reminders, reauth notices and purge warnings all go through `internal.email.send`, which calls the SES API from Convex (§14.6).
- **Why**: SES sending credentials then live in exactly one place. apps/api asks Convex to send the trial reminder through `ee.service.trialWillEnd`.

### D-048 The internal triage endpoint is protected twice
- **Decision**: `/internal/*` on apps/api requires a worker service JWT with `aud: "akatsuki-api"`, and host nginx also denies `/internal/` publicly. The worker reaches apps/api over the Docker network.

### D-049 Ids: clients pass only Convex ids to the worker
- **Context**: MVP used provider ids in worker URLs.
- **Decision**: the worker resolves every Convex id through `worker.resolveMessages` and `worker.resolveThread` with the caller's `userId`.
- **Why**: one ownership check, and provider ids never need to reach clients' request paths.

### D-050 Pin AI SDK 7 and provider packages to versions verified in the npm registry
- **Decision**: §0 lists the verified versions.
- **Verified exports**: `transcribe` is exported in `ai@7.0.145` (stable, alongside the `experimental_transcribe` alias). `@openrouter/ai-sdk-provider@3.1.0` exposes `provider.zdr` and `provider.data_collection`.
- **Haiku model id**: the direct id `claude-haiku-5-5` is confirmed by the claude-api reference, which settles MVP's open question.

### D-051 IMAP threading never merges by subject
- **Context**: CLIENT proposed subject normalization.
- **Decision**: threads are built from `X-GM-THRID`, otherwise from the `References`/`In-Reply-To` root. Merging by subject is rejected because it wrongly joins unrelated "Invoice" or "Hello" threads.

### D-052 Search `nowAt` comes from the client
- **Decision**: Convex queries cannot read the clock, so `search.threads` takes `nowAt` (rounded to the minute) for relative-date operators (§11.1).

---

## Implementation decisions (TypeScript root, tokens, shortcuts, Convex backend; 2026-10-20)

### D-053 Passkeys are not enabled in v1
- **Context**: the scaffolding task asked for passkeys "if supported".
- **Decision**: no passkey plugin. `@better-auth/passkey` needs a `passkey` table, and the default `betterAuth` component schema (0.12.5) has none, so it would force a Local Install, which D-034 rejects.
- **Revisit**: when the component ships a passkey table, or if the owner accepts a Local Install.

### D-054 GitHub social login is added next to Google and Microsoft (**§6.1 addition**)
- **Decision**: `convex/auth.ts` also enables `github` when `GITHUB_LOGIN_CLIENT_ID`/`GITHUB_LOGIN_CLIENT_SECRET` are set. It is login only (no mailbox access), env-gated like the other two.

### D-055 `bumpRev` may increment more than once per mutation (**§3.3 refinement**)
- **Context**: §3.3 says `bumpRev` increments once per mutation. Memoizing per mutation needs a per-transaction key that Convex does not expose reliably.
- **Decision**: every call increments. Writes to the counter document collapse into one write at commit, so the cost is the same. The mirror feed (§16.3) only needs revs to be strictly increasing per account, which still holds, and `sync.changes` never splits one rev across pages.

### D-056 Pure mail logic is temporarily copied into `convex/lib` (**§1.7 interim**)
- **Context**: `packages/mail-model` (owner of `applyOps`, label keys, `matchRule`, `threadAggregates`, the query grammar) did not exist when the backend was built.
- **Decision**: `convex/lib/model.ts` and `convex/lib/query.ts` follow §1.4, §3.4, §5.1 and §11.1 verbatim and are unit-tested. When `@akatsuki/mail-model` lands, their bodies become re-exports from it so the shared fixtures cover Convex too.

### D-057 Small contract-shape adjustments in Convex functions
- `worker.accountsForPush` args are one object `{ by, email?, subscriptionId? }`, because Convex `args` must be an object validator. Callers send exactly the contract's `{ by: "gmail_email", email }` or `{ by: "graph_subscription", subscriptionId }`.
- `worker.claimOps` returns an extra optional `draft: { providerDraftId?, providerMessageId? }` for `delete_draft` ops, which §5.4 left without the provider draft id the worker needs.
- `me.get` returns `null` until the user rows exist (between sign-up and the `user.onCreate` trigger or `me.bootstrap`), instead of throwing.
- `ai.getSettings` likewise returns `AiSettingsView | null`: `null` until the user's `aiSettings` row exists. Rust decodes it as `Option<AiSettings>` (`ClientApi::ai_settings`, fixed with D-310; §4.2 row: D-353); clients treat `null` as "no profiles, no routes, no consent".
- `search.threads` returns `ast` typed as `v.any()` (a recursive `QueryAst` is not expressible as a Convex validator). Its `searchThreads` rate limit is not enforced, because queries cannot write rate-limiter state; the client debounces.
- `ee.billing.entitlement` reports the newest `aiUsage` period on record, because queries cannot read the clock.
- `sync.changes` sets `resetRequired` only when `sinceRev` is ahead of the account's rev. Detecting tombstones garbage-collected past `sinceRev` needs a GC watermark the schema does not have; a client offline for more than 30 days should re-pull.
- The contract leaves `rules.*`, `snippets.*` and `senderPrefs.*` return shapes open; they return `{ ruleId }`/`{ snippetId }` on save, `null` on remove, and the row views from `convex/lib/rows.ts`.

### D-058 Local Convex harness: native binary fallback, ports 3410/3411/6891
- **Context**: on the dev machine the user is not in the `docker` group, so the Docker daemon is not reachable without sudo.
- **Decision**: `infra/docker-compose.dev.yml` is the documented harness (pinned images, `akatsuki-convex-*` containers). `scripts/convex-dev.mjs` uses it when Docker is usable and otherwise runs the `convex-local-backend` binary of the same upstream revision on the same ports. Ports 3410 (API), 3411 (HTTP actions) and 6891 (dashboard) avoid the Convex defaults so other local Convex projects keep theirs.
- **Also**: Convex rejects a push while an env var used by `auth.config.ts` is unset, so the harness provisions local-only ES256 service keys and issuers (`https://sync.akatsuki.localhost`, `https://api.akatsuki.localhost`).

### D-059 Tooling: Biome, TypeScript 6.0, Vitest 5
- **Decision**: Biome 2.5 for lint and format (one fast tool for JS, TS and JSON; no ESLint plugin needs yet). TypeScript is pinned to 6.0.3, the last JavaScript-based release, because the Convex CLI drives `tsc` through its JS API and TypeScript 7 is the native port. Vitest 5 with `convex-test` in the edge runtime.
- **Env naming**: the contract's `TOKEN_ENC_KEYS` (a key ring) is the name used everywhere; there is no separate `TOKEN_ENCRYPTION_KEY`.

---

## Implementation decisions (Rust workspace; 2026-10-20)

Numbered from D-090 so they cannot collide with the numbers other workstreams are assigning in parallel.

### D-090 Rust crate dependency direction; the actor driver lives with the provider traits (**§9.1/§9.2 refinement**)
- **Context**: §9.1 has `crates/providers` reference `akatsuki_core::wire::ErrorCode` and `search::query::Ast`, while §9.2 puts the account actor in `crates/core::sync`, which would need the provider traits: a dependency cycle.
- **Decision**: `akatsuki-core` depends on no workspace crate. It holds the pure state machine (`AccountPhase`, `PhaseEvent`, `ActorCmd`, `WorkQueue` priorities, `backoff_ms`, lease constants). The shared domain types the traits use (`Addr`, `RemoteLabel`, `RemoteThreadId`, `ImapLocation`, `SysLabel`, `LabelRole`, …) live in `akatsuki_core::model` and are re-exported by `akatsuki-providers`. The loop that owns a `MailProvider` and a `MetaSink` goes in `akatsuki-providers` (or `apps/sync`) and calls the core state machine. `ActorCmd::SearchFetch` replies with `Id<"threads">` from `akatsuki_core::ids`.

### D-091 Error code `not_implemented` (HTTP 501) (**§15 addition**)
- **Decision**: the worker serves every §7.2 route now. Routes whose provider logic is not built yet validate their input and answer `501 {"error":{"code":"not_implemented","message":"…","retryable":false}}`, so web, mobile and desktop can be built against the real shapes. `packages/wire` should add the code to `ERROR_CODES` and `packages/ui` a copy string ("This is not available yet.").
- **Lifetime**: the code disappears from responses route by route as the worker is completed; it stays in the registry.

### D-092 `crates/convex-api` is hand-written until `convex-gen` exists (**§18 interim**)
- **Context**: §18 says the bindings are generated from `npx convex function-spec` by `cargo xtask convex-gen`. The Convex functions were being written at the same time.
- **Decision**: `WorkerApi` (every `worker.*` function) and `ClientApi` (the client functions the desktop calls) are written by hand over a `ConvexTransport` trait, with argument and result types in `akatsuki_core::convex`, checked against `convex/worker.ts`, `convex/sync.ts` and `convex/mail.ts` validators (including D-057's `claimOps.draft` and `upsertLabels`' required `hidden` flag). The generator replaces the hand-written impls later and must keep the same names.
- **Values**: integral `Float64` results are returned to serde as JSON integers, `Int64` and `Bytes` as Convex's tagged JSON, so typed Rust fields round-trip (spike S9 covered by tests).

### D-093 Rust AI client: Ollama through its OpenAI-compatible `/v1`, structured output via `response_format` (**§12.4 refinement**)
- **Context**: §12.4 lists `format: <schema>` for Ollama, which is the native `/api/chat` endpoint.
- **Decision**: `crates/ai` talks to every non-Anthropic provider (OpenAI, OpenRouter, Google's and AI Gateway's OpenAI endpoints, Ollama `/v1`, LM Studio, llama.cpp, vLLM) through one OpenAI-compatible streaming path with `response_format: {type: "json_schema", json_schema: {name, strict: true, schema}}`; Ollama supports it on `/v1`. "Test connection" for Ollama still uses `GET /api/tags`. Anthropic uses `output_config.format` and `output_config.effort`, no sampling parameters, no forced tool. Every object result is validated against `output.schema.json` with one repair retry.
- **Shared rendering**: the Rust renderers follow `packages/prompts` exactly (`{{name}}` placeholders only, `<`/`>` escaped in values, `null` renders empty, system prompt = `system.md` + `shared/untrusted.md`; transcript and triage formats from `shared/transcript.md`, counting Unicode scalar values) and pass its render, transcript and triage-item fixtures. Resolution, egress and settings validation pass `packages/ai/fixtures/{resolve,settings}`. Quoted history is stripped by the caller (`crates/mime` quote detection) before a message reaches the transcript.

### D-094 Search grammar resolution rules (**§11.1 clarification**)
- **Decision** (`crates/search::query`; the TypeScript twin follows): `before:`/`after:` resolve to the start of the day in the user's zone (`date < at`, `date >= at`); `N(d|w|m|y)` is `N × 1/7/30/365` days; `is:`, `has:`, `category:` and built-in `in:` values are lowercased, other values keep their case; unknown values are reported in `errors` but stay in the AST; a malformed value (bad date, empty value) turns the whole token into a text word; a one-element `and`/`or` collapses; the empty query is `{"and": []}`; groups nest at most 32 deep. `normalized` renders fields lowercase, quotes values with spaces, writes dates as `YYYY-MM-DD` in the zone and ages as `<days>d`.
- **FTS5**: the desktop index stores one row per message with metadata columns, so negation and `OR` work offline; results are grouped per thread, ranked by bm25 (subject 5, from 3, to 2, body 1, filenames 2) when the query has free text, newest first otherwise. `to:` and `cc:` both search the To/Cc column.

### D-095 MIME part ids are IMAP body-section paths (**§7.3 clarification**)
- **Decision**: `AttachmentMeta.partId` and `data-akatsuki-cid` values from `crates/mime` are IMAP section paths (`"1"`, `"1.2"`, `"2"`), so the IMAP adapter can fetch `BODY.PEEK[<partId>]` and every other path finds the same part in the raw source (`part_content`). Gmail and Graph adapters map their own attachment ids behind the worker; clients treat part ids as opaque.
- **Sanitizer output**: HTML is wrapped in `<div class="akatsuki-body">`; `<style>` rules are scoped under `.akatsuki-body`, keep only allow-listed properties and drop every at-rule except `@media`; the first quoted-history container carries `data-akatsuki-quote` and its byte offset is `quoted.htmlStart`; bodies nested deeper than 1,024 elements are shown as escaped text (parser denial-of-service guard).

### D-096 Rust crates for tokens and shortcuts (**BRIEF layout addition**)
- **Decision**: `crates/tokens` (`akatsuki-tokens`) and `crates/shortcuts` (`akatsuki-shortcuts`) hold the Rust output of `packages/tokens` and `packages/shortcuts` (`generated.rs`, written by those packages' build scripts) plus a small hand-written shell with helpers and tests. The BRIEF names "generated Rust constants" without a location; a crate is the only way GPUI code can depend on them.

---

## Implementation decisions (AI layer: packages/ai, packages/prompts, apps/api; 2026-10-20)

Numbered from D-120 so they cannot collide with the numbers other workstreams are assigning in parallel.

### D-120 `routing.json` also carries escalation candidates and list prices (**§12.3 addition**)
- **Context**: the BRIEF asks for an escalation path to a heavier model and §12.3 says model ids live only in `routing.json`; §13.4 needs costs in micro-dollars for `aiPrecheck`/`aiRecordUsage`.
- **Decision**: two extra top-level keys. `escalation.heavy.<kind>` lists heavier models per BYO provider kind, cheapest first (Opus 5.5, then Fable 5.1). `pricing.<managed model id>` holds list prices (Sonnet 5.5 $2/$10, Haiku 5.5 $0.10/$0.50, Opus 5.5 $4/$20, Fable 5.1 $10/$50 per MTok; gpt-4o-transcribe $0.006/min). The tier table itself is unchanged.
- **Escalation hook**: `EscalationHook` in `packages/ai/src/routing.ts`. The default never escalates; `escalateOnInvalidOutput` retries one schema-invalid structured answer on the first heavy candidate. A feature moves tiers permanently only by editing `features` in `routing.json`, backed by an eval (§12.1).
- **Fable 5.1 is BYO-only**: it is not available under zero data retention, so the managed path (ZDR required, §12.3) never escalates to it.

### D-121 Client-side AI states are thrown, not streamed; BYO OpenRouter uses the same privacy routing (**§12.2/§15 clarification**)
- `runFeature` throws `AiFeatureError` before any request for `consent_required` (no `consent.grantedAt`, §12.7 rule 1), `ai_off`, `ai_model_required`, `key_missing` ("Key not set on this device") and `unsupported` (dictation on a provider without transcription). Like `ai_model_required` in §15 these are client-side codes with no HTTP status; `packages/wire` should add them to `ERROR_CODES`.
- BYO OpenRouter profiles get the same `provider: { zdr: true, data_collection: "deny", allow_fallbacks: true, require_parameters: true }` as the managed path, set at the provider factory so every model in the registry carries it. A model without a ZDR endpoint then fails instead of silently retaining data; changing this is one constant (`OPENROUTER_PRIVACY`).
- `maxInputChars` is checked against the sum of all string vars (Unicode scalar values), not the rendered prompt, so the template text never counts; `composeVars` gives the transcript whatever budget the instruction and draft leave.
- S6 resolved from the installed `@ai-sdk/anthropic@4.0.83`: `providerOptions.anthropic.structuredOutputMode: "outputFormat"` (never the forced-tool mode) and `providerOptions.anthropic.effort`. OpenRouter gets `providerOptions.openrouter.reasoning = { effort, exclude: true }`.

### D-122 Stripe webhook: snapshot events are applied before they are marked processed (**§13.2 refinement**)
- **Context**: §13.2 records the event id first and returns 200 on a duplicate. If the snapshot apply then fails, Stripe's retry is treated as a duplicate and the change is lost until the next event (bad after `checkout.session.completed`).
- **Decision**: for snapshot events apps/api re-fetches and applies first, then calls `recordStripeEvent`. Applying a full re-fetched snapshot is idempotent and older snapshots lose (`snapshotFetchedAt`), so a repeat costs one Stripe read. `customer.subscription.trial_will_end` sends an email, so it keeps the record-first order.

### D-123 apps/api runtime details (**§8 implementation notes**)
- Convex functions are called by name (`makeFunctionReference("ee/service:aiPrecheck")` …), so apps/api does not typecheck the whole Convex tree; `apps/api/test/convex-contract.test.ts` fails if a name or wrapper (`apiQuery`/`apiMutation`) changes. `GET /v1/ai/quota` calls `ee.billing.entitlement` with the user's own JWT instead of adding a service function.
- A client that disconnects mid-stream is charged for its input (estimated at 4 characters per token, no output) so aborting cannot dodge the budget.
- Transcription is metered by the provider's duration when it reports one, otherwise exactly from the WAV header or estimated from size (Opus 32 kbit/s, AAC 64 kbit/s, MP3 128 kbit/s). The 120 min/day transcription cap (§13.4) is **not enforced yet**: apps/api is stateless and `aiUsage` is monthly; it needs a daily bucket in `ee.service.aiPrecheck` (Convex owner).
- The image is one esbuild bundle (`dist/server.mjs`) on `node:26.7.0-alpine3.24` with the package managers removed, running as uid 1000 with no writes. `apps/api/Dockerfile.dockerignore` keeps `.env*`, keys and unrelated apps out of the build context.
- `pnpm install` added the contract-pinned AI SDK packages (published less than the workspace's minimum release age ago) to `minimumReleaseAgeExclude` in `pnpm-workspace.yaml`. The versions are the ones §0 pins.

### D-124 Transcript budget; one `maxInputChars` rule and one error code in TS and Rust (**§12.4 clarification**)
- **Context**: `renderTranscript` capped the transcript at 120,000 characters, the same number as `meta.maxInputChars` for summarize, instant_reply, extract_actions and compose. The cap is on the sum of all string vars (D-121), so a long thread plus `userName`/`today` always went over: `payload_too_large` in `streamFeature` and 413 in apps/api. `crates/ai` checked the rendered user prompt instead (template text and `&lt;` escaping included) and reported `invalid_vars`.
- **Decision**: every transcript feature renders the thread with a budget of `maxInputChars − Σ other string vars − 200` (`transcriptBudget` and the `summarizeVars` / `instantReplyVars` / `extractActionsVars` / `composeVars` builders in `packages/ai`; `transcript_budget` and `PromptPack::transcript_budget` in `crates/ai`). `crates/ai` `PromptPack::render` now checks the sum of var lengths before escaping and fails with `AiError::PayloadTooLarge` (`payload_too_large`). `composeVars` no longer has a 1,000-character floor that could push it over the cap.
- **Fixture**: `packages/prompts/shared/fixtures/at-cap/long_thread.json` (a thread whose default render is 119,995 characters) is run by `packages/ai/test/fixtures.test.ts` and `crates/ai` (`repo_pack_budgets_transcripts_at_the_cap`); both must produce the same budget, lengths and SHA-256 of the user prompt.

### D-125 Desktop OpenRouter requests carry the D-121 privacy routing (**§12.3 conformance fix**)
- **Context**: `crates/ai` sent BYO OpenRouter requests without `provider: {zdr, data_collection, …}` and without the effort mapping, so the same profile got zero-data-retention routing on web and mobile but not on desktop (§12.6).
- **Decision**: `Provider::OpenAiCompatible` has an `openrouter` flag (set only for `AiProviderKind::Openrouter`). Its bodies carry `provider = OPENROUTER_PRIVACY` (a Rust mirror of the TS constant) and, when `meta.effort` is set, `reasoning = {effort, exclude: true}`. `packages/ai/fixtures/openrouter/request.json` is the shared expectation; `packages/ai/test/providers.test.ts`, `crates/ai` unit tests and a wiremock test (`openrouter_byo_requests_carry_the_privacy_routing`) check the request bodies.

### D-126 Shared TS/Rust fixtures are checked in before `packages/mail-model` and `packages/wire` exist; the TypeScript query parser follows D-094 (**§1.7 interim, §11.1 conformance fix**)
- **Context**: §1.7 promised shared fixtures and a CI that runs both implementations, but `packages/mail-model/fixtures`, `packages/wire/fixtures` and the CI did not exist, Rust comments claimed to run missing files, and the twins had already drifted: `convex/lib/query.ts` did not lowercase `is:`/`has:`/`category:`/`in:` values, accepted relative `before:`/`after:` values (`before:7d`), had no 32-level group cap, dropped the `-` of `-(a b)`, treated a trailing `OR` differently and used other error messages. Desktop FTS and Convex results are merged by `threadId`, so the same query must mean the same thing on both.
- **Decision**:
  - The fixtures live at their contract paths now: `packages/mail-model/fixtures/{label-keys,apply-ops,rules,query}.json` and `packages/wire/fixtures/{aasp/*.txt + *.json, worker/*.json}`. These directories have no `package.json` yet. When the packages land, their sources replace the interim copies and the fixtures stay where they are.
  - Interim TypeScript runners: `convex/test/fixtures.test.ts` runs the mail-model fixtures against `convex/lib/model.ts` and `convex/lib/query.ts` (D-056). `packages/ai/test/aasp-fixtures.test.ts` runs the AASP streams against `packages/ai/src/sse.ts`. The worker fixtures are run by Rust only (`crates/core::wire::shared_fixtures`, serde round trip) until TypeScript worker types exist.
  - Rust runners: `crates/core::model::shared_fixtures`, `crates/core::wire::shared_fixtures`, `crates/search::query::tests::shared_fixtures` (fails if the file is missing or has fewer than 150 cases).
  - `convex/lib/query.ts` is now a character-level port of `crates/search::query`: same grammar, same D-094 rules, same error messages (compared exactly by the fixture), Unicode `White_Space` as whitespace and Unicode scalar values as characters, and a `normalizeQuery` twin. `before:7d` is now an error that stays text; use `older_than:`/`newer_than:` for relative ages.
  - Fixes found by the fixtures: Rust `parse` dropped everything after a second stray `)` (`a) b) c` lost `c`); Rust `relative_days` panicked on a multi-byte unit (`older_than:3é`); the TypeScript AASP reader turned a `\r\n` split across chunks into a blank line (ending the frame early) and threw on a frame whose data was not JSON instead of skipping it like Rust.
  - CI: `.github/workflows/ci.yml` runs typecheck, lint and every TypeScript test, then `cargo fmt --check`, clippy and tests for every crate except the GPUI desktop app (it needs X11/Wayland system libraries). `make fixtures` runs only the fixture-bearing suites on both sides.


## Verification fixes (2026-10-20)

### D-127 Secret AAD binds user, provider, kind and mailbox, not `accountId` (**§14.1 change; §4.3, §6.7 clarification**)
- **Context**: §14.1 set `AAD = "akatsuki:secret:v1:" + accountId + ":" + kind`, but §6.7 seals the refresh token (or the IMAP password) before `worker.createAccount`, and that mutation creates the `mailAccounts` `_id`. On a first connect the `accountId` does not exist when the AAD is needed, so the contract could not be implemented.
- **Alternatives**: (1) build the AAD from values known before insert; (2) split `createAccount` so it returns the `accountId` first, then seal and `putSecret` under a lease with `expectedVersion: 0`. Option 2 adds a round trip, a window in which an account exists with no secret (a crash leaves a `connecting` row that can never sync), and a lease the callback handler does not otherwise need.
- **Decision**: option 1. `AAD = "akatsuki:secret:v1:" + userId + ":" + provider + ":" + kind + ":" + providerAccountId`, with `providerAccountId` normalised as `worker.createAccount` stores it (Gmail: trimmed and lowercased; Graph and IMAP as given) and placed last because no other field contains `:`. Implemented as `crates/core::convex::worker::{secret_aad, normalize_provider_account_id}` plus `AssignedAccount::secret_aad` and `CreateAccountArgs::secret_aad`; the Convex normalisation in `worker.createAccount` carries a comment pointing at its Rust twin. No function shapes change.
- **Guarantee**: a ciphertext copied to a row of another user, another mailbox, another provider or another kind fails to decrypt. Rows that share an AAD belong to the same user and the same mailbox (a reconnect while the old row is still `removing`), so a copy between them grants nothing the owner does not already hold. `userId`, `provider` and `providerAccountId` never change on a row (a reconnect under a new address creates a new row), so the AAD is stable for `putSecret` and `rotate-secrets`.
- `v1` is kept: no ciphertext was ever sealed under the old formula.

### D-128 Managed dictation is not zero-data-retention (**BRIEF "AI layer" clarification; §12.3, §12.7 change**)
- **Context**: managed dictation (`POST /v1/ai/transcribe`) sends audio straight to the OpenAI transcription API with the managed `OPENAI_API_KEY`. The ZDR and data-collection-deny controls exist only on the OpenRouter path, yet README "Privacy guarantees" item 4 and §12.7 rule 4 claimed zero data retention for all managed requests, and the BRIEF says hosted AI stores and logs nothing.
- **Decision**: the claims are narrowed, not the feature. apps/api itself still stores and logs nothing (BRIEF holds for Akatsuki's own service). The README, §12.3 and §12.7 now say that managed dictation goes to OpenAI under its standard API data policy (not used for training; may be retained for abuse monitoring) and that zero data retention with OpenAI is not confirmed. A local Whisper endpoint keeps audio on the device. Comments in `apps/api/src/main.ts` and `infra/api.env.example` mark the point.
- **Owner decision still open**: either confirm zero data retention for the OpenAI organisation's audio endpoints (then add a new entry here, a startup or config assertion in apps/api, and restore the stronger copy), or route managed transcription through a provider with ZDR. Until then no copy may describe managed dictation as zero-retention. Tracked in `docs/STATUS.md`.

### D-129 One anchored baseUrl rule in Convex, packages/ai and crates/ai (**§4.2, §12.2 conformance fix**)
- **Context**: `ai.saveSettings` (the enforcing point, §4.2) checked LAN and loopback hosts with unanchored prefix regexes (`/^10\./`, `/^127\./`, …) and accepted any `*.localhost` name and URL fragments. `http://10.attacker.example/v1` and `http://127.evil.net` were stored as allowed http endpoints, while `packages/ai` and `crates/ai` anchored full IPv4 literals and rejected fragments.
- **Decision**: `convex/ai.ts` `isLocalHost`/`checkBaseUrl` now use the same rule as the TypeScript and Rust validators: `localhost`, a full 127.0.0.0/8 literal or `::1`; a full RFC 1918 IPv4 literal (10/8, 172.16/12, 192.168/16); or an mDNS `*.local` name (kept: a LAN Ollama or Whisper box is commonly reached as `name.local`, and `.local` resolves only on the link). `*.localhost` subdomains are refused everywhere. A URL with a fragment, including an empty one (`…/#`), is refused by all three.
- **Shared cases**: `packages/ai/fixtures/base-url/cases.json` (accept/refuse plus the local-host verdict, including `10.example.com`, `127.evil.net`, `172.32.0.1`, `evil.localhost` and fragment cases) runs in `packages/ai/test/fixtures.test.ts`, `crates/ai` `settings::tests::shared_base_url_cases` and `convex/test/pure.test.ts`. The Convex test also runs every valid settings fixture and every `invalid_base_url_*` / `invalid_http_*` fixture. New settings fixtures: `invalid_http_prefix_host.json`, `invalid_base_url_fragment.json`.
- The code stays as three ports, not one shared module: Convex does not depend on `@akatsuki/ai` (which pulls in the AI SDK), and the fixture keeps the three in step.


## Contract and hygiene fixes (verification round 2; 2026-10-20)

### D-350 `EMAIL_TRANSPORT=console` only on the local harness (**§14.6 addition**)
- **Context**: `convex/lib/email.ts` accepted `EMAIL_TRANSPORT=console` on any deployment and then logged the recipient address and the full text, including magic links and OTP codes. A cloud deployment with that variable set would put working sign-in credentials into retained function logs that every dashboard member can read.
- **Decision**: `consoleTransportMode(env)` decides. A loopback `CONVEX_SITE_URL` (`localhost`, a full `127.0.0.0/8` literal or `::1`; not `*.localhost`, the D-129 rule) is the local harness: the console sender logs address, link and code so offline sign-in keeps working (verified on the local backend). `AKATSUKI_DEV_HARNESS=1` on any other deployment allows the console sender in redacted mode: `to=<redacted>`, links and 4+ digit codes replaced, body withheld. Anything else throws when an email is sent. `ConsoleEmailSender` redacts by default when constructed directly. An unknown `EMAIL_TRANSPORT` value now throws instead of silently meaning SES; an empty value still means SES.
- **Why**: the harness needs the link in the log, and nothing else does. A remote deployment that wants to see "an email would have gone out" gets that without a credential in the log. Tests: `convex/test/pure.test.ts` ("email console transport").

### D-351 Push notifications default to `private`; `full` is documented as Expo egress (**§4.5, §14.2 clarification**)
- **Context**: `internal.notify.pushTargets` fell back to `"off"` when a profile row was missing, while `DEFAULT_PREFS.notifications` is `"private"`, so the two defaults disagreed. §14.2 did not say that `full` sends the sender and subject through a third party.
- **Decision**: the fallback is `DEFAULT_PREFS.notifications` (`private`). `private` sends a count only ("New mail", "N new messages"), no sender, subject or thread id. `full` is opt-in and sends the sender's display name (or address), the subject and the thread id through Expo's push service (`exp.host`) to APNs/FCM. §14.2 now has a table of what each mode sends. Tests: `convex/test/notify.test.ts`.
- **Why**: a new user should get a useful notification that leaks nothing. Anyone who turns on `full` should be able to see in the contract who receives the text.

### D-352 No turbo `lint` task; `test` caches no outputs
- **Context**: the root `lint` script ran `biome check . && turbo run lint`, but no package has a `lint` script, so the turbo step did nothing and suggested per-package linting that does not exist. The `test` task declared `coverage/**` outputs, but no package writes coverage.
- **Decision**: root `lint` is `biome check .` (one root `biome.json`); the `lint` task is removed from `turbo.json`; `test.outputs` is `[]`. `docs/DEV.md`, `CONTRIBUTING.md` and the `Makefile` help text say the same. A package that needs its own linter later adds a `lint` script and the turbo task back in the same change.

### D-353 `ai.getSettings` returns `AiSettingsView | null` (**§4.2 conformance**)
- The function has always returned `null` until the user's `aiSettings` row exists (created with the other user rows by the `user.onCreate` trigger or `me.bootstrap`). §4.2 now documents the nullable return; clients (web, desktop, mobile) treat `null` as the defaults (no profiles, no routes, no consent). No code change.

### D-354 §13.4 says which limits are enforced today
- **Context**: §13.4 listed every limit as if it were enforced. In code, `search.threads` has no rate limit (a query cannot write limiter state, D-057), the worker limits cannot exist while its routes answer `501` (D-091), the 120 min/day transcription cap is missing (D-123), and checkout's 10/h is per process.
- **Decision**: the table gains an "Enforced today" column with exactly that status, plus the per-request transcription caps apps/api does enforce (25 MB, 600 s) and the per-feature `maxInputChars` (≤ 120,000). The budget-exhausted note now matches apps/api: `429 ai_budget_exhausted` with the reset time in the message, and `resetsAt` from `GET /v1/ai/quota`. A "not yet" limit is still contract; its owner implements it, and nothing may rely on it until then.

### D-355 §18 environment table and `.env.example` list the same names
- **Context**: `.env.example` had names §18 lacked (`GITHUB_LOGIN_*`, `EMAIL_TRANSPORT`, the client `VITE_*`/`EXPO_PUBLIC_*`/`NEXT_PUBLIC_*` names, the Convex CLI names, `STRIPE_PUBLISHABLE_KEY`, `STRIPE_PRICE_ID`, `CLOUDFLARE_API_TOKEN`), and §18 lacked `WORKER_ISSUER` for apps/api, which `apps/api/src/config.ts` reads.
- **Decision**: §18 now has rows for the Convex deployment env, worker, apps/api, clients, Convex CLI, CI only, and **ops only** (`CLOUDFLARE_API_TOKEN` for DNS, `STRIPE_PUBLISHABLE_KEY` kept next to the secret key; neither is set on any component, since Checkout sessions are created server-side). `STRIPE_PRICE_ID` is removed from `.env.example`: nothing reads it, apps/api resolves the price from `STRIPE_PRICE_LOOKUP_KEY`. `AKATSUKI_DEV_HARNESS` (D-350) is added to both. The two lists were compared by script and are equal. Whoever adds a variable adds it to both in the same change.

### D-356 `docs/CREDENTIALS.md` holds names and status only; identifiers move to a gitignored local file
- **Context**: `docs/CREDENTIALS.md` is tracked and contained account identifiers: the Cloudflare zone id, the Stripe sandbox account and product ids, the Google Cloud project id and a DKIM selector token. They are not secrets, but publishing them helps targeting and social engineering. The BRIEF also put `OPENAI_API_KEY` in `.env.local`, while CREDENTIALS put it in `.env.credentials`.
- **Decision**: identifiers move to `docs/CREDENTIALS.local.md`, which `.gitignore` excludes (`docs/*.local.md`, checked with `git check-ignore`). `docs/CREDENTIALS.md` keeps services, secret names, where each secret lives and what is still needed. Keys created during setup, `OPENAI_API_KEY` included, live in `.env.credentials`; the BRIEF line now says so (the only BRIEF edit).

### D-357 `gpui 0.2.2` pulls `proc-macro-error2 2.0.1`, which triggers a future-incompatibility warning
- **Context**: `cargo build` reports a future-incompat lint (E0365, rust-lang/rust#127909: `pub use proc_macro;` re-exports a private extern crate) in `proc-macro-error2 v2.0.1`. The only path is `akatsuki-desktop → gpui 0.2.2 → stacksafe 0.1.4 → stacksafe-macro 0.1.4 → proc-macro-error2` (`cargo tree -i proc-macro-error2`). It is a warning on Rust 1.98.1 and does not fail `clippy -D warnings`, because lints in dependencies are capped. It will become a hard error in a future Rust release.
- **Decision**: accept the warning for now. No `[patch]`: `gpui` is pinned at the version §0 fixes, and patching a proc-macro crate deep in its tree is riskier than the warning. When the toolchain is bumped (D-358), run `cargo report future-incompatibilities`. If the lint has become an error, the options in order are: a `gpui` release whose tree no longer uses `proc-macro-error2 2.0.1`, a `proc-macro-error2` release that fixes it (a lockfile bump only), or a `[patch.crates-io]` to a fixed fork recorded here. The sync worker and every crate except `apps/desktop` do not depend on it.

### D-358 Toolchain pin policy
- **Rust**: exact pin in `rust-toolchain.toml` (`1.98.1` with `rustfmt` and `clippy`), the same version as the build image (`rust:1.98.1-trixie` in `apps/sync/Dockerfile`), so engineers, CI and the image use one compiler. No floating `stable`.
- **Node and pnpm**: `packageManager: pnpm@12.11.2` is exact; `engines` allows Node 26.x and pnpm 12.x, with `engine-strict`; CI and the `apps/api` image use Node `26.7` / `node:26.7.0-alpine3.24`.
- **Bumping**: one change moves `rust-toolchain.toml`, the Dockerfile `RUST_IMAGE` (or the Node image and CI `node-version`), `docs/DEV.md`/`docs/DEV-RUST.md` and the BRIEF toolchain line if the major/minor changes. It must pass `cargo fmt --check`, `cargo lint`, `cargo t`, `cargo deny check licenses bans sources` and `cargo report future-incompatibilities` (D-357), or the TypeScript gates for Node. Bumps are deliberate, never a side effect of another change. Pins that exist because of a dependency (for example `libc =0.2.189` for `gpui 0.2.2`, `docs/DEV-RUST.md`) are re-checked at the same time.

### D-359 Contract text brought in line with the code
- §1.7: the Rust crate list adds `akatsuki-tokens` and `akatsuki-shortcuts` (D-096), and the tokens/shortcuts row names the crates.
- §4.3: `worker.accountsForPush` documents the one-object args that D-057 introduced (`{ by, email?, subscriptionId? }`, `invalid_argument` when the matching field is missing), the Rust `AccountsForPushArgs` tagged enum, the 10-row cap, that `gmail_email` skips `removing` accounts and never sets `clientStateHash`.
- §12.4: the output-schema subset is the one in `packages/prompts/README.md` and `packages/prompts/src/fs.ts` (keywords `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `description`; adds `integer`; also bans `anyOf`, `allOf`, `minItems`/`maxItems`, `$schema`).
- `docs/contract/README.md` no longer says the decisions end at D-052.
- README "How it fits together": Convex never stores message bodies or attachments; it keeps headers and short snippets (≤ 200 characters). "It never holds message text" was wrong, since a snippet is message text. The repository map marks `apps/web` and `apps/mobile` as planned, because they did not exist when this change was made; the next engineer who creates one removes the mark.

## Infra and CI (2026-10-20)

### D-370 Hetzner layout: loopback ports 47310/47311, one compose project, hardened containers
- **Context**: the shared Hetzner host runs many unrelated services (BRIEF "Hosting"); Akatsuki must not collide with any name, port or volume, and the worker's host compromise yields everything (§14.1).
- **Decision**: `infra/hetzner/docker-compose.yml` (installed as `/opt/akatsuki/docker-compose.yml`, project `akatsuki`) declares only `akatsuki-sync` (`127.0.0.1:47310 -> 8787`) and `akatsuki-api` (`127.0.0.1:47311 -> 8787`), network `akatsuki-net` and volume `akatsuki-sync-spool`. Both containers: read-only root, `cap_drop: ALL`, `no-new-privileges`, non-root (`65532` distroless `nonroot`, `1000` for apps/api), `/tmp` tmpfs `noexec`, `restart: unless-stopped`, the images' healthchecks, json-file logs 5 × 10 MB, memory/CPU/pids limits so a runaway cannot starve the neighbours. Env files are `/opt/akatsuki/{sync,api}.env` (0600, root, never in git); `BIND_ADDR`, `API_INTERNAL_URL`, `PUBLIC_BASE_URL` and `SPOOL_DIR` are set in the compose file so a stale env file cannot move them. The spool volume is a named volume backed by `/opt/akatsuki/data/sync-spool` (created `65532:65532`, 0700, by the deploy script), because a fresh Docker volume is root-owned and the distroless image has no directory to seed ownership from.
- **Ports**: 47310/47311 were picked as unlikely to be in use; they are **not verified free** on the server (no ssh from this session). `scripts/deploy-hetzner.sh --preflight` checks them and aborts when anything else holds them.
- **Why**: unique names and loopback-only ports keep the blast radius to Akatsuki's own containers; the compose project label is what the preflight uses to tell ours from someone else's.

### D-371 `SPOOL_DIR` for the worker's encrypted upload spool (**§18 addition**)
- **Context**: §7.2 stores uploads in "the encrypted spool" for 24 h, but §18 lists no variable for its location, and the read-only root filesystem needs one writable mount.
- **Decision**: the worker reads `SPOOL_DIR` (default to be chosen by `apps/sync`; production and self-host set `/var/lib/akatsuki-sync/spool`, the only writable path in the container). `apps/sync` does not read it yet (the spool is not built); the variable is harmless until then. The `apps/sync` owner adds it to §18 and `.env.example` when the spool lands.

### D-372 nginx vhosts: coarse per-IP limits, per-route body limits, nothing on nginx's disk, no query strings in any log
- **Decision** (`infra/hetzner/nginx/{sync,api}.akatsuki.vc.conf`, included in the host's `http{}`):
  - All zones, maps, upstreams and log formats are prefixed `akatsuki_sync_`/`akatsuki_api_`, and no shared-name directives (`ssl_session_cache`, stapling, `default_server`, `reuseport`) are set, so the files cannot clash with other sites. `listen 443 ssl http2` (nginx 1.24 on Ubuntu 24.04; `http2 on;` after an upgrade), IPv4 only to match the A records.
  - Rate limits are per-IP abuse guards above the per-user limits of §13.4 (which stay in the services): sync 50 r/s burst 200–300, `/v1/connect/` and `/oauth/` 30 r/min, webhooks 200 r/s; api 20 r/s, `/v1/ai/*` 2 r/s burst 30 with 20 concurrent streams per IP, billing 30 r/min, Stripe hook 50 r/s.
  - Body limits follow the services: sync 2 MiB JSON, `/v1/uploads` 26m streamed (`proxy_request_buffering off`), GET-only routes 1k; api 64 KiB default, `/v1/ai/*` 260k, `/v1/ai/transcribe` 26m streamed, `/hooks/stripe` 1 MiB. `client_body_buffer_size` equals each JSON limit and `proxy_max_temp_file_size 0` is set, so request bodies (drafts, IMAP passwords, AI vars) and responses (message bodies, attachments) never land in nginx's temp files on disk.
  - `proxy_buffering off` for the AASP stream (`/v1/ai/*`, 300 s read timeout), the NDJSON body streams and attachment downloads.
  - `/metrics` (sync) and `/internal/` (api, D-048) return 404.
  - Access logs use a JSON format with `$uri` (never the query string), without client IPs or referrers, and with capability segments replaced (`/v1/img/:redacted`, `/v1/l/:token`, `/r/:token`). The error log is at `crit`, because nginx error lines contain the full request line with its query string; 4xx/5xx remain visible in the access log.
- **Why**: §7.1, §8 and §14.2 forbid query strings, tokens, addresses and bodies in logs; an unredacted image-proxy path contains the original image URL. Keeping bodies off disk preserves "bodies in transit and in memory only" (§14.2) through the proxy.
- **Verified**: `infra/hetzner/nginx/test-config.sh` passes `nginx -t` with nginx 1.24.0 built from source in a temp prefix, next to a stand-in site with `default_server` on the same sockets; CI repeats it with Ubuntu's nginx.

### D-373 TLS: certbot webroot under `/opt/akatsuki/acme` with a one-time bootstrap vhost; `certonly --nginx` as the alternative
- **Context**: the server's existing certbot method is unknown, a vhost that references a missing certificate fails `nginx -t`, and `certbot --nginx` (installer mode) would rewrite our vhost files.
- **Decision**: default `certbot certonly --webroot -w /opt/akatsuki/acme -d <name> --deploy-hook 'systemctl reload nginx'`. On a first deploy the script installs `infra/hetzner/nginx/acme-bootstrap/<name>.conf` (port 80 only) under the final vhost name, reloads, issues, then installs the full vhost; the full vhost keeps the ACME location for renewals. `AKATSUKI_CERT_METHOD=nginx` uses `certbot certonly --nginx` (authenticator only, never the installer). The webroot lives under `/opt/akatsuki` so the deploy touches no other directory.

### D-374 Cloudflare: `sync` and `api` stay DNS-only (**BRIEF "Hosting" confirmed, with reasons**)
- **Decision**: `sync.akatsuki.vc` and `api.akatsuki.vc` are A records to `65.21.141.237` with the proxy off; apex, `www` and `app` point at Vercel, also unproxied.
- **Why**: a proxied record terminates TLS at Cloudflare, which would decrypt message bodies, attachments and AI prompts (BRIEF privacy model); Cloudflare's 100 s origin timeout would cut AASP streams (up to 300 s) and long NDJSON/attachment streams; Gmail Pub/Sub and Graph validation callbacks, the http-01 challenge and per-IP limits work best against the origin. Cost: no Cloudflare DDoS shield for these two names, and the server IP is public (it already is, for the mail server).

### D-375 Deploy by building on the server from a per-tag source tree; dry run by default; GHCR publishing off
- **Decision**: `scripts/deploy-hetzner.sh` prints the plan unless `--apply`; `--preflight` runs read-only checks (ports, container/network names via the compose project label, vhost marker `# managed-by: akatsuki`, `server_name` clashes via `nginx -T`, env-file modes, certificates, disk). With `--apply` it rsyncs the tracked and unignored files (filtered again for `.env*`, `*.pem`, `*.key`; the list is checked before upload) to `/opt/akatsuki/src/<tag>/`, keeps three trees, builds `akatsuki-{sync,api}:<tag>` with each Dockerfile's allow-list `.dockerignore`, records the images in `/opt/akatsuki/release.env` (previous in `release.prev.env`, `--rollback` swaps back), runs `docker compose -p akatsuki up -d --wait`, checks loopback and public `/healthz`, and installs each vhost with a backup, `nginx -t`, then `systemctl reload nginx` (restoring the backup when the test fails). It never restarts nginx or any other service, never prunes, and never reads env files (`--upload-env DIR` copies them byte for byte with mode 0600).
- **Why**: the dev machine has no Docker daemon access, and GHCR publishing needs an owner decision about package visibility. `.github/workflows/release-images.yml` publishes `ghcr.io/<owner>/akatsuki-{sync,api}` for `v*` tags only when the repository variable `AKATSUKI_PUBLISH_IMAGES` is `true`.
- **Not run**: the script was only executed in plan mode and shellchecked; `--preflight` and `--apply` have never contacted the server.

### D-376 `gen-service-keys.mjs` lives at `infra/scripts/` and also creates the worker's symmetric keys
- **Context**: the infra task text said `scripts/gen-service-keys.mjs`; §6.6, §18, `docs/DEV.md` and `infra/api.env.example` all say `infra/scripts/gen-service-keys.mjs`. ARCHITECTURE wins.
- **Decision**: `infra/scripts/gen-service-keys.mjs` writes `service-keys.json`, `sync.env.part`, `api.env.part` and `convex.env.part` (public halves only) into `infra/secrets/<timestamp>/` (gitignored by `infra/.gitignore`, directory 0700, files 0600, refuses to overwrite) and prints only kids, paths and RFC 7638 thumbprints. Kids are `worker-YYYYMMDD-<hex>` / `api-YYYYMMDD-<hex>`; PEMs are one line with literal `\n` (both services un-escape them); values are single-quoted for Compose `env_file`, dotenv and `convex env set --from-file`. `--with-worker-secrets` adds `TOKEN_ENC_KEYS=k1:<32B>` and `STATE_KEY`, `SPOOL_KEY`, `LINK_SIGNING_KEY`, `IMG_PROXY_KEY`, `READ_RECEIPT_KEY` (32-byte base64, the shape `apps/sync` parses). `--previous DIR` keeps the old public keys in the JWKS for rotation. Tests: `node --test infra/scripts/gen-service-keys.test.mjs` (ES256 sign/verify against the JWKS, file modes, nothing private on stdout); also checked against `jose` (`importPKCS8` + `jwtVerify`).

### D-377 CI: SHA-pinned actions, every gate as its own job, optional workspaces detected
- **Decision**: `.github/workflows/ci.yml` runs `detect`, `typescript` (`pnpm lint`, `typecheck`, `test`, `build` of every workspace, `expo export` for iOS/Android when `apps/mobile` exists), `generated` (tokens, shortcuts, prompt pack and AI schema generators; `git status` must stay clean), `fixtures` (`make fixtures`), `convex` (local backend via `scripts/convex-dev.mjs up`, `pnpm convex:dev:deploy`, `convex/_generated` must be committed), `rust` (fmt, `clippy --workspace --exclude akatsuki-desktop -D warnings`, tests, `--locked`), `desktop` (Ubuntu with the GPUI packages of `docs/DEV-RUST.md`: clippy, tests, release build, plus a non-blocking 3 s Xvfb/lavapipe smoke run), `deny` (`check licenses bans sources` blocking, `advisories` reported), `images` (both Dockerfiles built with buildx, no push; image contents checked for env files, keys and package managers; then started with the production compose file, throwaway keys and a placeholder Convex URL, and both loopback `/healthz` checked), `e2e` (every app with a `test:e2e` script: Playwright Chromium, build, run, report uploaded on failure) and `infra` (no tracked env files, actionlint with a checksum-verified binary, shellcheck, `nginx -t` with Ubuntu's nginx 1.24, key generator tests, self-host compose validation, deploy-script dry run). Third-party actions are pinned to commit SHAs of releases at least about two weeks old; `.github/dependabot.yml` updates them weekly. No job uses a secret.
- **Conventions for other owners**: a Playwright smoke suite is exposed as the package script `test:e2e`; generated files are committed; the Convex job requires `convex/_generated` to match a fresh push.
- **Not run**: CI has never run (no remote). Locally verified: actionlint (with shellcheck) on both workflows, YAML parsing, `docker compose config` on both compose files.

### D-378 Vercel: monorepo builds through turbo; CSPs per app; `apps/web/vercel.json` created before the app
- **Decision**: both projects use their app directory as Root Directory, `pnpm install --frozen-lockfile`, and a turbo build filtered to the app (`cd ../.. && pnpm exec turbo run build --filter=...`). `apps/web/vercel.json`: framework `vite`, output `dist`, SPA rewrite to `/index.html` (except `/assets/` and `/fonts/`), immutable `/assets/*`, `no-cache` for `index.html`, CSP `default-src 'self'` with `img-src` limited to `data: blob: https://sync.akatsuki.vc`, `connect-src 'self' https: wss:` plus loopback (BYO AI goes from the browser straight to any provider or a local Ollama, D-009; sanitized bodies render in a srcdoc iframe that inherits this policy, §14.3), `frame-ancestors 'none'`, microphone allowed for dictation. `apps/landing/vercel.json` was created here and then refined by the landing owner (static export, no framework preset, HSTS).
- **Owner check**: Vercel must offer Node 26 for the `engines` pin, and `ENABLE_EXPERIMENTAL_COREPACK=1` is needed for `pnpm@12.11.2`; otherwise build in CI and deploy with `vercel deploy --prebuilt` (docs/DEPLOY.md §5).

### D-379 Self-host stack: self-hosted Convex, polling by default, optional AI and web profiles
- **Decision**: `infra/selfhost/docker-compose.yml` runs the self-hosted Convex backend (same pinned revision as D-058) and `akatsuki-sync` by default; `akatsuki-api` (profile `ai`, `AKATSUKI_BILLING=off`), a static web server for a local `apps/web` build (profile `web`, `nginxinc/nginx-unprivileged:1.30.6-alpine3.24`) and the Convex dashboard (profile `dashboard`) are optional. Everything binds to `127.0.0.1` behind the user's own TLS proxy; container names are `akatsuki-selfhost-*` and volumes/network `akatsuki-selfhost-*`, so the stack can run next to the dev harness. The worker reaches Convex on the Docker network while `CONVEX_SITE_URL` stays the public issuer. `SYNC_PUSH` defaults to `off` (no public webhooks needed). A one-shot `busybox:1.37.0-musl` init container makes the spool volume writable by `65532`. Real env files (`infra/selfhost/.env`, `sync.env`, `api.env`) are gitignored. The guide (`infra/selfhost/README.md`) states the non-commercial licence terms and never calls the product open source.

## Gmail adapter (2026-10-20)

### D-200 Gmail adapter layout; `fetch_threads` omits deleted threads (**§9.1 clarification**)
- **Context**: §9.1 gives `fetch_threads` no way to say "this thread no longer exists", and Gmail history has no thread-deleted event (only `messagesDeleted`).
- **Decision**: `crates/providers::gmail` (`GmailProvider`, `auth`, `client`, `batch`, `convert`, `quota`, `push`). `fetch_threads` returns only threads Gmail still has: a 404 inside the HTTP batch is skipped, never an error. The engine treats every requested id missing from the result as deleted and sends it in `upsertThreads.deletes`. `ChangePage.deleted_threads` is always empty for Gmail; a thread whose last message was deleted shows up as a touched thread that then comes back missing. Thread reads go through `POST /batch/gmail/v1` in groups of `batch_size` (50, Google allows 100); a single id is a plain GET.
- **Why**: keeps the trait unchanged and the deletion rule identical for history deltas, resync and op post-reads.

### D-201 Gmail quota costs from the 2026-10-16 table; two lanes; configurable (**§9.4 correction, spike S7 verified**)
- **Context**: S7 asked to verify the unit costs. Google's quota page (last updated 2026-10-16, read 2026-10-20) still has 6,000 units per user per minute and 1,200,000 per project per minute, but the costs moved: `threads.get` 40 (120 with `format=full`), `messages.get` 20 (60 with `full`/`raw`/`html`), `messages.attachments.get` 20, `threads.list` 10, `messages.list` 5, `history.list` 2, `threads.modify` 10, `messages.batchModify` 50, `drafts.update` 15, `drafts.send`/`messages.send`/`watch` 100, `stop` 50. §9.4's examples (`threads.get` 10, `messages.get` 5) are out of date.
- **Decision**: `gmail::quota::QuotaCosts::default()` carries the 2026-10-16 table and deserialises from camelCase JSON with per-field defaults, so a deployment can override costs without a release. `AccountQuota` has a `sync` lane (5,000/min: actor work, ops, sends) and an `interactive` lane (1,000/min: `fetch_raw`, `fetch_part`, drafts autosave, search), plus a shared `ProcessQuota` (1,000,000/min). Calls are charged before they are sent; a 429 or `rateLimitExceeded` for the account drains both lanes. Rate-limit scope is `Project` only when Google's message names the consumer/project limit, otherwise `Account`.
- **Consequence**: metadata thread reads cost 4x what §9.4 assumed, so one account syncs about 125 threads per minute at full budget (a 3,000-thread inbox takes about 24 minutes after the first 25-thread paint). §17 estimates should be revisited by whoever owns them.

### D-202 Google OAuth: web and installed clients, PKCE always, consent forced (**§6.7, §9.7 implementation**)
- **Decision**: `gmail::auth::OAuthClient::{web, installed}`. Both use PKCE S256 and `access_type=offline&prompt=consent&include_granted_scopes=true` (so reconnects get a refresh token again), with the §6.7 scopes by default. Web clients must have a secret (refused locally otherwise). Installed (desktop) clients send their secret only when configured, because Google does not treat it as confidential. The secret goes in the form body, never in a URL or `Debug`. `GoogleTokenSource` caches the access token, refreshes single-flight (one mutex held across the refresh) at `exp − 5 min`, and hands a changed refresh token to `RefreshTokenStore::rotated` (the worker's `putSecret` CAS) before using it. `invalid_grant` → `Auth { reauth_required: true }`; `invalid_client` → `Invalid` (an operator error, not the user's); 5xx → `Transient` with `Retry-After`. A Gmail 401 is retried once after `invalidate()`; a second 401 is `Auth { reauth_required: true }`. `revoke` treats 400 `invalid_token` as already revoked.

### D-203 Gmail `q` translation lives in `crates/search::gmail` (**§11.2, §18 implementation**)
- **Context**: §18 gives "provider query translation" to `crates/search`, which had none.
- **Decision**: `akatsuki_search::gmail::to_gmail_q(ast, label_name)` returns `{q, dropped}` or `TranslateError`. Fields map 1:1 (`from`, `to`, `cc`, `subject`, `list`, `filename`, `has:attachment`, `is:unread|read|starred|important`, `in:inbox|sent|drafts|trash|spam|anywhere`). `in:archive` becomes `(-in:inbox -in:trash -in:spam)` (§1.4). `in:`/`is:snoozed` and `label:sys:snoozed` become the `Akatsuki/Snoozed` label. Labels use Gmail's search form (lowercase, punctuation and spaces become `-`, as Gmail's own UI writes them). Dates go out as epoch seconds and ages as `<n>d`. Words Gmail would read as operators are quoted. A top-level `account:` is dropped and reported (the worker already chose the account); anywhere else it is unsupported. `category:` (Akatsuki triage), `is:needs_reply` and `in:<split>` are `Unsupported`, never silently dropped. `label:l:<id>` cannot be resolved by the adapter, which does not know Convex ids, so the caller (worker `/v1/search`) must rewrite it to the label name first; otherwise it is `UnknownLabel`, which `search()` maps to `provider_invalid`. `GmailProvider::translate_query` exposes the detailed error.
- **Graph/IMAP**: their translators should be sibling modules (`crates/search::{graph,imap}`).

### D-204 Gmail history paging uses only the history-id cursor (**§9.3, §9.5 clarification**)
- **Decision**: `changes_since` follows `nextPageToken` within one call, up to `history_pages_per_call` (10) pages of 500 records. If it finishes, `next` is the response `historyId` (the mailbox head). If it stops early, `next` is the id of the last record seen and `more = true`, so progress survives restarts without storing page tokens. Re-reading one record is harmless because touched threads are re-fetched idempotently. 404 → `CursorExpired`. `labels_changed` is true when a delta carries a non-system label id that was missing from the last `labels.list` (or any user label before the first listing), so the engine re-lists labels before mapping. Touched threads come from `messagesAdded`, `labelsAdded`, `labelsRemoved` and `messagesDeleted`, deduplicated in order.

### D-205 Gmail polling is a first-class mode (**§9.6 implementation**)
- **Context**: Pub/Sub needs a billed Google Cloud project, and the owner has not linked billing yet.
- **Decision**: without `GMAIL_PUBSUB_TOPIC`, `caps().push` is `Poll { every: 60 s }` and `watch()` returns `None` without calling Gmail. With a topic, `users.watch` uses the §9.6 label filter. If Gmail refuses the watch with 400/403/404/409 (topic missing, Gmail's publisher not granted, billing off), `watch()` logs a content-free warning and returns `None` too. The engine then polls `changes_since` every `poll_interval()`. Rate-limit and auth errors stay errors. `gmail::push::decode_push` decodes the Pub/Sub envelope for `/hooks/gmail` after the worker has verified the OIDC JWT; `GmailPush::is_newer_than(cursor)` implements the "drop hints at or below the cursor" rule.

### D-206 Gmail sends check Sent inside the adapter; ambiguity is detectable (**§5.6, §10.2 implementation**)
- **Decision**: `send_draft` and `send_raw` (`messages.send` with `raw`) both call `find_sent` first and return the existing `SentRef` when the message is already there, so the adapter is at-most-once even if a caller forgets the §5.6 check. `find_sent` queries `rfc822msgid:<id> in:sent` with `includeSpamTrash=true`. The `in:sent` keeps the draft, which carries the same `Message-ID`, from matching. `gmail::is_ambiguous_send_error` is true for every `Transient` except a connection that never opened: a timeout, reset or 5xx after the request left may have been accepted, so the engine sets `verifyOnly`. 4xx, quota (`dailyLimitExceeded` → `Quota`) and rate limits are certain failures. Messages over 4 MiB (`upload_threshold`) go through `/upload/gmail/v1/...?uploadType=multipart` (`multipart/related`: JSON metadata with `threadId`, then `message/rfc822`) instead of base64url JSON. `save_draft` creates a new draft when the existing one was deleted elsewhere (404 on update), so autosave never loses text. A send of a deleted draft stays `NotFound` (`draft_deleted`).

### D-207 Gmail op mapping (**§5.4, §9.5 implementation**)
- **Decision**: a delta that adds `sys:trash` calls `threads.trash`, one that removes it calls `threads.untrash` (and `Untrash` re-adds `INBOX`, matching §5.1). The rest of the delta then goes through `threads.modify`. Five or more consecutive ops with the same plain delta (`batch_modify_min_threads`) use one `messages.batchModify` (1,000 ids per call) over the message ids from the claim. This is cheaper (50 units against 10 per thread) but misses messages that arrived after Convex last synced; the push or poll echo corrects that. Every applied thread op re-reads the thread (`threads.get metadata`, batched for groups) and returns it as `post`, because `completeOps` needs the post-op state. `sys:snoozed` resolves to `Akatsuki/Snoozed`, created lazily once and cached. `label_create` treats 409 "name exists" as success and returns the existing label, so a replay after a crash cannot fail. A colour off Gmail's palette is dropped and the create retried. `label_delete` and `delete_draft` on a missing target are `moot`. `delete_forever` and `move` are `Unsupported` (D-039, `multiLabel`). `threads.delete` exists as `delete_thread_permanently` for deployments with the full scope, but `apply` never calls it.

### D-208 Gmail backfill, parts and dependencies (**§9.3, D-095, §9.1 implementation**)
- **Backfill**: `WindowRequest::Before` lists `messages.list q="before:<ceil s> after:<horizon s>"` with spam and trash included. It rounds `before` up so the boundary message is listed again rather than skipped, deduplicates thread ids, and reads `oldest` from one `messages.get format=minimal` of the last hit. When the range is exhausted (no next page), `oldest = horizon`, so the engine finishes the backfill.
- **Parts**: `fetch_part` maps the IMAP section path (D-095) onto Gmail's `payload` tree (`"a.b"` → `parts[a-1].parts[b-1]`, a single-part body is `"1"`). It returns inline `body.data` or `attachments.get` bytes, decoded. When the tree does not name the part (for example children of `message/rfc822`), it falls back to `format=raw` plus `akatsuki_mime::part_content`, the same parser that assigned the ids.
- **Headers**: metadata headers are re-serialised into a header block and parsed by `akatsuki_mime::parse_message`, so encoded words, address lists, `References`, `List-Unsubscribe` and `Authentication-Results` follow the body parser exactly. `crates/providers` therefore now depends on `akatsuki-mime` (direction `core ← mime ← providers`, no cycle). `date` is Gmail's `internalDate`, which falls back to `Date`. Snippets are HTML-entity-decoded. `hasAttachments` is `multipart/mixed` (metadata) or any named or attachment-id part (full); `attachment_sweep` provides the §9.5 `has:attachment` id sweep.
- **New dependencies** of `crates/providers` (all already workspace dependencies, no root `Cargo.toml` change): `akatsuki-mime`, `reqwest`, `serde_json`, `tokio` (sync, time), `base64`, `url`, `uuid`, `tracing`; dev: `wiremock`, `pretty_assertions`, `tokio` test-util.

## AI layer and apps/api completion (2026-10-20)

### D-340 Egress labels take the feature; `network` and `akatsuki_openai` labels (**§12.2 change, README privacy item 7**)
- **Context**: `egressFor(profile)` / `egress(profile)` ignored the feature, so managed dictation was labelled "Sent to Akatsuki for processing, not stored" although apps/api forwards the audio to OpenAI without zero data retention (D-128), and a Whisper or Ollama box on the LAN was labelled "Stays on this device" although the data leaves the device.
- **Decision**: `egressFor(profile, feature)` and `egressText(profile, feature)` in `packages/ai`, `egress(profile, feature)` and `egress_text(profile, feature)` in `crates/ai` (`feature` is a feature or `"default"` for the settings row of the default route). Labels: `device` (loopback only: `localhost`, 127/8, `::1`, and Ollama without a `baseUrl`), `network` ("Stays on your local network (<host>)" for RFC 1918 and `.local`), `provider` ("Leaves this device: <what the feature sends>, sent to <host>", with a per-feature phrase such as "your recording" or "the headers and snippets of new mail"), `akatsuki`, and `akatsuki_openai` for managed dictation: "Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring". `isLocalHost` keeps its meaning for the http rule (D-129) and is now `isLoopbackHost || isPrivateNetworkHost`.
- **Shared cases**: `packages/ai/fixtures/egress/cases.json` (25 cases covering every feature, label and provider kind; byte-identical text) runs in `packages/ai/test/fixtures.test.ts` and `crates/ai` `settings::tests::shared_egress_cases`. Resolve fixtures `whisper_on_lan` (now `network`) and `managed_dictation` (now `akatsuki_openai`) changed. ARCHITECTURE §12.2's table and §12.7 rule 6, and README item 7, describe the five labels.
- **Why**: the label is the user's only per-action statement of where data goes; it must be true per feature, and D-128 forbids "not stored" copy for managed dictation.

### D-341 `crates/ai` embeds the prompt pack and `routing.json`, with the same loader checks as TypeScript (**§12.3, §12.4 conformance fix**)
- **Context**: `crates/ai` read the prompt pack from the repository at run time (a packaged desktop app has no checkout), kept `routing.default.json` as a second copy of the model table, and its loader was laxer than `loadPromptPackFromDir`: no CRLF normalisation, unknown keys accepted, no temperature refusal, no `meta.id`, placeholder or output-schema checks, partial packs accepted. Its placeholder scanner refused `{{{x}}}`, which TypeScript renders as `{v}`.
- **Decision**:
  - `build.rs` embeds `packages/ai/routing.json` (the only table; the build fails without it, `routing.default.json` is deleted) and generates `(path, include_str!(…))` entries for `manifest.json`, `shared/untrusted.md` and the files directly in `features/<id>/`. `PromptPack::embedded()` runs them through the same loader as `PromptPack::load(dir)`. A test checks the embedded pack equals the directory and `pack.json` (system, user, variables, meta, schemas); `Routing` gained `escalation` and `pricing` and `deny_unknown_fields`.
  - Both loaders: CRLF → LF; `Manifest`, `FeatureVersion` and `Meta` refuse unknown keys (TypeScript gained the same checks); sampling keys (`temperature`, `top_p`, `topP`, `top_k`, `topK`) are refused with a D-042 message; `meta.id`/`version`/tier; manifest features equal the eight prompt features and the `features/` directories; placeholders equal the input-schema properties; `additionalProperties: false`; static system prompt; the strict output-schema subset (`check_output_schema`, a port of `checkOutputSchema`). `compose_system` trims with JavaScript's `trimEnd` set.
  - Placeholder rule (both): leftmost, non-overlapping `{{name}}` matches; the template with all placeholders removed must not contain `{{` or `}}`. `packages/prompts/fixtures/template/cases.json` (26 cases incl. `{{{x}}}`, `{{{{x}}}}`, `}{{a}}}`) runs in both.
  - The streaming tests now use the real embedded pack instead of a hand-written mini pack.
  - Root `.gitattributes`: `* text=auto eol=lf` (plus explicit binary types), because the pack and fixtures are compared byte for byte.
- **Why**: one source for model ids and prompts, the same refusals on every client, and desktop builds that work outside the repository.

### D-342 One whitespace set for the transcript and triage renderers (**§12.4 clarification**)
- **Context**: `renderTranscript`/`renderTriageItems` used JavaScript `trimEnd`, `trim` and `\s`, the Rust twins `trim_end` and `split_whitespace`. The sets differ: JavaScript includes U+FEFF and not U+0085, Rust the reverse, so a message ending in U+0085 or a subject with U+FEFF rendered differently (and the at-cap budgets could diverge).
- **Decision**: both use Unicode `White_Space` plus U+FEFF (`isRenderWhitespace` / `is_render_whitespace`, explicit lists, no built-in trim). `packages/prompts/shared/fixtures/whitespace.json` lists the set and both test suites check their predicate against it for every code point; `shared/fixtures/transcript/whitespace_set.json` and `triage-items/whitespace_set.json` pin U+FEFF and U+0085 behaviour. `shared/transcript.md` and the prompts README say so.

### D-343 Managed dictation cap: a daily bucket in `ee.service`, no schema change (**§13.4 implementation; supersedes the "not enforced" note of D-123/D-354 for this row**)
- **Context**: §13.4 caps managed transcription at 120 min/day per user, but `aiUsage` is monthly and apps/api is stateless.
- **Decision**: `convex/ee/service.ts` defines `transcriptionLimiter` (the existing `rateLimiter` component, its own name `transcriptionDay`): a fixed window of 7,200 tokens (audio seconds) per user and `period: DAY` with `start: 0`, so the day is the UTC day. `aiPrecheck` takes an optional `audioSeconds` (apps/api sends its estimate, ≤ 600) and, for dictation, **checks** without consuming; a refusal is `{ok:false, code:"rate_limited", retryAfterMs (until UTC midnight), limit:"transcription_day"}`. `aiRecordUsage` for dictation **takes** the real length, capped at what is left, so one recording that ran past its estimate is not carried into the next day. apps/api answers `429 rate_limited` with "Dictation is limited to 120 minutes a day. It resets at <ISO>." and logs `transcription_day_cap`. No table is added (the §3.2 schema block stays as is); `convex/test/transcription-cap.test.ts` covers the cap, the reset, per-user buckets and text features. Pushed to the local backend.
- **Why**: a rate-limiter bucket is the smallest additive change that is transactional with the precheck; check-then-take keeps refused or failed transcriptions free.

### D-344 Rust transcription client (**§12.1 `dictation`, §8 implementation in `crates/ai`**)
- **Decision**: `crates/ai::transcribe` with `TranscriptionTarget::{Managed, OpenAiCompatible}` and `dictate(client, settings, audio, media_type, opts, key_for)`: consent, resolution of the `dictation` route, then apps/api `POST /v1/ai/transcribe` with the user's JWT (multipart `audio`, BCP-47 `language`, `prompt`; envelope codes such as `rate_limited` are kept), OpenAI `POST {base}/audio/transcriptions` with the device key, or a local OpenAI-compatible Whisper server without a key. Like `@ai-sdk/openai`: `response_format` `json` for the gpt-4o transcription models and `verbose_json` otherwise, the language sent to OpenAI as its primary subtag, Whisper language names mapped to ISO 639-1. Inputs are checked before any request (empty, > 25 MB, media type, language tag; the hint is cut to 500 characters). Kinds without a transcription API fail with `AiError::Unsupported` (`unsupported`). The multipart body is built in the crate (random per-process boundary), so reqwest needs no new feature. `run_feature` refuses `dictation`.
- **Tests**: `crates/ai/tests/transcribe.rs` (wiremock: managed, managed error envelope, OpenAI BYO, local Whisper, refusals without network, provider text never surfaced).

### D-345 Escalation in `crates/ai` (**D-120 parity**)
- **Decision**: `Escalation::{Never (default), OnInvalidOutput}` and `run_feature_with(…, escalation)`. With `OnInvalidOutput` the one repair retry after a schema-invalid answer runs on the first heavy candidate from `routing.json` `escalation.heavy.<kind>` (Opus 5.5 for Anthropic, OpenRouter and AI Gateway; Fable 5.1 is never chosen) and a second `meta` event names it, as `streamFeature` does. Managed profiles, the transcription tier and kinds without candidates do not escalate. The retry keeps OpenRouter's privacy routing.

### D-346 Stripe: per-process webhook dedupe and a fetch seam (**§13.1, §13.2 refinement**)
- **Decision**: `StripeEventDedupe` in apps/api answers a redelivered event id that already finished in this process with 200 and no Stripe or Convex call, and lets a concurrent second delivery wait for the first run; failed runs are not remembered (Stripe's retry runs again), entries expire after 24 h and the map is bounded (10,000). `recordStripeEvent` stays the durable dedupe and the D-122 order is unchanged. `createStripeBilling` takes an optional `fetch` (passed to `Stripe.createFetchHttpClient`) so tests run the real client against a stub: checkout sends `trial_period_days: 14` with `trial_settings.end_behavior.missing_payment_method: "cancel"` only when the trial is unused, `payment_method_collection: "always"` and the price resolved from lookup key `akatsuki_pro_monthly` (cached 10 minutes); portal and cancel are covered too. Stripe keys come only from `STRIPE_*` in the environment.
- **Env names**: `API_ENV_NAMES` (the keys of the config schema) is pinned by `apps/api/test/config.test.ts` to the apps/api row of §18 (including `WORKER_ISSUER`, D-355), to `infra/api.env.example`, and to the root `.env.example`.

## Implementation decisions (Microsoft Graph adapter, `crates/providers/src/graph`; 2026-10-20)

Numbered D-210 to D-219.

### D-210 The Graph adapter owns Microsoft OAuth; rotated refresh tokens go through a `RefreshTokenStore` (**§6.7, §9.7 implementation**)
- **Context**: §9.1 defines `TokenSource` but not who talks to the Microsoft identity platform, and §9.7 requires the rotated refresh token to be persisted with `worker.putSecret` (CAS), which `crates/providers` may not call (§18: no Convex access).
- **Decision**: `graph::MicrosoftOAuth` (identity platform v2, `common` tenant so personal and work or school accounts both work, scopes of §6.7) provides `authorize_url` (PKCE S256, `prompt=select_account`), `exchange_code` and `refresh`; `GraphProvider::profile()` reads `/me` (`mail`, else `userPrincipalName`, as given per D-127). `GraphTokenSource` refreshes 5 min before expiry under one async lock (single-flight) and hands a changed refresh token to a `RefreshTokenStore` trait **before** using the new access token; `apps/sync` implements it with re-seal + `worker.putSecret`. A store error stops the refresh, so a lost CAS releases the account. `invalid_grant`, `interaction_required`, `consent_required`, `login_required` → `Auth{reauth_required:true}`; 429 → `RateLimited`; 5xx/`temporarily_unavailable` → `Transient`; `invalid_client` → `Invalid`. Token-bearing types redact `Debug`.

### D-211 Graph delta: `$select=conversationId`, a window filter, batched folders and verified removals (**§9.3, §9.5 deviation**)
- **Context**: §9.5 asks for per-folder `messages/delta` with header fields selected. Graph has no `$deltatoken=latest` for messages, so a head cursor needs a full delta round, and `ChangePage` only reports thread ids that the engine re-reads with `fetch_threads` anyway.
- **Decision**:
  - Delta rounds select only `conversationId` and start with `$filter=receivedDateTime ge (now − 90 d)`, `Prefer: odata.maxpagesize=50`. `baseline()` drains that round for every synced folder (ids only, cheap) and commits the delta links. Per Graph's documentation, read-state changes and removals are still reported for older messages; category and flag edits on messages older than the window made in another client are not, until a resync or search touches them.
  - `changes_since` sends one page per folder, 20 folders per `$batch`. `GraphFolderCursor.delta_link` holds the next URL to call (a `nextLink` mid-round, with `more: true`, or the `deltaLink`).
  - `@removed` entries are moves or deletes. A removed id that does not reappear in another folder's page is looked up with `GET /me/messages/{id}`: 404 → `deleted_messages`, found → its conversation is touched. `deleted_threads` is never set.
  - The folder list is reused for 10 minutes. A new folder starts its own windowed round, a vanished one (absent, or 404 on delta) is dropped, and either sets `labels_changed`. 410 or a `SyncState*` error → `CursorExpired`.
  - `list_window` uses plain listings ordered by `receivedDateTime desc` (Inbox: `mailFolders/inbox/messages`; `Since`/`Before`: `/me/messages` with a date `$filter`), not "delta on the Inbox folder" (§9.3), because `WindowPage` carries no cursor. `Before` pages by date (`next_page: None`, `oldest` set).

### D-212 Graph label ids: folders by id with `/` paths, categories as `cat:<name>` (**§9.5 clarification**)
- Messages reference categories by display name and Graph cannot rename a master category, so a category's `providerLabelId` is `cat:` + its display name. `label_create` on Graph creates a master category (Convex already creates Graph labels as `kind:"category"`), with the label colour mapped to the nearest of Outlook's 25 presets; a 409 adopts the existing category. `label_rename` of a category is `Unsupported`; folders are renamed with `PATCH /me/mailFolders/{id}`.
- Folder names are full paths (`Inbox/Projects`). `inbox, sentitems, drafts, deleteditems, junkemail` are `kind:"system"` with their role, `archive` is a `folder` with role `archive`, and `Akatsuki/Snoozed` is the child `Snoozed` of a top-level `Akatsuki` folder (created lazily by `ensure_label`). Hidden folders and the well-known `outbox`, `conversationhistory`, `syncissues` (and its children), `scheduled` and `clutter` are not synced.

### D-213 Graph drafts: MIME up to 2.8 MB, JSON with `createReply` and upload sessions above; replacing a draft changes its id (**§10.1 deviation**)
- **Context**: §10.1 says Graph drafts are `POST /me/messages` then `PATCH`. A JSON draft cannot carry our `In-Reply-To`/`References` or exact HTML and inline parts, and Graph rejects requests over 4 MB.
- **Decision**: drafts up to `mime_draft_max_bytes` (2.8 MB) are created from the base64 MIME of `crates/mime::compose` (`Content-Type: text/plain`), which keeps `Message-ID`, threading headers and parts. Larger drafts are recreated as JSON: replies start from `createReply` on the parent found by `internetMessageId` (so Outlook files them in the conversation), fields are `PATCH`ed (including `internetMessageId`), attachments under 3 MB are posted, larger ones go through `createUploadSession` in 3.2 MB chunks (multiples of 320 KiB) without the bearer token, to an upload URL whose host must be Microsoft's. Graph cannot replace MIME, so a save over an existing draft creates the new draft and then deletes the old one, best effort; the new id goes to `worker.recordDraft`.
- `send_draft` reads the draft (404 → `NotFound`, an already-sent draft is not sent again), then `POST /messages/{id}/send`; with immutable ids the copy in Sent Items keeps the draft id, which is returned. `find_sent` filters `internetMessageId eq '<id>'` over the whole mailbox and ignores drafts.
- **Unverified live**: that Exchange honours `internetMessageId` on large JSON drafts. If it does not, `find_sent` cannot match those sends (needs a test mailbox).

### D-214 Graph op semantics on conversations (**§5.1 helper table clarification**)
- `modify` sends one `PATCH` per message with the net `isRead`/`flag`/`categories`. Mark read and unstar touch every message that has the state; mark unread and star touch only the newest non-draft message, and do nothing when the thread already has the state (matching "at least one message" in §1.4). `sys:important` is `Unsupported` (Graph has no equivalent).
- A `modify` that adds a folder key is executed as a move to it; one that only removes a folder key is `Invalid` (folder providers use `move`, §5.1).
- `move` skips messages already in the target and leaves Sent Items and drafts in place unless the target is Deleted Items or Junk; `trash` moves every message; `untrash` sends drafts to Drafts, the account's own messages to Sent Items and everything else to the Inbox; `delete_forever` is `DELETE` per message (Exchange keeps it in Recoverable Items until retention ends).
- All message requests of one `apply` call share `$batch` requests of 20. A 404 for a single message is skipped; an op whose requests are all 404, or whose thread no longer exists, is `NotFound` (moot). Successful thread ops return the thread re-read after the change as `post`.

### D-215 KQL translation lives in `crates/providers::graph::kql` (**§11.2 implementation; §18 placement deviation**)
- **Context**: §18 puts provider query translation in `crates/search`; the Graph translation needs only the AST.
- **Decision**: `to_kql(ast, now)` lives next to the adapter that is its only user. Text/phrases, `from:`, `to:`, `cc:`, `subject:`, `label:` → `category:`, `filename:` → `attachment:`, `has:attachment` → `hasattachments:true`, `is:important` → `importance:high`, `before:`/`after:`/`older_than:`/`newer_than:` → `received<`/`received>=` on the user's calendar date (the start-of-day instant rounded to the nearest UTC midnight). A top-level `in:` scopes the request to that well-known folder; top-level `account:` and `in:anywhere` are no-ops. `is:` states, `in:snoozed`/splits, `list:`, triage `category:` and `in:`/`account:` inside groups return `Unsupported`, because Graph cannot combine `$search` with `$filter`. Embedded double quotes in values become spaces, so a value cannot break out of its phrase. `label:` on a folder label is still sent as `category:` (known gap).

### D-216 Graph throttling, error classification and link checks (**§5.5, §9.4 implementation; security**)
- Per mailbox: a semaphore of 4 concurrent requests and a token bucket of 16 requests per second, where a `$batch` costs one token per sub-request. A 429 or 503 whose `Retry-After` is at most 5 s is retried inline up to twice; longer waits surface as `RateLimited{scope: Account}` so the actor pauses. A 503 without `Retry-After` is `Transient`. A 401 drops the cached token and retries once; a second 401 is `Auth{reauth_required:true}`.
- Error values carry only the HTTP status and Graph's `error.code` (an identifier), never Graph's message text or the URL (§14.2).
- `@odata.nextLink`/`@odata.deltaLink` values are stored in Convex and returned in page tokens, so they are only followed when they start with the configured API base followed by `/`; anything else is `Invalid` before a bearer token is sent.

### D-217 Graph subscriptions and webhook helpers (**§9.6 implementation**)
- `watch` creates `me/messages` subscriptions (`created,updated,deleted`, `notificationUrl` and `lifecycleNotificationUrl` under `hook_base`, 4,200 min) with the immutable-id preference, keeps a subscription that has more than 24 h left, renews it with `PATCH` otherwise and recreates it after a 404. A renewal returns `client_state: None`, meaning "unchanged: keep the stored hash". With push off it returns `None` and caps say `Poll { every: 2 min }`.
- The new `clientState` is 32 bytes from two v4 UUIDs (244 random bits from the OS source via `uuid`), base64url, instead of 32 fully random bytes, to avoid a new dependency; only its hex sha256 (`client_state_hash`) is stored.
- For `apps/sync`: `parse_notifications`, `verify_client_state` (constant-time), `lifecycle_action` (`reauthorizationRequired` → `GraphProvider::reauthorize`, `subscriptionRemoved` → recreate and resync, `missed` → resync).

### D-218 `revoke()` on Graph is a no-op (**§9.1 clarification**)
- Microsoft has no endpoint that revokes one application's refresh token for a mailbox; `revokeSignInSessions` would sign the user out of every app. Account removal therefore drops the sealed token and the subscription (`unwatch`); removing the app's consent is up to the user in their Microsoft account. The removal copy should say so.

### D-219 Graph part ids and sizes (**D-095 conformance**)
- `fetch_part` accepts the IMAP-style section paths of `crates/mime` (D-095): it downloads `/messages/{id}/$value` and extracts the part with `part_content`, so part ids match what the sanitized body references. It also accepts `att:<attachmentId>` and streams `/attachments/{id}/$value` for callers that hold a Graph attachment id. `RemoteMessage.size_bytes` is `None` on Graph (v1.0 exposes no message size without extended properties).

## Landing site and privacy document (2026-10-20)

### D-300 akatsuki.vc is a Next.js static export served by Vercel as plain files
- **Context**: §18 gives `apps/landing` marketing, pricing, docs and legal; BRIEF puts it on Vercel. Nothing on the site needs a server, and a server runtime would add cost, cold starts and a place for logs.
- **Decision**: `output: "export"` (Next 16.4.2, React 19.3.1, Turbopack). The Vercel project uses Root Directory `apps/landing`, framework "Other", output `out/`, `cleanUrls`, and the response headers in `apps/landing/vercel.json`: a CSP with `connect-src 'self'` and no third-party origins (`script-src 'unsafe-inline'` is required by the inline RSC payload of a static export and the theme boot script), HSTS, `X-Frame-Options: DENY`, `nosniff`, Referrer- and Permissions-Policy, immutable `/_next/static/*`, and `Content-Type: image/png` for the extensionless `next/og` images. `scripts/serve.mjs` serves `out/` with the same headers (minus HSTS and `upgrade-insecure-requests` on plain http) and gzip, so the Playwright smoke test runs under the production CSP. Setup steps are in `apps/landing/README.md`; nothing was deployed.
- **Why**: fastest possible delivery, nothing to operate, and the "no trackers" promise is enforced by the CSP, not only by convention.

### D-301 Landing styles are plain CSS on `tokens.css`; dark-mode accent text is lifted for contrast
- **Context**: the reference site uses Tailwind and framer-motion. The dark accent `#800000` on `#0A0A0A` is 1.8:1, below WCAG for any text, and light foreground on it is needed for filled buttons.
- **Decision**: no Tailwind, no animation library: one `globals.css` built only from `@akatsuki/tokens` variables, CSS keyframes for orbs, the letter reveal and scroll-linked fades (`animation-timeline: view()` as progressive enhancement), all disabled under `prefers-reduced-motion`. Three derived variables, computed from token variables with `color-mix`, never literals: `--on-accent` (background on light crimson, foreground on deep crimson), `--accent-text` (dark: accent mixed 50/50 with foreground, about 5.5:1) and `--accent-display` (dark: 70/30, about 3.3:1, for display-size text only). A unit test fails on any hex colour in landing CSS. Keycaps and numerals use Geist with tabular figures instead of Geist Mono, and weights are limited to 400/500, so a page loads three font files.
- **Why**: smaller bundle and full Lighthouse accessibility without changing the token package. If the tokens owner wants these for web and mobile too, `onAccent` and `accentText` would belong in `packages/tokens`.

### D-302 Legal pages render the repository documents at build time
- **Decision**: `/privacy` renders `docs/PRIVACY.md` and `/security` renders `SECURITY.md` with `marked@18.0.14` (MIT; first `#` heading becomes the page title, headings get stable ids, repository links map to site pages, tables scroll on narrow screens); `/license` embeds `LICENSE` verbatim under a plain-English summary. `src/lib/repo.ts` reads only repository files and refuses `.env*` paths. Terms of service are a page of their own, marked "Draft pending lawyer review".
- **Why**: one source of truth; the privacy document cannot drift from the site. `apps/landing/test/content.test.ts` also pins the facts the privacy draft must keep stating (no bodies or attachments stored, 200-character snippets, OpenAI dictation retention, push content at "Full", SES, Stripe, no trackers, draft status).

### D-303 The waitlist has no backend: the form writes an email in the visitor's own mail app
- **Decision**: the download page's form builds a `mailto:` to the owner's published contact address with the visitor's email, platforms and note, and opens it; without JavaScript it falls back to a `mailto:` form action. No request leaves the page (the smoke test asserts no POST and no third-party request). Desktop, web and mobile are shown as "Coming soon".
- **Why**: BRIEF forbids trackers and no backend for sign-ups exists yet. When one exists, the form's `onSubmit` is the only place to change.

### D-304 The landing command palette reuses the app key map; no landing-only shortcuts
- **Decision**: `palette.open` (Ctrl K / ⌘K) opens a palette of pages, home sections and actions (theme, waitlist, email), navigated with `palette.next/prev/run/close`, all through `createMatcher` from `@akatsuki/shortcuts`. The home page's keyboard demo is a fictional inbox driven by the real bindings (`nav.*`, `thread.*`, `undo`); key labels follow the visitor's platform after hydration (`useSyncExternalStore`, "linux" labels in the prerender). It autoplays a short key sequence only while visible, idle and when motion is allowed. No new ids were added to `shortcuts.json`.
- **Why**: one key map (BRIEF "Quality bar"); the site demonstrates the product's own keys.

### D-305 What docs/PRIVACY.md says where the contract is silent
- **Decision**: (1) Managed dictation: "OpenAI does not train on API data, but may retain inputs for up to 30 days for abuse monitoring; Akatsuki has not confirmed zero data retention" (D-128). (2) Push: Off / Private ("New mail", counts only) / Full (sender name and subject via Expo and Apple/Google push), matching `convex/notify.ts`. (3) Read receipts record time and whether the open was proxied, no IP address (the `readReceipts.opens` shape). (4) Retention follows §4.5 and §13.3: account deletion purges everything; 30 days after a subscription lapses mail metadata and credentials are deleted; ops after 24 h / 30 d. (5) A sub-processor table (Convex, Hetzner, Vercel, Stripe, AWS SES, OpenRouter and its ZDR providers, OpenAI, Expo/Apple/Google, Cloudflare DNS, the user's mail providers). (6) Minimum age 16. Data locations, legal bases, transfer mechanisms, backup retention and governing law are left as bracketed "Lawyer review" notes rather than guessed.
- **Why**: plain and honest, and nothing claimed that the code does not do.

### D-306 Landing tool versions
- **Decision**: `@playwright/test@1.63.0` (not 1.64.1) because its Chromium revision 1243 is already installed on the dev machine, so tests need no browser download; `next@16.4.2`, `react`/`react-dom@19.3.1`, `@types/react@19.3.0`, `marked@18.0.14`; TypeScript, Vite and Vitest as in the rest of the workspace. `apps/landing/turbo.json` adds `out/**` as a build output and a `test:e2e` task that depends on `build`. The unit suite is the package's `test` script (runs in `pnpm test`); the browser suite is `test:e2e` (not in `pnpm test`, because it needs a build and a browser).

### D-307 Lockfile side effect: better-auth's optional `next` peer now resolves
- **Context**: with `autoInstallPeers: true`, adding `next` to the workspace made pnpm resolve `better-auth@1.6.33`'s optional peer `next` to `next@16.4.2` for `convex` (and `@convex-dev/better-auth`, `@better-auth/expo`), so their lockfile keys gained a `next@16.4.2(...)` suffix. Versions are unchanged.
- **Decision**: accepted. The Convex test suite (362 tests) passes after the change. If a smaller Convex install is wanted later, `peerDependencyRules.ignoreMissing`/`allowedVersions` cannot help (the peer is satisfied, not missing); the alternative is `dedupePeerDependents` tuning in `pnpm-workspace.yaml`, owned by the tooling owner.

## Shared TypeScript packages: mail-model, wire, ui (2026-10-20)

### D-250 `@akatsuki/mail-model` is the TypeScript home of the mail logic; one undo-inverse rule for TS and Rust (**§1.7, §5.2, §16.1; ends D-056 and the TS half of D-126's interim**)
- **Context**: `convex/lib/model.ts`/`query.ts` held the interim copies (D-056). The undo inverse existed only in Convex (`convex/lib/ops.ts`: on folder providers `move`/`trash` moved back to "the first removed key that is not unread/starred", otherwise the effect was swapped) while `crates/core` only had `Effect::inverse()` (always a `modify`), so the desktop overlay and Convex would undo a folder move differently. `untrash` on a folder provider became `modify{add:[sys:trash]}` (not a location change), and `delete_forever` produced a `modify` that cannot restore a deleted thread.
- **Decision**: `packages/mail-model` owns `labelKeys`, `applyOps`, `inverseAction`, `actions`, `matchRule`/`firstMatchingSplit`, `threadKeysFromMessages`/`threadAggregates`, the query grammar (`src/query/`, a straight move of the D-126 port) and `optimisticApply` (subpath `./optimistic`, the only module that needs the `convex` client). `convex/lib/model.ts` and `query.ts` are now re-exports under the old names, `convex/package.json` depends on `@akatsuki/mail-model`, and `convex/tsconfig.json` sets `allowImportingTsExtensions` (the package uses `.ts` import extensions like every other package). Pushed to the local backend.
- **Inverse rule** (`inverseAction(action, effect, folderProvider)` and `apply_ops::inverse_action`): empty effect, `unsubscribe` and `delete_forever` → none (`mode: "noop"`; a queued delete is still undone by cancelling it); on folder providers `move`, `trash` and `untrash` → `move{to}` back to the first removed key in sorted order that is not `sys:unread`/`sys:starred`/`sys:important`, else `sys:inbox`; otherwise `modify{add: effect.removed, remove: effect.added}`. 22 vectors in `apply-ops.json` `inverses` (effect, inverse and restored state) run in `packages/mail-model/test/fixtures.test.ts` and in a test inside `crates/core/src/model/apply_ops.rs` (the one Rust file this change touches).
- **Optimistic signature**: §16.1's `optimisticApply(localStore, threadIds, action)` gains a fourth argument `{ contextFor(accountId) }`, because a folder account's `ApplyContext` needs its exclusive labels and rows do not carry them. It also counts `mail.unreadCounts` up (mark unread), not only down, and leaves counts at the server cap (100) alone.
- **Action helpers**: `actionsFor(account, labels)` follows the §5.1 table. Where the table is silent: `removeLabel(folder)` on Graph/IMAP moves to the inbox; `archive()` on a folder account without a role-`archive` folder throws `ActionUnavailableError("invalid_action")` and `available("archive")` is false (nothing creates that folder lazily yet; follow-up for the worker/Convex owners).

### D-251 `@akatsuki/wire`: main entry without Convex sources, `./convex` for validators; offsets are UTF-8 bytes (**§1.7, §6.2, §7.3, §12.6**)
- **Entries**: `@akatsuki/wire` (types of §7.3/§8, `ERROR_CODES`, AASP, offsets, `getAccessToken`, typed `createWorkerClient`/`createApiClient`) uses only `convex/values` types (`Id<T>` = `GenericId<T>`, identical to the generated `Id`), so NodeNext consumers (packages/ai, apps/api) and Node's type stripping load it. `@akatsuki/wire/convex` re-exports `api`, `Doc`, `Id`, every validator of `convex/lib/validators.ts` and the row-view types for Bundler consumers (web, mobile); its typecheck runs under a second tsconfig with Convex's rules.
- **AASP**: the parser and encoder moved from `packages/ai/src/{sse,aasp}.ts` to `packages/wire/src/aasp.ts`; packages/ai re-exports them (its API is unchanged; `AaspError` is now the §15 `ErrorPayload`, which adds the optional `version`).
- **Offsets**: `MessageBody.quoted.htmlStart/textStart` are UTF-8 byte offsets (the Rust worker's string indices). JS clients must convert: `utf8OffsetToUtf16`, `utf16ToUtf8Offset`, `utf8ByteLength`, and `splitQuoted` (keeps everything visible on a bad offset). Fixtures: `packages/wire/fixtures/offsets.json` and the new worker value `message-body.non-ascii.json` (round-tripped by Rust too).
- **Tokens**: `getAccessToken()` wraps `authClient.convex.token({fetchOptions:{throw:false}})` behind `setAccessTokenSource()`; memory cache, refresh 60 s before `exp`, one request in flight, `WireError("unauthenticated")` when signed out.
- **Clients**: bearer auth, one retry with a fresh token after a 401, §15 bodies → `WireError` (status, `retryAfterMs` from the body or `Retry-After`, `version`), network failure → `unreachable` with `retryable: true`.

### D-252 Error registry additions (**§15 table updated**)
- `ERROR_CODES` (`packages/wire/src/errors.ts`) has every §15 code plus `not_implemented` (501, D-091) and the client-only `consent_required`, `ai_off`, `key_missing` (D-121), in table order, each with `http` (or `null`), `retryable`, `clientOnly` and a safe default message. `packages/wire/fixtures/error-codes.json` is the snapshot; the §15 table now lists the four codes.
- **Not done (Rust owner)**: `crates/core::wire::ErrorCode` lacks `consent_required`, `ai_off` and `key_missing` (desktop BYO raises them in `crates/ai` its own way) and does not yet run `error-codes.json`. `packages/ai` `transcribe.ts` throws a non-registry code `"managed"` for "use apps/api for Akatsuki AI dictation"; it should become a registry code or a typed return (packages/ai owner).

### D-253 `@akatsuki/ui`: Radix primitives, token-only CSS, copy for every §15 code (**§5.7, §15, BRIEF design**)
- **Stack**: React 19.3.1 (peer, exact), `@radix-ui/react-dialog` 1.3.1, `react-dropdown-menu` 2.2.1, `react-tooltip` 1.4.1, `react-visually-hidden` 1.2.13 (all MIT); tests with Vitest + Testing Library 16.3.3 / user-event 14.6.8 on jsdom 30.1.3 (MIT). No cmdk: the palette is a small combobox over a listbox with its own fuzzy scorer, so its keys come from `packages/shortcuts` (`palette.next/prev/run/close`) instead of a library's defaults. Toasts are our own (dedupe by id for opId-keyed failures, sticky errors, pause on hover/focus) rather than Radix Toast.
- **Styling**: one stylesheet `@akatsuki/ui/styles.css` with `ak-` classes; only token variables (a test rejects colour literals and unknown variables). Derived roles defined there: `--ak-on-accent` (text on the accent = the light end of the theme in both modes, since the dark accent #800000 needs light text), `--ak-surface`, `--ak-raised`, `--ak-shadow`, `--ak-focus`. If the tokens owner adds an `on-accent` token, `styles.css` should switch to it.
- **Copy**: `ERROR_COPY: Record<ErrorCode, {title, reason, remedy}>` is typed against the wire registry; `describeOpFailure` builds §5.7's "Couldn't archive 'Subject' in sami@…: the message no longer exists." with a masked account; `@akatsuki/ui/errors` has no React imports so mobile can share it.

### D-220 IMAP transport: `async-imap` for framing, auth and IDLE; an own command layer for the rest (**§9.5 implementation**)
- **Context**: `async-imap` 0.12.0 (pinned by §0) drops extension data in its typed helpers: no QRESYNC `SELECT`, `VANISHED`, `COPYUID`/`APPENDUID`, `X-GM-THRID` search, or literals in arguments.
- **Decision**: `crates/providers::imap::session` uses `async-imap` for the response framing (literals), `LOGIN`, `AUTHENTICATE` (`PLAIN` when advertised, else `LOGIN`; `XOAUTH2` with a `TokenSource`) and `IDLE`. Every other command is written by `ImapSession::exec` with its own tags (`k<n>`), `LITERAL+` when offered, and returns every untagged response plus the tagged code, parsed with `imap-proto` 0.17.0 (the version `async-imap` uses). TLS is `tokio-rustls` on aws-lc-rs with `webpki-roots` (plus `TlsOptions::extra_roots` for private CAs); `Security::Plaintext` exists only for loopback hosts (local bridges, the test servers) and is refused for anything else before connecting. `ENABLE QRESYNC` runs after login when offered.
- **Why**: one maintained, permissively licensed client crate as the contract wants, without losing the extension data the delta protocol depends on.

### D-221 SMTP submission is implemented in the adapter; ambiguous sends are flagged by message prefix (**§5.6, §10.2**)
- **Context**: `lettre`/`mail-send` report failures without saying whether the upload had begun, but §5.5/§5.6 need exactly that: only failures where the provider certainly did not accept the message may count as retryable send attempts, and a broken connection after `DATA` was accepted must become `verifyOnly`.
- **Decision**: `imap::smtp` is a small RFC 6409 client on the same rustls transport (implicit TLS or STARTTLS, `AUTH PLAIN`/`LOGIN`/`XOAUTH2`, `SIZE`, `8BITMIME`, `SMTPUTF8`, dot-stuffing). Errors carry a `Stage`; a transport failure in `Stage::Upload` maps to `ProviderError::Transient("send_ambiguous: …")`, checked with `imap::is_ambiguous_send`. Reply codes map per §5.5 (535 → `Auth`, 4xx → `Transient`, 4.7.x rate replies → `RateLimited`, 552 → `Quota`, other 5xx → `Invalid`). The envelope sender is the account address, the EHLO name the account's domain (never the worker host name), `Bcc:` is stripped from the transmitted data and kept in the Sent copy. After SMTP accepted the message nothing (Sent `APPEND`, draft removal) can fail the send.
- **Follow-up**: a `ProviderError` variant for ambiguous sends would be cleaner than the message prefix; it changes the §9.1 enum, so it is left to the contract owner.

### D-222 IMAP label and id details (**§9.5 clarification**)
- Keywords use the provider label id `keyword:<keyword>` (folders use the mailbox name), kind `keyword`, non-exclusive, only when INBOX `PERMANENTFLAGS` has `\*`; client-state keywords (`$Forwarded`, `$MDNSent`, `$Junk`, `NonJunk`, …) are ignored. `$Important` maps to `sys:important`.
- `\All` and `\Flagged` mailboxes are virtual views: neither labels nor synced, so no message is counted twice. Without SPECIAL-USE, roles come from well-known names (`Sent Items`, `INBOX.Trash`, `Junk E-mail`, `Archive`, …), never overriding an attribute. A folder whose `/`-path ends in `Akatsuki/Snoozed` is `sys:snoozed`.
- `X-GM-THRID` is written as lowercase hex, the Gmail API's `threadId` form. Mailbox names are modified UTF-7 on the wire and UTF-8 in `remoteFolderId`.
- Because an IMAP folder's provider label id is its name, `label_rename` returns the renamed label in `OpEffect::Applied.created` so the engine can store the new `providerLabelId`. New folders use the server delimiter and the `INBOX.` personal prefix when every other folder has it.

### D-223 IMAP thread lookups come from an in-memory index; IMAP threads are partial (**§9.1 refinement**)
- **Context**: `fetch_threads(ids)` cannot be answered by IMAP for computed thread ids (§9.5); only Gmail has `X-GM-THRID` search.
- **Decision**: `list_window`, `changes_since`, `search` and op post-states fetch headers and record them in a bounded `ThreadCache` (50,000 messages, worker memory only, §9.2 no durable state; entries of a folder are dropped when its UIDVALIDITY changes). `fetch_threads` answers from it, falling back to `UID SEARCH X-GM-THRID` per folder on Gmail. A `FetchedThread` holds the messages seen in this pass, not necessarily the whole thread, so the engine upserts IMAP delta threads with `complete:false`; op post-states contain every message of the op's thread ref at its new location and may be `complete:true`. `provider_version` is `None` (MODSEQ is per mailbox, not comparable across folders). Merging two threads when a late root arrives (§9.5) stays with the engine/Convex.

### D-224 IMAP delta details (**§9.5, §9.3 refinement**)
- `changes_since` first `STATUS`es each folder and skips folders whose `UIDNEXT` (and `HIGHESTMODSEQ` with CONDSTORE) did not move and whose scan is not due; the currently selected mailbox is re-selected instead (no STATUS on a selected mailbox).
- QRESYNC: `SELECT (QRESYNC (v m))`, `VANISHED (EARLIER)` → `deleted_messages`, changed UIDs → re-fetched headers. CONDSTORE only: `UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE m)`. New mail always via `UID SEARCH UID <uidNext>:*`, paged by `SyncTuning::max_new_per_page` (500) with `more: true`.
- Servers without QRESYNC run the expunge diff (and, without CONDSTORE, the flag diff) over `UID SEARCH SINCE <now − 90 d>` at most every 15 minutes per folder. The scan memory lives in worker memory, so the first scan after a start only records: expunges that happened while the worker was down surface through the engine's resync, not through the delta.
- A UIDVALIDITY change in any folder returns `CursorExpired` for the whole cursor. A folder that disappeared is dropped from the next cursor with `labels_changed: true`.

### D-225 Folder-provider ops on IMAP (**§5.1, §9.5**)
- `modify` that adds a folder target (`sys:inbox`, `sys:spam`, `sys:snoozed`, an exclusive `l:`) moves the messages there after applying flags; removing a folder without adding one is `Invalid` (folder providers use `move`, §5.1).
- `move`, `trash` and `untrash` use `UID MOVE`, else `UID COPY` + `\Deleted` + `UID EXPUNGE`, else (no UIDPLUS) `COPY` + `EXPUNGE` (which also removes other `\Deleted` messages in that folder, as every client does on such servers) with the copies found again by `Message-ID`. `COPYUID` rewrites message ids. Missing Trash, Junk, Sent, Drafts, Archive or `Akatsuki/Snoozed` folders are created lazily.
- Ops on UIDs that no longer exist return `NotFound` (→ `moot`). `label_create` on an existing folder succeeds (replays), `label_delete` of a missing folder is `moot`, keyword deletes are `moot`, keyword renames `Unsupported`.

### D-226 IMAP IDLE re-issue is 25 minutes; NOTIFY is not used yet (**§9.6**)
- The IDLE timer follows §9.6 and `crates/core::sync::IMAP_IDLE_REISSUE_MS` (25 minutes), not the 29 minutes RFC 2177 allows (the task text said 29; the contract wins). `ImapIdle` is the second connection of §9.4: `EXAMINE INBOX`, IDLE, and a `STATUS (MESSAGES UIDNEXT [HIGHESTMODSEQ])` sweep of the other folders every 5 minutes, reported as `FolderChanged`. Servers without IDLE are polled (`SyncTuning::poll_every`, 2 minutes). A broken connection returns an error once; the next call reconnects with backoff and yields `Reconnected`. NOTIFY (RFC 5465) is a follow-up.

### D-227 IMAP server discovery helpers, without a route yet (**§6.7 addition**)
- **Decision**: `imap::autoconfig` has a built-in table (iCloud, Fastmail, Yahoo, AOL, Zoho .com/.eu, Gmail, Outlook.com, GMX, mailbox.org, with an "app password required" flag) and a Mozilla autoconfig lookup (`autoconfig.<domain>`, `<domain>/.well-known/autoconfig`, then the Thunderbird ISPDB; HTTPS only by default, 5 s, 256 KB cap, plaintext and OAuth-only entries skipped), returning `ImapConnect`-shaped endpoints. Domains that are IP literals, `localhost`, `.local` or `.internal` are never fetched. The fetch uses the `reqwest::Client` the caller passes, which in the worker must be the §14.4 SSRF-guarded client.
- **Not done**: no worker route exposes it (a `GET /v1/connect/imap/discover?email=` would be a §7.2 addition), and there is no MX-based lookup (needs a DNS resolver).

### D-228 `crates/providers` depends on `crates/mime` (**D-090 refinement**)
- The IMAP adapter parses header sections, builds snippets and decodes parts with `akatsuki-mime` (`parse_message`, `snippet`, `part_content`), so the same parser serves bodies and list metadata. Direction: `core` ← `mime` ← `providers`; no cycle.

### D-229 IMAP drafts and sending (**§10.1, §10.2**)
- A draft is `APPEND Drafts (\Draft \Seen)`; its provider draft id is its message id (`"Drafts:<uidValidity>:<uid>"`, from `APPENDUID` or a `Message-ID` search). The new version is appended before the previous UID is expunged.
- `send_draft` reads the draft, checks that its `Message-ID` is the send's `rfcMessageId` (`Conflict` otherwise), submits it, then appends to Sent (`\Seen`) unless `sentAutoSaved`, then removes the draft. `find_sent` searches Sent then INBOX with `UID SEARCH HEADER Message-ID` and confirms the exact id. When the server saves sent mail itself and the copy is not visible yet, the returned `SentRef.providerMessageId` is `<rfcMessageId>`.
- `sentAutoSaved` defaults from the host (Gmail / `X-GM-EXT-1`, iCloud, Fastmail) unless `mailAccounts.imap.sentAutoSaved` is given.

## Implementation decisions (sync engine and worker: crates/core::sync, apps/sync; 2026-10-20)

Numbered from D-230 so they cannot collide with the numbers other workstreams are assigning in parallel.

### D-230 The actor driver lives in `apps/sync::engine`; `crates/core::sync` holds the pure decisions (**§9.2 refinement of D-090**)
- **Context**: §9.2 places the account actor in `crates/core::sync`, but `core` may not depend on `crates/providers` (D-090) and the actor needs `MailProvider`, `MetaSink` and `WorkerApi`.
- **Decision**: `akatsuki_core::sync` holds everything that can be decided without I/O and is unit-tested there: phases and priorities (existing), `budget` (token buckets, Gmail unit table, the 30% backfill share), `plan` (coalescing consecutive `modify` ops per thread into a net delta minimised against the per-message keys of the claim, moot detection), `send` (the §5.6 decision table) and `backfill` (window bounds, date cursor, resync diff). The loop that performs them is `apps/sync::engine::{actor, ops, supervisor, sink}`.

### D-231 Provider factory, fake provider, adapter wiring (**§9.1 implementation**)
- The engine only sees `Arc<dyn MailProvider>` and an optional `Box<dyn PushSource>`. `apps/sync::providers::AdapterFactory` builds `GmailProvider` (shared process quota), `GraphProvider` (push only with `PUBLIC_BASE_URL`) and `ImapProvider` + `ImapIdle` from the decrypted credentials; `POST /v1/connect/imap` probes through `akatsuki_providers::imap::test_connection`. Engine tests use `engine::fake::FakeProvider` and an in-memory `WorkerApi` that follows the ops rules of `convex/worker.ts`.
- If a factory fails, the account is reported `degraded` / `provider_unavailable` and retried after 10 minutes instead of crash-looping.

### D-232 Where a claimed account resumes, and its window bounds (**§9.2/§9.3 clarification**)
- **Context**: the worker receives `mailAccounts.status` (`assignedAccounts`) and `syncState` (`claimAccount`), but not `syncPhase`, `windowStartAt` or `backfillHorizonAt`.
- **Decision**: no committed cursor → Baseline; `live`/`degraded` with a cursor → Live (plus one delta at once); `connecting`/`initial_sync` with a cursor → InboxFirst again (upserts are idempotent and version-guarded). `windowStartAt` = `syncState._creationTime − 90 d` (the row is created at connect), the horizon is the default 365 days. A user-chosen horizon (`accounts.setBackfillHorizon`) is not visible to the worker yet: `worker.assignedAccounts` (or `claimAccount`) should return `syncPhase`, `windowStartAt` and `backfillHorizonAt` (Convex owner).

### D-233 Resync without a Convex listing (**§9.3 step 6 interim**)
- No worker function lists the Convex threads inside the window, so a resync re-lists the window and re-fetches every listed thread (`providerVersion` makes unchanged ones `skippedStale`), then commits the new baseline. Threads deleted at the provider during the outage are not removed by the diff; later deltas and `NotFound` op results remove them. `core::sync::backfill::resync_plan` already computes refetch/delete sets and takes the known map once Convex offers it.

### D-234 Ambiguous sends are left inflight; verification uses retryable `send_unconfirmed` results (**§5.6 implementation**)
- **Context**: `worker.completeOps` cannot set `verifyOnly`; only `ops.reapLeases` does (for sends with `sendStartedAt`).
- **Decision**: after `markSendStarted`, an ambiguous provider error is not reported at all; the op's lease expires and the reaper requeues it as `verifyOnly`. A `verifyOnly` op only runs `find_sent`: not found → `{ok:false, code:"send_unconfirmed", retryable:true, retryAfterMs: 30 s | 2 min | 10 min, countAttempt:true}`; after the third miss a non-retryable `send_unconfirmed`, which Convex turns into `unknown`. Every attempt runs `find_sent` first. Classification: the IMAP adapter's `send_ambiguous` prefix is ambiguous and its other transient errors certainly did not send; for Gmail and Graph every transient send failure is ambiguous. `markSendStarted` failing with `lease_lost` aborts before the provider call.

### D-235 The worker calls `drafts.get`, `drafts.discard` and `identities.list` with the caller's own JWT (**§10.1 implementation**)
- **Context**: `PUT /v1/drafts` needs the draft's fixed `rfcMessageId`, provider draft id and CAS version, the identity's signature, and `DELETE /v1/drafts` must enqueue `delete_draft`; no worker service function returns or does that.
- **Decision**: the worker forwards the request's user JWT to the Convex HTTP API (`POST {CONVEX_URL}/api/query|mutation`) for these three client functions (`apps/sync::user_convex`). Convex checks ownership as for any client; no new service functions. The JWT is never logged (`UserToken` redacts `Debug`).

### D-236 Additive worker query `worker.threadIdsByProvider` (**§4.3 addition**)
- `GET /v1/search` must answer with `Id<"threads">` after upserting provider hits, but `worker.upsertThreads` returns ids only for new inbound threads. `worker.threadIdsByProvider({accountId, providerThreadIds ≤ 100})` → `{providerThreadId, threadId}[]` (index `by_account_provider`) was added to `convex/worker.ts`, with Rust bindings (`WorkerApi::thread_ids_by_provider`) and `convex/test/worker-search.test.ts`.

### D-237 FIFO per thread also inside one claim batch (**§5.4 refinement**)
- `worker.claimOps` may return several ops of one thread in one batch. The actor sends provider ops in rounds of at most one op per thread (or label); when one fails, the later ops of that thread in the batch are not sent and are requeued behind it with `countAttempt:false`. A local (non-provider) failure blocks its lane the same way and counts toward poison (two strikes → `failed`/`internal`).

### D-238 `sys:snoozed` targets the concrete `Akatsuki/Snoozed` label (**§9.3 implementation**)
- Ops that add, remove or move to `sys:snoozed` pass `LabelTarget::Label{providerLabelId}` of the snoozed label (from the claim's role labels), so adapters need no special case. When it does not exist yet the actor calls `ensure_label(Snoozed, "Akatsuki/Snoozed")` and sends the new label in the same `completeOps`.

### D-239 Read-receipt tokens are HMAC-derived and verifiable (**§7.2, §14.1 implementation**)
- Token = base64url(10-byte `HMAC(READ_RECEIPT_KEY, "rr-id|userId|rfcMessageId")` ‖ 6-byte `HMAC(key, "rr-tag|"+id)`), 22 characters: stable across autosaves of a draft, and `/r/{token}.gif` rejects forged tokens without a Convex call. Opens through Google/Yahoo image proxies or without a User-Agent are `proxied`. The pixel is only composed when the draft asks for it and `prefs.readReceipts.enabled`.
- **Gap**: no Convex function creates `readReceipts` rows, so `worker.recordReadReceipt` finds nothing to update yet. Convex owner: insert the row when a draft with a receipt is scheduled (the worker can pass the token in `worker.scheduleSend` or `recordDraft`).

### D-240 Spool file format and `SPOOL_DIR` (**§10.4 implementation**)
- One file per upload: `AKS1` ‖ 24-byte nonce ‖ XChaCha20-Poly1305 ciphertext under `HKDF-SHA256(SPOOL_KEY, "akatsuki:spool:v1:<uploadId>")`, AAD the same string. The plaintext carries the owner, file name, type, size and digest, so the directory shows only random ids. Upload ids are 32 hex characters (no path traversal). Files go when attached to a provider draft or after 24 h (sweeper every 30 min, by mtime). `SPOOL_DIR` (optional, default `/var/lib/akatsuki/spool`) is the one environment variable added beyond §18.

### D-241 Image proxy and SSRF guard details (**§14.4 implementation**)
- `apps/sync::fetch::SafeFetcher` resolves DNS itself (reqwest resolver), drops loopback, private, link-local (incl. metadata), CGNAT, benchmarking, reserved, multicast, documentation, ULA, Teredo, NAT64/6to4-embedded private IPv4 and IPv4-mapped forms, refuses `localhost`, `*.localhost`, `*.internal` and `*.local` names and private IP literals, follows at most 3 redirects by hand (each re-checked), no proxy from the environment, no cookies or Referer, 10 s, 10 MB. The served `Content-Type` comes from magic bytes (PNG, GIF, JPEG, WebP, AVIF, BMP, ICO); SVG and everything else are refused (no rasterizer). 600 requests per minute per user hash.

### D-242 Parts are cut from the raw message (**§7.2 implementation, D-095**)
- `GET /v1/messages/{id}/parts/{partId}` and `/v1/l/{token}` fetch the RFC 822 source and extract the part with `crates/mime::part_content`, so part ids mean the same thing for every provider and `Range` applies to the decoded part (single range; 416 when unsatisfiable). `MailProvider::fetch_part` is not used yet.

### D-243 Unsubscribe reads the newest message's headers (**§9.1 implementation**)
- The claim carries no `listUnsubscribe`, so the engine fetches the newest message of the thread and parses `List-Unsubscribe`/`List-Unsubscribe-Post` with `crates/mime`. One-click needs `oneClick` and an https URL and goes through `SafeFetcher`. A mailto unsubscribe uses a Message-ID derived from the op id and checks `find_sent` first, so a retry never mails twice. At most 3 attempts.

### D-244 Budgets: adapters charge exact quota, the actor paces backfill (**§9.4 implementation**)
- The Gmail adapter charges per-call units against its account and process buckets. The actor does not charge sync steps a second time; it only charges backfill pages against the 30% share while the user is active (an op, body, part or search in the last 5 minutes) and pauses the whole actor (or every Gmail actor for `scope: Project`) on `RateLimited`.

### D-245 OAuth connect details (**§6.7 implementation**)
- Google: `access_type=offline&prompt=consent` so every connect returns a refresh token; Microsoft: `prompt=select_account`, scopes sent on the token request too. `state` is sealed with `STATE_KEY` under the AAD `akatsuki:oauth-state:v1` and bound to the provider path. A callback without a valid state answers `400` (there is no trusted `returnTo`); otherwise errors redirect with `status=error&code=` (`unauthenticated` for a denied consent or rejected code, `provider_unavailable`, `provider_invalid` when no refresh token came back, or Convex's `already_connected`/`not_entitled`). Gmail send-as aliases with a non-accepted verification status are skipped.
- `POST /v1/drafts/{id}/adopt` records the provider message id as the draft id when Convex has none (Graph and IMAP address drafts by message; the Gmail adapter must map it).

### D-246 Watch bookkeeping (**§9.6 implementation**)
- Push is active only with `SYNC_PUSH=on` and `PUBLIC_BASE_URL` (and `GMAIL_PUBSUB_TOPIC` for Gmail); otherwise `syncState.watchKind` is set to `poll` (or `imap_idle`) and the actor polls (Gmail 60 s, Graph 2 min, IMAP 5 min) besides the 5-minute safety poll. Gmail watches renew every 24 h, Graph subscriptions when less than 24 h remain; only a new Graph `clientState` writes a new hash. Losing entitlement stops the watch; regaining it restarts the actor.

### D-247 `isOutbound` uses the account's primary address (**§4.4 interim**)
- `worker.assignedAccounts` carries no identities, so `ConvexSink` marks a message outbound when its sender is the account address. Send-as aliases need the identities in `assignedAccounts` (Convex owner).

### D-248 `rotate-secrets` claims each account briefly (**§14.1 implementation**)
- `worker.putSecret` requires the lease, so the CLI claims each account (`rotate-<id>`, 60 s), re-seals under the active key with `expectedVersion`, and releases. Accounts held by a running worker are reported busy; that worker re-seals them itself whenever it claims an account whose secret uses an older key. Exit code 0 only when nothing is left busy, unreadable or failed. Output carries counts only.

### D-310 Desktop alpha: the mirror is `akatsuki-core`'s `store` feature; desktop client calls added to `ClientApi` (**§16.3, §4.2 implementation**)
- **Context**: §18 puts the desktop SQLite mirror in `crates/core::store`, but the sync worker also depends on `akatsuki-core` and must not compile SQLite. The desktop needs a few client functions `ClientApi` did not bind, and `ClientApi::ai_settings` failed to decode the `null` that `ai.getSettings` returns before the row exists.
- **Decision**: `crates/core/src/store` is behind the optional feature `store` (`rusqlite` bundled, `zstd`); only `apps/desktop` enables it. `ClientApi` gains `list_splits`, `mail_snooze`, `set_reminder`, `sends_cancel` and `save_ai_settings` (types in `core::convex::client`, names `splits:list`, `mail:snooze`, `mail:setReminder`, `sends:cancel`, `ai:saveSettings`), and `ai_settings` returns `Option<AiSettings>` (D-057, D-353). `keyring` is pinned `=4.2.0` (§0) with its default `v1` stores (Secret Service on Linux, Keychain, Credential Manager); `zstd 0.13.3`, `dirs 5.0.1` and `unicode-segmentation 1.13.3` are workspace dependencies.
- **Why**: keeps the worker image small and SQLite-free, keeps every Convex call typed in one crate (D-092), and makes a new user's settings readable.

### D-311 The desktop FTS5 index is a sibling file, `search.sqlite` (**§16.3 refinement**)
- **Context**: §16.3 lists `messages_fts` among the mirror's tables, but `akatsuki-core` depends on no workspace crate (D-090) and `crates/search::SearchIndex` owns its own connection and schema.
- **Decision**: the desktop keeps `mirror.sqlite` (core `Store`) and `search.sqlite` (search `SearchIndex`) side by side in the data directory (mode 0700). Headers and snippets of every mirrored message are indexed when the feed applies them; bodies are indexed when opened (§11.2). Sign-out deletes both.
- **Why**: no dependency cycle, and either file can be rebuilt without touching the other.

### D-312 Desktop overlay: base is the server's `labelKeys`; rows leave at `min_rev` or when the server shows them (**§16.3 refinement**)
- **Context**: §16.3 says visible = `apply_ops(synced, overlay)`. The server's `labelKeys` already include every op the server knows about (other devices' too), and `mail.undo` and `mail.snooze` return no revision to wait for.
- **Decision**: visible = `apply_ops(thread.labelKeys, local overlay in insertion order)`. A row gets `min_rev` from `mail.apply`'s `rev` and is dropped once the mirror reaches it. Mutations without a revision mark their rows `acked_at`; such a row is dropped once applying it to the server state changes nothing, or after 60 s. A permanent error drops the rows (rollback by construction); a transport error keeps them, with the durable outbox replaying later. Local undo inverts the recorded per-thread effect (`op_effects`, kept an hour), so the list changes at once whatever the server has done. The overlay and the outbox survive a `resetRequired` wipe. Thread rows are applied only when their `rev` is not older than the stored one; a batch computed from a revision the mirror does not hold is refused (a gap) and the feed re-subscribes from the local revision.
- **Why**: no flicker in either direction, and the same "recompute, never patch" rule as §5.1.

### D-313 Desktop lists: split tabs, "Reminders", unified and per-account views
- **Decision**: the inbox shows the user's splits (`splits.list`, by `order`) as tabs plus "Other" (`split:other`) last; with no splits there are no tabs and the list is `sys:inbox`. "Reminders" lists threads with `reminderAt`, soonest first, from the mirror. Lists are unified across accounts unless one account is chosen in the sidebar or palette. Opening a thread marks it read; acting on the open thread opens the one that takes its row (Superhuman's auto-advance). `#` inside Trash restores, `!` inside Spam is "not spam", and `l` on threads that all carry the label removes it.

### D-314 Desktop configuration and device settings
- **Decision**: endpoints come from `desktop.json` in the config directory, overridden by `AKATSUKI_CONVEX_URL`, `AKATSUKI_CONVEX_SITE_URL`, `AKATSUKI_SYNC_URL` and `AKATSUKI_API_URL` (§18 and `.env.example` list them); `AKATSUKI_DATA_DIR` moves the data directory. Theme, density, key map, motion, the undo-send window and "keep bodies on this device" are stored per device in `desktop.json`, not yet synced to `prefs` (no `me.updatePrefs` binding). The device id is a UUIDv4 file next to the mirror. Development hooks: `--demo`/`AKATSUKI_DEMO=1`, `AKATSUKI_EXIT_AFTER_MS`, `AKATSUKI_SCRIPT` (command ids run at start-up, `search=<q>` types a search) and `AKATSUKI_PROMPTS_DIR`.
- **Why**: a self-hoster can point the app at any deployment without a rebuild; nothing here is a secret, and secrets stay in the keychain.

### D-315 Desktop keys go through a Rust twin of the web matcher, not GPUI key bindings (**BRIEF "Keyboard-first"**)
- **Context**: "keyboard navigation identical to web" needs the same sequences (`g i`), timeouts (1.2 s), scope rules (global single keys off in text fields, command combos on) and Gmail overrides as `createMatcher` in `packages/shortcuts`.
- **Decision**: `apps/desktop/src/keys.rs` ports `parseCombo`, `comboFromEvent`, `scopeApplies` and the matcher over `crates/shortcuts`' "+" notation, and the workspace feeds it every key-down. Text fields keep GPUI actions in their own `TextField` context (editing keys, Enter, Escape). On Linux GPUI reports Ctrl+Shift+, as `ctrl-<`, so with a command modifier a US-layout table maps shifted symbols back to their key plus Shift, as the web reads the physical key. The palette lists every `palette: true` shortcut with the same key hint, and both go through one dispatcher.

### D-316 The demo mailbox
- **Decision**: `akatsuki-desktop --demo` (or the sign-in screen when no server is configured) opens a demo mailbox: two accounts (Gmail-like and Outlook-like) with invented `.example` mail, served by an in-process `DemoApi` that implements `ClientApi` and the worker routes, so the feed, overlay, undo, rendering, search and compose run their production code paths. It uses an in-memory mirror and an in-memory secret store, never the keychain or the network, and the UI labels it "Demo".
- **Why**: the app is usable and testable before any account, OAuth client or deployment exists, and smoke tests can drive it.

### D-317 "Open original" opens a sanitized file in the system browser, not a webview (**§14.3 implementation**)
- **Context**: §14.3 asks for a sandboxed webview window with JavaScript disabled. `wry` is MIT/Apache, but on Linux it needs WebKitGTK and GTK, which GPUI does not use.
- **Decision**: the desktop writes the locally sanitized HTML to `originals/<hash>.html` in its data directory (directory 0700, file 0600, name derived from a hash, never message data) with `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; font-src data:; form-action 'none'; base-uri 'none'">` and `referrer: no-referrer`, and opens it in the system browser. Inline `cid:` images become `data:` URLs. Remote images stay blocked unless the user chooses "Show images in the original", which allows `img-src https:` in that one file; those requests go straight to the sender's hosts (the desktop sanitizes from `?format=raw` locally and cannot sign proxy URLs), which the button says. The directory is cleared at start-up and sign-out.
- **Why**: no new native dependency, the same CSP as the web's sandboxed frame, and a sanitizer bug still cannot run script.

### D-318 Native renderer scope
- **Decision**: the reading pane draws the safe tree natively: paragraphs and headings with bold, italic, underline, strike-through and monospace runs, clickable links (the sanitizer keeps only http, https, mailto and tel), quotes, lists, preformatted text, rules, data tables, and inline (`cid:`) or small `data:` PNG/JPEG/GIF/WebP images; one-column layout tables are flattened into blocks. The email's colours and fonts are not used, so every message reads in light and dark. Quoted history after the sanitizer's `quote_start` is collapsed behind "•••". Messages with tables nested three deep, more than 120 cells or more than 16 images show "This message has a complex layout. Open the original". GPUI is never handed a URL to load.

### D-319 Desktop body cache
- **Decision**: bodies are fetched with `GET /v1/messages/{id}?format=raw`, parsed and sanitized locally with `crates/mime` (`RemoteImagePolicy::Original`, nothing loads until asked), and cached as zstd-compressed raw, sanitized HTML and text keyed by message id and `SANITIZER_VERSION`, LRU-evicted above 2 GB (§16.1). With "keep messages on this device" off, bodies stay in memory only. A sanitizer bump re-sanitizes from the cached raw bytes without a fetch.

### D-320 Desktop BYO AI
- **Decision**: the desktop uses the prompt pack embedded in `crates/ai` (`AKATSUKI_PROMPTS_DIR` overrides it for prompt work). Settings → AI providers adds a profile (kind, label, optional base URL, model), stores its key in the keychain (service `vc.akatsuki.ai`, account `<profileId>`), adds this device to `keyOnDevices`, makes it the `default` route and, with the consent switch, sets `consent.grantedAt`; profiles, routes and consent go to `ai.saveSettings`, keys never do. "Test connection" runs from the device. Summaries and instant replies stream AASP events into a panel that shows the egress line (§12.2). A managed (`akatsuki`) profile without a base URL uses `AKATSUKI_API_URL` and sends the user's JWT. Nothing runs without consent and a resolved route.

### D-321 Dark-mode link text and letter spacing on the desktop (**BRIEF "Design theme" implementation**)
- **Context**: the dark accent `#800000` is too dark to read as text on `#0A0A0A`, and GPUI 0.2.2 has no letter-spacing.
- **Decision**: `Theme.link` is the crimson in light mode and the foreground in dark mode; links keep a crimson underline, and fills (buttons, the cursor bar, unread dots, orbs) keep the deep crimson. Uppercase tracked labels approximate `0.12em` with hair spaces between letters. Like D-301 on the landing site, nothing outside `packages/tokens` defines a colour: both are derived from the token palette and opacity scales.

## Web client, part 1: the core (`apps/web`, 2026-10-20)

### D-260 Web env: only `VITE_*`, never the repo-root env files; dev defaults to the local harness (**§18 clients, BRIEF secrets rule**)
- **Context**: the local harness URLs live in the root `.env.local`, next to the owner's root `.env` with real keys. Vite's `loadEnv` on the repo root would read both files into the build process.
- **Decision**: `apps/web` keeps Vite's default `envDir` (the app directory) and reads only `VITE_*` from the process environment or app-local env files. In development, missing Convex URLs default to the documented harness ports (`http://127.0.0.1:3410`, site `:3411`) and the worker to `http://127.0.0.1:8787`; a production build without `VITE_CONVEX_URL`/`VITE_CONVEX_SITE_URL` renders a "not configured" screen instead of failing silently. The dev server is pinned to port 5173, the harness's `SITE_URL` and trusted origin.
- **Why**: no code path in the web app opens a root env file, and `pnpm dev` works with zero setup.

### D-261 `VITE_LOGIN_PROVIDERS` decides which social sign-in buttons the web app shows (**§6.2, §18 addition**)
- **Context**: Google/Microsoft login are env-gated on the Convex deployment (D-054), and Better Auth has no public endpoint that lists the configured providers; probing `sign-in/social` would create OAuth state rows.
- **Decision**: a public build-time variable `VITE_LOGIN_PROVIDERS` (comma-separated, `google`, `microsoft`; unknown names ignored, GitHub not shown in part 1). Empty means email code and magic link only. Added to `.env.example` and to the §18 "Clients" row.
- **Why**: the owner sets the deployment's `*_LOGIN_CLIENT_ID`s and this variable together; no runtime probe, no server change.

### D-262 The UI talks to a `Backend` interface with two implementations: Convex and an in-memory mock
- **Decision**: `apps/web/src/backend/types.ts` is the only data surface screens use (hooks for every §4.2 query the core needs, mutations, auth, and the worker calls). `convex.tsx` implements it with `convex/react` (`usePaginatedQuery`, `useQueries`, optimistic updates from `@akatsuki/mail-model/optimistic`), Better Auth and `@akatsuki/wire`'s worker client. `mock/` implements the same contract over an in-memory store with the shared `applyOps`/`viewContains`/`inverseAction` semantics, a deterministic synthetic mailbox (invented names, `example.*` domains), simulated provider latency and one thread whose archive fails. The mock is selected only in Vite modes `mock` and `test` behind a compile-time constant, so production bundles do not contain it.
- **Why**: component tests and Playwright smoke tests exercise every screen without a backend or network, and the same screens run unchanged on Convex.

### D-263 Split inbox: an "Inbox" tab for the whole inbox, then the splits in order, then "Other"
- **Context**: §3.4 materialises one split key per inbox thread, and the default splits (VIP, News) are narrow, so starting on the first split would usually show an empty list while most mail sits in "Other".
- **Decision**: the inbox view shows tabs `Inbox` (`sys:inbox`), each split by `order` (`split:<id>`), then `Other` (`split:other`); `tab`/`shift+tab` (`split.next`/`split.prev`) cycle them, `g i` opens `Inbox`. Split tabs show unread counts only when `showCount` is set (and `Other`).
- **Why**: nobody lands on an empty tab by default, and users who ignore splits get a plain inbox.

### D-264 IMAP host autodetect: the web app asks the worker at `GET /v1/connect/imap/discover?email=`, with a client fallback (**§7.2 proposal**)
- **Context**: discovery lives in `crates/providers::imap::autoconfig` (built-in table, the domain's autoconfig, the ISPDB, all through the SSRF-guarded client), but §7.2 has no route for it yet.
- **Decision**: the web client calls `GET /v1/connect/imap/discover?email=<address>` expecting the Rust `Discovered` JSON (`{imap, smtp, username, source, appPasswordRequired}`, camelCase). Until the route exists (404/501), or when the worker is unreachable, it falls back to a copy of the built-in table and then to `imap.<domain>:993` / `smtp.<domain>:587` marked as a guess; the server fields stay editable and the guess opens them.
- **Follow-up (worker and wire owners)**: add the route to §7.2, `apps/sync` and `packages/wire` (`ImapDiscovery` type and a `discoverImap` client call); the web app then drops its table copy.

### D-265 Instant undo on Convex: the `mail.undo` optimistic update also re-inserts removed rows
- **Context**: `optimisticApply` patches cached rows but never inserts them (§16.1), so undoing an archive would show the thread again only when the server answered.
- **Decision**: when a gesture is sent, the client remembers the rows as they were and the per-thread inverse (`inverseAction`, D-250). The `mail.undo` optimistic update applies those inverses through `optimisticApply` and puts each removed row back into every cached `mail.listThreads` page whose view contains it and whose `sortAt` range covers it. A snooze is undone with `mail.unsnooze` (a snooze's ops alone would leave its timer active). Gestures are undoable for 10 s (§5.2) with `z` or the toast.
- **Why**: Superhuman-grade undo is instant; Convex still drops the optimistic layer when the server result lands, so there is no reconciliation code.

### D-266 Mail HTML renders on the light "paper" palette in both themes
- **Decision**: the frame's base CSS uses the light token colours (`packages/tokens` TS constants, injected into `srcdoc`) in dark mode too; plain-text bodies follow the app theme. The frame uses the system part of the tokens' sans stack, because the frame CSP (`font-src data:`) cannot load the app's Geist files.
- **Why**: most HTML mail hard-codes dark text or white backgrounds and becomes unreadable when re-coloured.

### D-267 The body frame's height is measured off-frame in an inert shadow root
- **Context**: without `allow-same-origin` (§14.3) the app cannot read the frame's document, and the frame cannot run a script to report its height.
- **Decision**: the prepared HTML is laid out a second time in a hidden, `inert`, closed shadow root of the same width with the same base CSS (minus `url()` values and `href`s); the frame takes that height and re-measures on width changes and image loads. Images allowed into the frame (proxy or `data:` URLs) are the only fetches. The parent CSP (`script-src 'self'`, no inline handlers) is a second guard should the worker's sanitizer ever regress.
- **Why**: no inner scrollbars and no layout jumps, without weakening the sandbox.

### D-268 Part-2 slots: `registerExtension()` for compose, settings, AI and billing
- **Decision**: `apps/web/src/app/extensions.ts` lets a module register routes (`/compose`, `/settings[/<section>]`, the `checkout` onboarding step), shortcut handlers (`compose.new`, `thread.reply`, `settings.open`, `ai.summarize`, ...), palette items and an open-thread footer. Until a module registers, those shortcuts have no handler, so they neither fire nor appear in the palette, and the routes say plainly that the feature is not in this build.
- **Why**: part 2 adds features without touching the shell, and part 1 never presents a placeholder as a feature.

### D-269 Web fixes found while wiring the client (**D-251, D-378 follow-ups**)
- **Token source**: `setAccessTokenSource(authClient)` (`packages/wire/src/token.ts`) misdetects a Better Auth client as a ready `AccessTokenSource`, because the client is a Proxy whose `in` checks are true for any name; `getAccessToken()` then returned an object and every worker call failed. The web app passes a function source that calls `authClient.convex.token()` instead. **Follow-up (wire owner)**: detect sources with `typeof source.getAccessToken === "function"` on a non-proxy, or only accept functions and explicit sources.
- **CSP**: `apps/web/vercel.json` listed `http://[::1]:*` in `connect-src`; Chromium rejects IPv6 host sources with a port wildcard and logs an error on every page, so it was removed (loopback stays reachable as `localhost` and `127.0.0.1`). `vite preview` applies the vercel.json headers and adds `ws://127.0.0.1:* ws://localhost:*` only for the local harness's plain WebSocket.

## Mobile app (apps/mobile, 2026-10-20)

### D-330 Expo SDK 58 with Expo Router; versions pinned to the SDK's bundled set (**BRIEF decision 9, §6.4 implementation**)
- **Context**: the BRIEF fixes Expo + TypeScript but no SDK. On 2026-10-20 npm `latest` is `expo@58.0.16` (SDK 58 since 2026-09-29: React Native 0.88.0, React 19.3.0); SDK 57 is four months older. The workspace already uses React 19.3.1 for `packages/ui` (web).
- **Decision**: SDK 58, every dependency pinned exactly to the version in `expo/bundledNativeModules.json` (no `~`), Expo Router for navigation and deep links, CNG (no committed `android/`/`ios/`), the default Metro config (it resolves the workspace packages' `.ts` import extensions and `with { type: "json" }` imports unchanged). Mobile pins React 19.3.0 because React Native requires its renderer's exact React; the peer warning from `@akatsuki/ui` (19.3.1) is harmless because mobile imports only `@akatsuki/ui/errors`, which has no React code. New native dependencies (MIT): `@shopify/flash-list` 2.3.2, `react-native-webview` 14.0.1, `react-native-gesture-handler` 3.3.0 (v3 hook API), `react-native-reanimated` 4.7.1, `expo-key-event` 1.9.0 (D-339), `expo-build-properties` (D-334). `THIRD_PARTY.md` lists them.
- **Why**: a new app should start on the current SDK (Expo Go and EAS support move with it), and exact pins keep `expo install --check` and the lockfile in agreement.

### D-331 Mobile logic lives in pure view-models tested with Vitest; tokens map to RN styles in one module
- **Decision**: everything that decides something (time presets in the user's zone, list rows and split tabs, swipe thresholds, the thread HTML document and link policy, compose parsing/validation and the undo-send state machine, search chips, AI settings and egress labels, notification copy, plan status, keyboard adaptation, palette ranking, outbox, LRU, instant undo, env parsing, ids) is in `apps/mobile/src/vm` without React Native imports and runs under Vitest 5 in Node like the other packages (no `jest-expo`). Screens are thin. `src/theme/theme.ts` maps `@akatsuki/tokens` to RN styles (tracking in em becomes `letterSpacing` in points, one font family per Geist weight because Android does not synthesise weights, a 1.15 scale for body text on touch screens). Dark-mode accent text uses the deep crimson mixed 50/50 with the foreground, as the landing site does (D-301), because `#800000` on `#0A0A0A` is 1.8:1. Runtime check beyond unit tests: the web export was driven with Playwright against the local Convex harness (OTP sign-in, seeded synthetic Gmail threads, `j`/`e`/`z` archive and undo, thread view, palette, settings).
- **Why**: the parts that can be wrong are tested without a device; nothing else hard-codes a colour (a test compares `app.config.ts` native colours with the tokens).

### D-332 `EXPO_PUBLIC_LOGIN_PROVIDERS` decides the social sign-in buttons on mobile (**§6.4, §18 addition; mirrors D-261**)
- **Decision**: comma-separated `google`, `microsoft` (others ignored); empty shows magic link and email code only. Added to `.env.example` and the §18 "Clients" row.
- **Why**: Better Auth exposes no list of configured providers and probing `sign-in/social` creates OAuth state rows; same rule as the web app.

### D-333 The mobile app sells nothing on iOS or Android; plan status only (**§6.4, §13.1 implementation**)
- **Decision**: Settings shows the plan state from `me.get` (trial end, renewal, cancellation, grace, lapsed, self-hosted) with no price, no purchase or manage button, no link and no text pointing to a website to buy. A signed-in user without `hostedSync` sees the status, Settings and Sign out. The mobile copy for `not_entitled` and `ai_budget_exhausted` overrides `@akatsuki/ui/errors` ("This account has no active Akatsuki Pro plan.") because the shared copy says "Start a trial or subscribe in Settings". The same rule is applied on Android (Google Play payments policy) so both apps behave alike. Account deletion is in the app (App Store guideline 5.1.1(v)): `me.deleteAccount`, apps/api `billing/cancel {immediately:true}`, then Better Auth `deleteUser` if the server enables it, then local wipe.
- **Follow-up (Convex owner)**: Better Auth `user.deleteUser` is not enabled in `convex/auth.ts`, so the login identity survives account deletion until it is.

### D-334 Local-network AI on a phone: an address is required, loopback is refused, cleartext is allowed natively (**§12.2 implementation**)
- **Context**: Ollama without a `baseUrl` means `127.0.0.1` (`device` egress), which on a phone is the phone itself, where Ollama does not run. A LAN endpoint is plain http, which iOS ATS and Android block by default.
- **Decision**: on mobile, Ollama and OpenAI-compatible profiles require a `baseUrl`, and a loopback address is rejected ("localhost is this phone"); the egress label then comes from `egressText` in `packages/ai` ("Stays on your local network (host)"). Native config: iOS `NSAllowsLocalNetworking` plus a local-network usage string; Android `usesCleartextTraffic` through `expo-build-properties` (Android cannot scope cleartext to address ranges). The app still accepts http only where `baseUrlProblem` allows it (loopback, RFC 1918, `.local`), and every Akatsuki endpoint is https outside local development (`src/vm/env.ts`). BYO keys are stored in SecureStore as `ai.<profileId>` (`AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY`), `keyOnDevices` gets this device id, and sign-out deletes the keys this device stored (§16.1 lists bodies, AI cache, outbox and mirror; keys are added because a signed-out phone should not keep API keys).

### D-335 Magic links on mobile are accepted only when requested on this device, for the same address (**§6.4 security refinement**)
- **Context**: the server's Expo plugin appends the new session cookie to the `akatsuki://auth/callback` redirect. Any app or page can open that URL, so a link someone else requested would sign this phone into their account (login CSRF).
- **Decision**: requesting a link stores `{email, at}` in SecureStore; `auth/callback` accepts a cookie only within 15 minutes of a request and only if the session's user email equals the requested address, otherwise it signs the session out and says why. Email OTP and social sign-in are unaffected (their responses come back to the app directly).

### D-336 Compose and undo on mobile (**§10.1–10.3, §16.1 implementation; web D-265 parity**)
- **Decision**: drafts are plain text (`html: null`); the worker adds the identity signature and builds MIME. Autosave runs 2 s after the last change and also keeps a local copy in SQLite (`compose:<clientDraftId>`), which Undo send and failed sends reopen. On `draft_version_conflict` the composer takes the server's `version` and saves its own text over it on the next autosave (last writer wins) instead of calling `adopt`; adopting the other device's text would discard what the user is typing. Send closes the composer at once; the toast counts down `undoSeconds` and Undo calls `sends.cancel` on Convex (tombstone if the worker has not scheduled yet). After `sendAt` the toast says "Sent" (presumptive; a later failure arrives through `ops.failures`). Instant undo of thread actions remembers the cached rows when a gesture is sent and puts them back in the `mail.undo` optimistic update (`src/vm/undoOptimistic.ts`); a snooze is undone with `mail.unsnooze`.

### D-337 Thread rendering on mobile: one sandboxed document per thread (**§14.3 implementation**)
- **Decision**: all messages of a thread render in one `react-native-webview` document with JavaScript, DOM storage, cookies, file access, multiple windows, link previews and data detectors off. Messages and quoted text collapse with `<details>` (no script needed); unread messages and the newest start open. The CSP is §14.3's plus `form-action 'none'; base-uri 'none'`. HTML bodies sit on the light paper palette in both themes (web D-266); plain text follows the theme. Remote images stay blocked until `prefs.remoteImages = "proxy"`, the sender is allowed (`senderPrefs`), or "Show"; only URLs under the worker's `/v1/img/` become `src`. Inline `cid:` images (raster, ≤ 1 MB) are fetched through the worker and injected as `data:` URLs keyed by message and part. Attachment links use an `akatsuki-attachment://` URL that the app intercepts and turns into a 5-minute `POST /v1/links` capability opened in the system browser; every web or mail link opens outside the app; any other navigation is blocked. The web preview build uses an `<iframe sandbox="allow-popups allow-popups-to-escape-sandbox">` instead.

### D-338 EAS configuration without cloud builds; `EAS_PROJECT_ID` and `EXPO_OWNER` (**§18 addition**)
- **Decision**: `app.config.ts` (name Akatsuki, scheme `akatsuki`, bundle id and package `vc.akatsuki.app`, tablet support, splash and icons generated from the tokens and Instrument Serif) reads `EAS_PROJECT_ID` and `EXPO_OWNER` at build time; both are public and listed in `.env.example` and §18. `eas.json` has development (dev client, internal), development-simulator, preview and production profiles with Node 26.7 and pnpm 12.11.2 and the public sync/api URLs; the Convex URLs are to be set as EAS environment variables once Convex Cloud exists. Without a project id the app does not request a push token and says so in Settings. No cloud build was started.
- **Unverified**: EAS images with Node 26 (the repo's `engines` require it); if EAS lacks it, the build profiles need a different Node and `engine-strict` relaxed for EAS only.

### D-339 Hardware keyboards through `expo-key-event`; mobile implements a declared subset of the key map (**BRIEF "Keyboard-first"**)
- **Decision**: key events come from `expo-key-event` 1.9.0 (MIT, community Expo module for iOS, Android and web; `key` is a DOM `code`, mapped to the shared `KeyEventLike`). Bindings come only from `packages/shortcuts` (`createMatcher`, Superhuman or Gmail keymap from prefs; iPad hints use ⌘). `MOBILE_SHORTCUTS` lists the ids mobile implements; a test fails if any shared shortcut is neither implemented nor listed as unsupported. Not on mobile yet: multi-select (`select.*`), "Open original", AI actions (`ai.*`, `compose.aiWrite`), snippets and attachments in compose. Screens register handlers per scope while focused; the palette lists the implemented commands for the scopes of the screen underneath and runs the chosen one through the same handlers, so every action is reachable from the palette on a phone without a keyboard too ("Commands" in the inbox header). The app runs as a development build, not in Expo Go.

## Web client, part 2: compose, settings, AI, billing (`apps/web`, 2026-10-20)

### D-280 The web composer edits plain text; HTML is derived; the worker adds the signature (**§7.3, §10.1**)
- **Context**: a rich-text editor (contenteditable) is a large surface for little gain in a keyboard-first client, and `crates/mime::compose` already inserts the identity's signature and re-sanitizes the HTML part.
- **Decision**: the composer edits plain text. `DraftContent.text` is the body plus the reply quote or forwarded block; `html` is derived from it (`textToHtml`: escaped paragraphs, `<br>`, `>` runs as `<blockquote>`, only http(s) URLs linked). The signature is shown read-only under the editor and never added by the client. Snippet and signature HTML is reduced to text for editing (`htmlToText`) and stored as the same simple HTML. A reply quotes the newest non-draft message once its body is on the device ("On …, X wrote:" plus `> ` lines, collapsed in the editor); a forward appends a forwarded-message block and keeps the original attachments as `{fromMessageId, partId}`.
- **Why**: fast, predictable, testable in jsdom, and one HTML producer per side (the worker owns MIME).

### D-281 Draft conflicts: "Load that copy" (adopt) or "Keep this one" (**§10.1 step 3**)
- **Decision**: autosave is a 2 s debounce over `PUT /v1/drafts/{clientDraftId}`; every edit is written to the device's compose store (IndexedDB `drafts`) at once. On `409 draft_version_conflict` saving pauses and the composer offers: **Load that copy** — `drafts.get` gives `threadId` and `rfcMessageId`, `mail.getThread` gives the draft message with that `rfcMessageId`, and `POST /v1/drafts/{id}/adopt` returns the other device's content and version; **Keep this one** — save again with the version the 409 reported. Empty drafts never create a provider draft; closing an empty composer deletes its local copy and discards a provider draft it had created.

### D-282 AI and Settings load in their own chunks (**BRIEF quality bar: speed**)
- **Decision**: `@akatsuki/ai`, `@akatsuki/prompts` and the AI SDK are imported only through `src/ai/module.ts` (`loadAi()`, preloaded on idle after the mail shell mounts); egress labels render once it is there. `/settings` and the onboarding checkout step are `React.lazy` chunks. The first-paint bundle grows by about 85 kB (27 kB gzip) for compose, reminders, AI panels and sender actions.

### D-283 Undo-send, send later and follow-ups on the web (**§10.2, §10.3**)
- **Decision**: Send closes the composer and shows "Sending…" with Undo for `prefs.undoSendSeconds` (0 = no undo, "Sent." at once); `z` undoes the newest send still in its window, otherwise it is the mail undo. A later send shows "Scheduled for …" with Undo for 10 s and is listed under "Show scheduled messages" (palette) with Change time / Cancel, and with "It was sent" / "It was not sent" for `unknown` sends. After Undo the composer reopens from the device copy with the version `drafts.get` reports (it waits for `/v1/send` first, so the worker's synchronous save is counted). "Remind me if no reply in 1 day / 3 days / 1 week" becomes `mail.setReminder({condition: "if_no_reply"})` once the window has passed; for new mail the thread comes from `drafts.get(clientDraftId).threadId`, polled up to 10 × 3 s, and the user is told when it could not be set.

### D-284 Part-2 backend contract (**D-262 extension**)
- **Decision**: `Backend` gains hooks for identities, snippets, rules, devices, AI settings, `ee.billing.entitlement`, sends, drafts, read receipts and the full sender preference; mutations for every §4.2 part-2 function; worker calls `upload`, `putDraft`, `adoptDraft`, `deleteDraft`, `send`; an `ApiService` over `createApiClient` (managed AI stream, transcribe, quota, checkout, portal, cancel; `configured: false` without `VITE_API_URL`); one-shot `fetchThread` / `fetchDraft`; and Better Auth `listSessions`, `revokeSession`, `deleteUser`. The mock implements all of it (`mock/extras.ts`: CAS on draft versions, undo tombstones, `too_late`, delivery into Sent, canned AASP streams, checkout that starts a trial; plan `trialing` by default, `none` starts at checkout).

### D-285 BYO keys in their own IndexedDB database; sign-out deletes them (**§12.2, D-009**)
- **Decision**: keys typed with "Remember on this browser" are sealed with AES-GCM under a per-browser CryptoKey generated with `extractable: false` and stored (as a structured-clone handle) next to the ciphertexts in the database `akatsuki-keys`; without "Remember" they live in memory for the tab. `keyOnDevices` lists this `deviceId` only for remembered keys. Sign-out (`wipeLocalData`) deletes `akatsuki-keys` as well as the `akatsuki` database (bodies, outbox, drafts, AI cache) and the device copy of the writing-style profile.
- **Why**: a key is useful to whoever signs in next on a shared browser; re-entering it after sign-out is cheap.

### D-286 Blocking a sender also creates a filter rule (**§3.2 `senderPrefs.blocked`; follow-up for the Convex owner**)
- **Context**: `senderPrefs.blocked` is stored but nothing in Convex or the worker acts on it.
- **Decision**: "Block sender" (palette, Settings → Senders) sets `blocked: true` and saves a rule "Blocked: <address or @domain>" (`from_email` / `from_domain` → `trash`) that Convex applies to new inbound mail; the open conversation moves to Trash. Unblocking removes both. **Follow-up**: when Convex applies `blocked` itself (e.g. in `worker.upsertThreads` step 5), the web client stops creating the rule.

### D-287 Filter rules made on the web are scoped to one account
- **Decision**: rule actions are built with `actionsFor(account, labels)` (archive is a `modify` on Gmail and a `move` to the archive folder on IMAP/Graph; labels and folders belong to one mailbox), so the editor always sets `accountId`. Unscoped rules made elsewhere are listed and can be toggled or deleted; editing one scopes it. Block rules (D-286) are unscoped because `trash` is the same everywhere.

### D-288 More extension slots; keys typed in a dialog never reach the mail shell (**D-268 extension**)
- **Decision**: `AppExtension` gains `overlay` (rendered over the mail shell with the signed-in user) and `keyboardScopes` (the composer returns `["compose"]` while it has focus); `notifyExtensions()` / `useExtensionsVersion()` re-render the shell when an extension's state changes. Extension `commands()` only override handlers they define (a missing id no longer erases a part-1 handler), and the composer's handlers are resolved when the key is pressed. `useShortcuts` ignores key events whose target is inside an `.ak-dialog`, so typing "e" in the reminder picker never archives the open thread.

### D-289 Part-2 actions without keys are palette-only; `packages/shortcuts` is unchanged
- **Decision**: Compose, reply, reply-all, forward, send, send later, snippets, attach, Cc/Bcc, write with AI, summarize, suggest replies, unsubscribe, remind and settings use the existing ids. Extract actions, Search with AI, Show scheduled messages, Continue draft, Block sender, rewrite and dictation (buttons in the composer) and every settings section are command-palette entries. Adding keys would need the Rust generated file regenerated by the shortcuts owner.

### D-290 Read receipts: one switch, a warning that must be acknowledged, per-message choice (**BRIEF decision 8**)
- **Decision**: Settings → Mail → Read receipts shows the warning (unreliable; Apple Mail Privacy Protection loads images in advance; recipients are not told; a hint, never proof) and enables only after "I understand" is checked, storing `warningAcknowledgedAt`. The composer then offers a per-message "Read receipt" checkbox (default off); opens of your own messages are listed under the conversation, with proxied opens called out.

### D-291 Stripe return URLs and the checkout step (**§13.1**)
- **Decision**: `/billing` and `/billing/done` route to Settings → Billing (a "Stripe is confirming" note while `session_id` is present and the plan is not active yet). New hosted users (`onboarding: "checkout"`) see a "Start your trial" page; after Checkout the page says it is waiting and moves on by itself when Convex flips onboarding to `connect` (`ee.service.applyStripeSubscription`). Copy: $20 a month after a 14-day trial, card up front, one trial per person, no free hosted tier.

### D-292 Account deletion calls `deleteUser`, which the server does not enable yet (**§4.2 `me.deleteAccount`; follow-up for the auth owner**)
- **Decision**: Settings → Your data → Delete account (type DELETE) runs `me.deleteAccount`, then `POST /v1/billing/cancel {immediately: true}` when a subscription exists, then Better Auth `deleteUser`, then signs out. `convex/auth.ts` does not set `user.deleteUser.enabled`, so the last call fails today and the client continues (the purge still removes every app row; the Better Auth user row remains). **Follow-up**: enable `deleteUser` in `convex/auth.ts`.
- Export downloads one JSON file of profile, prefs, accounts (no secrets), identities, labels, splits, rules, snippets, devices, AI settings (no keys) and billing status.

### D-293 Web BYO triage runs once a minute while the tab is visible (**§12.5**)
- **Decision**: when `consent.autoTriage` is on and the triage route is a BYO or local profile with a key on this device, the web client claims up to 25 threads with `ai.claimTriage` every 60 s while `document.visibilityState` is `visible`, triages them on the device and writes `ai.setTriage`. Triage labels (category and "Needs reply") show on list rows and the open thread.

### D-294 Plain-http endpoints on other machines do not work from the web app (**§12.2 `network` egress**)
- **Context**: `app.akatsuki.vc` is https; browsers block mixed content to http hosts that are not loopback, and the CSP allows `http://localhost:*` and `http://127.0.0.1:*` only.
- **Decision**: the settings schema still accepts RFC 1918 / `.local` http endpoints (the desktop app uses them), but the web UI warns when one is entered: serve it over https, use the desktop app, or run it on this computer. Ollama additionally needs `OLLAMA_ORIGINS` to include the web origin; Test connection says so.

### D-295 Snippet variables and the `;shortcut` trigger
- **Decision**: built-in variables are `{{first_name}}`, `{{name}}`, `{{email}}` (first recipient) and `{{day}}`, `{{date}}` (today in `profiles.timezone`); any other `{{variable}}` stays in the text and the composer says which to fill. Snippets insert with Ctrl/⌘ ; (picker) or by typing `;shortcut` then Tab; `snippets.markUsed` orders them by use.

### D-296 Split editor details
- **Decision**: conditions are edited as field + value rows (all §3.1 fields), match all or any, at most 20; addresses and domains are lowercased before saving. The form previews how many of the newest 50 inbox conversations the rule would take (VIP status is not visible to the client, so `vip` conditions count as false in the preview only). Splits reorder with arrow buttons (`splits.reorder` with an optimistic update) or Alt ↑ / Alt ↓ on a split's Edit button.

### D-297 More natural-language times for snooze, reminders and send later
- **Decision**: `parseSnoozeInput` also understands "an hour", "in 2 months", "end of day"/"eod", "this evening", "morning", "afternoon", "noon", "tomorrow morning/afternoon/evening", "this weekend", "next week", "next month", "next fri [3pm]", and month dates ("nov 3", "3rd of december at 2pm"; a past date means next year), with optional leading "on/at/until/by". Resolved in `profiles.timezone`; only instants reach Convex.

### D-298 IndexedDB schema version 2
- **Decision**: the `akatsuki` database (D-262 era: `bodies`, `outbox`) gains `drafts` (the device's compose store) and `ai` (on-device AI output cache keyed by feature, prompt version, model and the SHA-256 of the input vars; at most 300 entries). The upgrade only adds stores.

### D-130 Owner decisions, 2026-10-20

- **Production Convex is Convex Cloud, never self-hosted.** The owner restated it: Akatsuki's hosted service runs on Convex Cloud. The self-hosted Convex backend is only for the local dev/test harness and for non-commercial self-hosters (`infra/selfhost`). No production doc, script or compose file may point the hosted service at a self-hosted Convex.
- **D-028 approved.** Storing the triage result (category, needsReply, model) and the opt-in voice profile in Convex is approved by the owner. Message bodies are still never stored.
- **Credentials present** (names only): `OPENAI_API_KEY`, `STRIPE_SECRET_KEY` and `STRIPE_PUBLISHABLE_KEY` now exist in the gitignored `.env.credentials`.
- Still open: the two licence points in `LICENSE` §1.6(f) and §3.4 (organisations count as commercial use; compiled builds need permission).

### D-131 Licence: organisations are commercial, anyone may compile (owner, 2026-10-20)

- LICENSE §1.6(f) stays: use by any organisation, including non-profits, schools and public bodies, counts as Commercial Use and needs permission.
- LICENSE §3.4 changed: anyone may compile Akatsuki and distribute builds (including through app stores and package registries) free of charge, under ASAL-1.0 with notices intact. Builds with changes must say so, and they may not pose as official releases or use the Akatsuki marks. Recipients get the same Personal Non-Commercial Use rights. README "Licence in brief" updated to match.
- The owner describes the project as "still open-source, free for personal use". How the public copy words this is pending the owner's answer (see the note in the conversation of 2026-10-20). Until then the existing copy ("source-available, not OSI open source") stays.

### D-380 `crates/shortcuts/src/generated.rs` is rustfmt-skipped, like the tokens file (integration)

- Context: `cargo fmt --all --check` failed on `crates/shortcuts/src/generated.rs`: the shortcuts generator writes empty `Keys { … }` literals over several lines and rustfmt joins them. Formatting the file by hand would make `pnpm shortcuts:build` and CI's "generated files are committed" check disagree.
- Decision: `crates/shortcuts/src/lib.rs` declares `#[rustfmt::skip] mod generated;`, the same rule `crates/tokens` already uses. The generator output is the canonical layout. The file was regenerated and is byte-identical to the generator output.
- Why: one owner of the layout per file; the fmt gate and the generated-files gate can both pass.

### D-381 `MAIL_CA_FILE`: extra trusted roots for IMAP and SMTP (**§18 addition**)

- Context: the IMAP adapter trusts the Mozilla roots only and refuses plaintext to non-loopback hosts. `POST /v1/connect/imap` offers only `implicit` and `starttls`, so a self-hosted mail server with a private CA could not be connected, and neither could the local end-to-end harness (self-signed loopback certificates).
- Decision: optional worker variable `MAIL_CA_FILE`, a PEM file of extra roots. `app::mail_tls` reads it at start-up (unreadable or certificate-free files stop the worker with a message that names the problem, never the content); the roots go into `TlsOptions::extra_roots` of every IMAP and SMTP connection, both for the connect probe (`AdapterImapProbe`) and for syncing (`AdapterFactory::with_mail_tls`). Parsing is `imap::tls::parse_pem_roots`. Added to ARCHITECTURE §18 (with `SPOOL_DIR`, which `.env.example` already listed, D-371/D-355) and `.env.example`.
- Why: certificate verification stays on; trust is widened only by an explicit file the operator controls. No contract change to the wire types or the Convex schema.

### D-382 Local end-to-end harness without Docker: `akatsuki-fake-mail`, `scripts/e2e-local.mjs`, `make e2e` / `make e2e-web`

- Context: nothing exercised Convex, the worker, a mail server and a client together. Docker is not available on the dev machine and no real mailbox may be used.
- Decision:
  - `crates/providers` feature `fake-servers` exposes `imap::fake` (the adapter tests' scripted IMAP and SMTP servers) outside `cfg(test)`; `rcgen` and `mail-parser` become optional dependencies for it.
  - `apps/sync` feature `e2e-fakes` builds `akatsuki-fake-mail` (`src/bin/fake_mail.rs`, `required-features`, so release builds and images never contain it): implicit-TLS IMAP (IDLE, MOVE, UIDPLUS, CONDSTORE, QRESYNC, SPECIAL-USE) and SMTP on random loopback ports, a throwaway login per run, an INBOX of invented mail including one hostile HTML message, both certificates written to a PEM file for `MAIL_CA_FILE`, and a loopback control API (`GET /state`, `POST /deliver`).
  - `scripts/e2e-local.mjs` brings up the native Convex harness, pushes functions, builds, starts the fake servers and the real worker, then drives the API flow (email-code sign-in, connect, sync, sanitized bodies, archive and undo, draft and send, IDLE delivery, removal) with 40 checks. `--web` runs `apps/web/e2e/local/mail.spec.ts` (Playwright: sign-in, IMAP onboarding form, inbox, sanitized body in the sandboxed frame, archive with `e` checked on the server, undo with `z`, removal in Settings). `--serve` keeps the stack running.
  - Secrets: the script never reads `.env.local`. It starts the worker through `node --env-file=.env.local <script> __exec <binary>`, so Node loads the file into that one process (as the Convex CLI does with `--env-file`); variables the script passes win over the file. The worker's symmetric keys for the harness are random, kept in `.convex-dev/e2e/worker-keys.json` (0600, gitignored) so later runs can still open earlier sealed secrets. Sign-in codes are read from `convex logs` for the run's own address only. Playwright gets a minimal environment (no `.env.local` values reach Vite).
  - The worker listens on `127.0.0.1:18787` (or the next free port): 8787 is often taken by unrelated local services. `apps/web/playwright.config.ts` takes `AKATSUKI_E2E_SYNC_URL`. The sign-in code reader moved to `apps/web/e2e/local/otp.ts`, shared by both local specs.
  - Every run removes the accounts it created; run logs are kept for the last five runs under `.convex-dev/e2e/`.
- Why: a repeatable, offline proof that the pieces agree on the contract, which surfaced D-383 to D-385. Self-signed TLS keeps the real TLS code path under test.

### D-383 The worker re-reads a cached user context once before answering `404` for an account

- Context: `Services::user_context` caches `worker.userContext` for 60 s. `POST /v1/connect/imap` reads it before creating the account, so a `PUT /v1/drafts` (or manual sync, or a reconnect start) right after connecting answered `404 not_found` for up to a minute. Found by D-382.
- Decision: `Services::forget_user` drops the cache entry after `createAccount` (IMAP connect and the OAuth callback); `Services::entitled_with_account(user, account)` re-reads the context once when the cached one does not list the account. Draft save, manual sync and reconnect start use it. Tests: `an_account_connected_within_the_cache_ttl_is_found`, and `manual_sync_is_debounced` now expects the re-read.
- Why: one extra Convex read only on a miss; ownership is still checked against Convex, never assumed.

### D-384 Numbers from Convex's HTTP API are normalized before typed decoding in the worker

- Context: the worker's user-JWT calls (`drafts.get`, `identities.list`) use Convex's HTTP API with `format: "json"`, which writes every `v.number()` as a float (`1.0`). `DraftMeta.version: u64` failed to decode, so `POST /v1/send` without an inline draft answered `503 provider_unavailable`. The WebSocket client path already converts integral floats (`crates/convex-api::value::convex_to_json`). Found by D-382.
- Decision: `akatsuki_convex_api::value::normalize_json_numbers` rewrites integral floats within ±(2^53−1) as JSON integers; `HttpUserConvex::call` applies it to every successful value. The wiremock test now sends `"version": 3.0`, as the real API does.
- Why: one rule for both transports; Rust integer fields keep their types.

### D-385 A removing account is acknowledged even when no actor can serve it

- Context: the supervisor built the provider before the actor could see `status: "removing"`. When the secret was missing or unreadable, no OAuth client was configured, or the server was unreachable, it marked the account `degraded`/`reauth_required` (overwriting `removing`) and retried forever; Convex never purged it. Found by D-382 (accounts of earlier runs pointing at stopped fake servers).
- Decision: in `Supervisor::spawn`, a removing account whose secret, OAuth client or adapter is unavailable is acknowledged directly with `worker.ackAccountRemoved` and released (`SpawnError::Removed`; the supervisor records it as removed). Revoking the grant is skipped on that path (it needs a working provider and is best effort, §9.1). Removing accounts are never re-marked degraded. Test: `removing_account_without_a_usable_secret_is_still_acknowledged`.
- Why: removing a mailbox must always delete its metadata, including when the mailbox is already gone.

### D-386 `setAccessTokenSource` recognises ready sources by brand, not by `in` (fixes the cause of web D-269)

- Context: `setAccessTokenSource(authClient)` treated the Better Auth client as a ready `AccessTokenSource` because the client is a Proxy and `"getAccessToken" in client` is always true; worker calls then sent an object as the bearer token. Web and mobile worked around it with a function source.
- Decision: sources from `createAccessTokenSource` carry a non-enumerable `Symbol.for("akatsuki.wire.AccessTokenSource")` brand; otherwise only a value whose own properties `getAccessToken` and `clear` are functions counts as a source. Minting tries `client.convex.token` before calling the fetcher as a function, because the Better Auth client may itself be a callable Proxy. Tests cover a Proxy client, a branded source and a plain object. The web and mobile workarounds keep working and can be dropped later.
- Why: the documented one-line setup (`setAccessTokenSource(authClient)`) now works.

### D-387 Flaky OAuth state test fixed; e2e clippy step in CI

- Context: `oauth::tests::state_is_sealed_bound_and_expires` asserted that the sealed base64url state does not contain `"u1"`, which random ciphertext does about 2% of the time (seen once during integration). The `akatsuki-fake-mail` binary is only compiled with `--features e2e-fakes`, which the workspace clippy run does not enable.
- Decision: the test uses a long marker (`user-sealed-marker`) that random output cannot contain by chance. `.github/workflows/ci.yml` (rust job) adds `cargo clippy -p akatsuki-sync --features e2e-fakes --all-targets --locked -- -D warnings`. CI does not run the end-to-end harness itself: it needs the native Convex backend binary, and the convex CI job already covers pushing functions.
- Why: gates must not fail at random, and code that only builds behind a feature still gets linted.

## Review fixes, round 1 (2026-10-20)

### D-390 Managed triage runs on the worker through `worker.claimTriage` (**§12.5 implementation, §4.3 addition**)

- Context: §12.5 has the worker triage `newInbound` inbox threads for users on the managed (`akatsuki`) route, but nothing called apps/api `POST /internal/v1/triage` or `worker.applyTriage`, and `ai.claimTriage` refuses that route, so Pro users on Akatsuki AI got no triage and no triage-based splits. `newInbound` carries only `{threadId, isInbox}`, not the headers triage needs.
- Decision: a new worker function `worker.claimTriage({accountId, workerId, threadIds?, limit ≤ 25})` (lease-checked) returns `TriageInput[]` with the same projection as `ai.claimTriage` (`triageInputOf`), only when the owner has `managedAi` and the triage route is `akatsuki`, only for inbox threads of that account in state `none` or `pending` with an expired lease, and only for mail of the last 14 days (so connecting a mailbox does not spend the month's budget on old mail). It sets the same 2-minute lease. Without `threadIds` it sweeps the account. In the worker, each account actor starts a `Triager` task (`apps/sync::triage`) beside it when `API_INTERNAL_URL` and the worker service key are configured: `commit` hands it the inbox `newInbound` of every upsert; it claims batches of 25, posts them to `/internal/v1/triage` with an ES256 token (`aud: "akatsuki-api"`, `sub: worker:<id>`), writes only answers for threads the batch claimed with `worker.applyTriage`, sweeps 5 s after start and every 15 minutes, retries a failed batch after its lease (150 s), and pauses an hour on `402` and for `Retry-After` (default 5 min) on `429`. It follows `serverTriage` from `worker.assignedAccounts`. Backfill pages do not trigger it.
- Why: triage runs where the contract puts it, never blocks sync (its own task), stays inside the budget, and recovers from apps/api outages without bookkeeping beyond the existing lease.

### D-391 Protocol fixes: GC watermark for the mirror feed, per-thread FIFO for thread ops only, timed send verification, reauth email after 24 h (**§16.3, §5.4, §5.6, §9.7 conformance**)

- Feed: `accountRevs.gcRev` (schema and §3.2) holds the highest tombstone rev `tombstones.gc` deleted; `sync.changes` answers `resetRequired` when `0 < sinceRev < gcRev` (a pull from 0 never needs tombstones). The GC reschedules itself while it deletes full batches. On `Reset` the desktop also drops the account's search index entries before re-pulling.
- Ops: `send` and `delete_draft` ops no longer carry `threadId` (§3.2 says it is set for kind `thread`), `claimOps` applies FIFO per thread to thread ops only, and `pendingOpsForThread` (hence `pendingOpCount` and visible state) counts thread ops only. A reply's send never waits behind a backed-off archive.
- Verification: `claimOps` returns `op.sendElapsedMs` (time since `sendStartedAt` on Convex's clock). The worker's `verify_due` checks at about +30 s, +2 min and +10 min after the send began and answers `unknown` only after a check at or after +10 min; misses are reported with `countAttempt: false`. The in-memory per-op verify counter is gone, so a worker restart keeps the schedule and verification never runs out of the send's 5 attempts.
- Reauth email: the template is `reauth_needed` (§9.7's name). Every transition into `reauth_required` (worker status report or an op's auth failure in `completeOps`) schedules `internal.accounts.reauthReminder` 24 h later, replacing a pending one for the same account; it sends only if the account is still `reauth_required`.
- Why: each fix makes the code do what the contract already said; tests cover each (convex-test and the engine's fake Convex).

### D-392 The Archive folder of folder accounts is created lazily through a placeholder label (**§5.1 implementation**)

- Context: §5.1 says archive on Graph/IMAP moves to the role-`archive` folder, "created lazily if missing", but nothing created it, so `e` was unavailable on IMAP servers without `\Archive`.
- Decision: a complete `worker.upsertLabels` on a folder account (`caps.multiLabel` false) without a role-`archive` label inserts a placeholder row `Archive` (kind `folder`, role `archive`, exclusive, no `providerLabelId`). Clients see it at once, so `archive()` is `move{to: l:<placeholder>}` on every folder account. The first thread op that adds the key gets a `label_create` op inserted before it (`ensureLabelCreate`), and the move waits for it (§5.2 step 4). The worker runs a role-`archive` `label_create` as `MailProvider::ensure_label(LabelRole::Archive, name)` (finds an existing archive folder or creates one), never as a category or keyword. When the provider reports a label whose name (or archive role) matches a pending row, `upsertLabelsImpl` adopts that row instead of inserting a duplicate (this also fixes a duplicate row after any `labels.create`). When a `label_create` fails for good, the thread ops waiting for it fail with it and roll back.
- Why: the folder exists at the provider only once someone archives, the label key never changes, and the optimistic archive is still instant.

### D-393 Deleting an account deletes the Better Auth login; one trial per person survives deletion (**§4.2 `me.deleteAccount`, §13.1; ends D-292's follow-up**)

- Decision: `me.deleteAccount` now deletes the Better Auth user, its sessions and its linked accounts through the component adapter in the same mutation, so deletion works whatever the session's age (Better Auth's own `deleteUser` demands a fresh session from passwordless users). `user.deleteUser.enabled` is on, and a `user.onDelete` trigger schedules `purgeUser`, so a client calling `deleteUser` alone also purges. The web client cancels the subscription before calling `me.deleteAccount` (the purge removes the billing row apps/api reads). A new table `trialLedger {emailHash, trialUsedAt}` records sha256(lowercased email) when a trial starts (`applyStripeSubscription`); `ee.service.billingIdentity` reports `trialUsedAt` from it when the billing row has none, so `trialEligible` stays false after delete-and-sign-up. PRIVACY.md says what is kept. Not done: matching on the Stripe card fingerprint (needs Stripe calls in apps/api; owner decision whether to add it).
- Why: a deleted account must not leave a working login behind, and the "one trial, ever" rule must not be reset by deletion.

### D-394 Sign-in abuse limits: Better Auth in database storage, plus a per-recipient email limit (**§13.4, §14.6 addition**)

- Context: Better Auth 1.6.33 limits only when `NODE_ENV === "production"` and in memory, which does not work across Convex isolates; nothing limited sign-in emails per recipient.
- Decision: `rateLimit: {enabled: true, storage: "database", window: 60, max: 100}` with `/sign-in/magic-link` and `/email-otp/send-verification-otp` at 3/min and `/sign-in/email-otp` at 10/min, keyed by IP from `x-forwarded-for` / `x-real-ip` (IPv6 by /64). `internal.email.send` limits the sign-in templates (`magic_link`, `otp`) per recipient with `@convex-dev/rate-limiter` (5/hour and 20/day, keyed by sha256 of the lowercased address, never the address) and drops the excess quietly with a content-free log line. Verified with the local end-to-end harness (sign-in still works with database storage). Not verified: which client-IP header Convex Cloud's HTTP actions receive; when there is none, Better Auth skips its IP limit and only the per-recipient limit applies (owner check on the first Cloud deployment).
- Why: one mail-bomb script must not burn the SES reputation that every sign-in depends on.

### D-395 Mailbox OAuth is bound to the user who finishes it; the PKCE verifier stays in the worker (**§6.7, §7.2 change; security**)

- Context: the callback created the account for `state.userId` with no tie to the browser that finished the flow, so a consent link an attacker started could attach a victim's mailbox to the attacker's account. The PKCE verifier travelled in `state`, next to the code; `state` was reusable for 10 minutes; reauth ignored `state.accountId`.
- Decision: `start` keeps `{userId, provider, pkceVerifier}` in the worker under a random single-use `nonce` that the sealed `state` names (10 minutes, at most 10,000 pending, one process per D-044). The callback takes the flow (a second use of the state fails), exchanges the code, seals the grant and keeps it under a random handle, and redirects to `returnTo?status=pending&grant=<handle>` without creating anything. The client calls `POST /v1/connect/complete {grant}` with its JWT; the grant is consumed by the first attempt whoever makes it, and the account is created only when the caller is `state.userId` (`400 invalid_argument` otherwise). A reauth passes `reauthAccountId` to `worker.createAccount`, which refuses a grant for another mailbox. Web (onboarding page), mobile (auth session result) and the web mock follow the flow; wire types `ConnectComplete`/`ConnectCompleteResponse` with fixtures on both sides. A worker restart between start and completion means starting again.
- Why: an attacker can still send someone a genuine consent link, but the grant lands in the victim's browser, where it is refused and burned.

### D-396 Wire, registry and tooling: IMAP discovery route, Rust error registry parity, accepted advisories, exact Rust pins, binding name check (**§7.2, §15, §18 changes; D-264, D-252, D-092 follow-ups**)

- `GET /v1/connect/imap/discover?email=` (worker, through the SSRF-guarded client, 60/h per user) answers `ImapDiscovery` or `404`; `@akatsuki/wire` has the type, a fixture and `WorkerClient.discoverImap`. The web app drops its copy of the provider table and falls back only to the `imap.<domain>` guess.
- `ErrorCode` in Rust gains `consent_required`, `ai_off`, `key_missing` (registry order, HTTP 400 like the other client-only codes); `crates/ai` maps `ConsentRequired`, `FeatureOff` and `KeyMissing` to them; a Rust test runs `packages/wire/fixtures/error-codes.json` (order, HTTP, retryable). `packages/ai` raises `unsupported` instead of the ad-hoc `managed`, and a test checks every literal `AiFeatureError` code against the registry.
- `deny.toml` lists each accepted advisory with its reason (RUSTSEC-2023-0071: RSA used for public-key verification only; ten unmaintained crates reached only through gpui 0.2.2). `cargo deny check` passes, CI runs it whole, and any new advisory fails it.
- Every Rust dependency is an exact `=` pin, the version in `Cargo.lock`; `scripts/check-exact-pins.py` runs in CI.
- `crates/convex-api` stays hand-written until `cargo xtask convex-gen` exists (§18 amended): a test checks that every bound `module:function` is exported by `convex/<module>.ts`; shapes are covered by shared fixtures, convex-test and `make e2e`.
- AI SDK 7 warns that `@openrouter/ai-sdk-provider@3.1.0` (the newest stable) implements the v2 specification; `installAiWarningFilter` drops exactly that notice (tests use it; apps/api already turns warnings off) and the Convex test workers run with `--no-experimental-webstorage`, so test output is clean.

### D-397 Per-user limits in the worker (**§13.4 implementation; supersedes the "not yet (D-091)" rows**)

- Decision: `apps/sync::limits` (in memory, keyed by JWT `sub`, `429 rate_limited` with `Retry-After`): body, thread-bodies, prefetch and part requests 30/s with a burst of 200; raw reads (each may hold 64 MB) at most 4 at once per user and 8 per process, waiting up to 15 s; attachment and raw-message egress 2 GB per UTC day; uploads 200 MB/h, at most 50 files and 250 MB alive per user for the spool's 24 h, and 2 GiB in the whole spool; OAuth start and IMAP connect 10/h; IMAP discovery 60/h. The image proxy limit is set to the contract's 300/min (it was 600). §13.4 "Enforced today" and the nginx comment say so.
- Why: one account must not fill the shared disk, OOM the worker or turn it into a login-testing relay.

### D-398 IMAP and SMTP reach only public addresses on mail ports (**§6.7, §14.4 extension; security**)

- Decision: `crates/providers::imap::tls::tcp_connect` resolves the host once, keeps only addresses that pass `akatsuki_core::net::ip_allowed` (the image proxy's list, moved to `crates/core`) and connects to exactly those; a host with no allowed address is `unreachable`, like one that does not answer. It applies to the connect probe, IDLE, sync and SMTP. `POST /v1/connect/imap` accepts IMAP ports 993/143 and SMTP 465/587/25/2525 only. `MAIL_ALLOW_PRIVATE_HOSTS=1` (new optional env, §18, `.env.example`) lifts both for the local harness (`scripts/e2e-local.mjs` sets it) and for self-hosted installs with a LAN mail server (documented in `infra/selfhost/README.md`).
- Why: user-chosen hosts must not reach the host's private networks or scan its ports.

### D-399 Client fixes from the UI and accessibility review (**BRIEF design and keyboard-first; §14.3, §14.4**)

- Measuring host (web): the shadow root that measures a body's height is transparent, paint-contained and clipped (`opacity: 0`, `contain: strict`, `overflow: hidden`, `clip-path`), set through the CSSOM as well as the stylesheet, and the measured copy drops `visibility` declarations. A Playwright test paints an escaping block into a host styled like the live one and compares screenshots (and checks that the old styles would have painted it).
- Desktop "Show images" loads remote images only through the worker's image proxy: it fetches the worker's sanitized body (proxy URLs), turns only `{sync}/v1/img/` URLs into `src`, and the page's CSP allows images from data: and the sync origin only. The demo has no proxy, so images stay blocked there.
- Accent text: `packages/tokens` gains `accentText` per mode (`#C1272D` light, `#B77777` dark, from D-301's 50/50 lift; a test checks ≥ 4.5:1), emitted as `--accent-text`, `accentText` and `accent_text`. Web, `packages/ui`, landing, mobile and desktop use it for accent-coloured text, links, icons and thin strokes; `accent` stays for fills. Desktop links (D-321) now use it in both modes.
- Keyboard: the landing demo releases Tab, Shift+Tab and Escape. The web sign-in focuses the code field once it is mounted. The mail shell focuses the conversation list on load and after dialogs close; Tab switches splits only when the list has focus (elsewhere it moves focus); F6/Shift+F6 cycle mailboxes, search, list and reading pane; a skip link and a `<main>` landmark exist. F6 is a web-only accessibility binding outside `packages/shortcuts` (its key notation has no function keys yet).
- Narrow screens: below 720 px the sidebar is a drawer behind a Menu button in a compact bar with Command palette and Compose; open threads have a Back bar below 1100 px; rows have two lines below 600 px; toasts sit bottom right (full width on phones, above the reply row). The sidebar has a Settings button. Landing: the palette chip reads "Menu" below 768 px without keycaps on phones or touch screens, the waitlist stays as "Waitlist", the keyboard demo has touch buttons, footer links are ≥ 24 px tall, and scrollable tables are focusable named regions. Mobile web export clips its orbs; email code is the primary sign-in method on every client.
- Smaller: web "Open original" (palette, keywords "print"/"source") opens a wrapper page whose only content is the same sandboxed frame; message bodies render dark in dark mode when they set no colours of their own (`isPlainLooking`), blocked images are sized, labelled boxes instead of broken-image glyphs; the idle reading pane and the AI providers empty state use the empty-state treatment; search parse errors wait for a 0.9 s pause and never flag a trailing operator; `<kbd>` hints carry visually hidden text instead of `aria-label`; settings pages have an `h1`; light-mode previews on tinted rows and keycaps in primary buttons meet 4.5:1; palette commands carry synonyms from `shortcuts.json` (`keywords`) on web and desktop. Mailto unsubscribe shows its destination first on web, mobile and desktop, warns when it is not the sender's domain, and the worker sends only a short token-like subject (else `unsubscribe`). Desktop collapsed messages no longer shrink under their text; the wordmark and TL;DR use token text styles.
- Not done: Geist inside message frames (the frame CSP allows `font-src data:` only, and embedding the font would add about 170 KB per message); a per-message "render dark" toggle; a mute command (needs a thread-level rule the schema does not have).

## Review fixes, round 2 (2026-10-20)

### D-400 A theme or density picked just before a reload survives it; `make e2e-web` runs every local spec (**BRIEF design theme, §4.2 `me.updatePrefs`**)

- Context: `apps/web/e2e/local/auth.spec.ts` failed twice: "Use dark theme" then an immediate `page.reload()`. The reload could land before `me.updatePrefs` reached Convex; on the next load `useSyncServerTheme` saw the older server value (`system`), applied it and wiped the local choice. The spec was in no gate (`make e2e-web` ran only `mail.spec.ts`).
- Decision: `apps/web/src/lib/theme.ts` keeps a theme or density choice under `ak:prefs-pending` (with its time) from the moment it is applied until the save resolves (`setAppearance`). While a value is pending, a different server value does not overwrite it: the local value is applied and sent again (once per page), and the marker clears when the server echoes it; a pending value older than a day is dropped. The mail shell, Settings → Appearance and the settings shell all use it. The spec reloads at once (the product case), then waits for the marker to clear, drops the local copy and reloads again to prove the server holds `dark`. `scripts/e2e-local.mjs --web` runs the whole `e2e/local` directory. The worker-error assertion accepts "not available" because under `make e2e-web` a worker without a Google client answers `provider_unavailable`, whose copy is now generic (D-404).
- Why: the user's last choice wins over a stale server copy without a blocking save or an unload prompt.

### D-401 Better Auth limits only requests with a client IP, in an indexed app table (**§13.4, §14.6; refines D-394**)

- Context: Better Auth 1.6.33 keys every request without a resolvable client IP to one `no-trusted-ip|<path>` bucket. On the loopback harness no proxy adds `x-forwarded-for`, so three OTP emails a minute from anyone blocked every sign-in (`429` in the local suites); on Convex Cloud a missing or multi-hop header would make the 3/min rule global. Its database storage also swept the component's `rateLimit` table with a full scan (`lastRequest` has no index; the default component schema cannot get one without a Local Install, D-034).
- Decision:
  - `rateLimitRules()` makes every rule (the three sign-in paths and a `/**` catch-all that keeps Better Auth's own defaults) a function that returns `false` when the request has no client IP (`getIp`, ignoring Better Auth's 127.0.0.1 development fallback). Such requests rely on the per-recipient email limit (D-394). `AUTH_TRUSTED_PROXIES` (optional, Convex env, §18) feeds `advanced.ipAddress.trustedProxies` so a multi-hop `x-forwarded-for` can resolve once the owner knows Convex Cloud's proxies.
  - `rateLimit.customStorage` keeps the counters in a new app table `authRateLimits {key, count, lastRequest}` (`by_key`, `by_last_request`; §3.2) through one serializable mutation per request (`internal.authLimits.consume`, the same fixed-window decision as Better Auth's) and a 10-minute cron sweep through the index (`authLimits.gc`). The component's `rateLimit` table is no longer written, so its `deleteMany` warning is gone.
  - Better Auth's "could not determine a client IP … shared bucket" warning is dropped by a logger filter (`authLog`): it is printed before the rules run and no longer describes what happens.
- Verified: `make e2e-web` (API flow plus both web specs, three sign-ins within a minute) passes on the harness. Not verified: which header Convex Cloud's HTTP actions carry (owner check on the first Cloud deployment, as in D-394).
- Why: a missing IP header must never turn a per-client limit into a global outage, and rate-limit storage must not degrade with traffic.

### D-402 Filter rules and new-mail pushes apply only to mail that arrives after the mailbox was connected (**§4.4 steps 5 and 8; D-046**)

- Context: `upsertThreadsImpl` treated every message new to Convex (not outbound, not a draft) as newly arrived. Initial sync, backfill, resync re-fetches and provider-search hits (`GET /v1/search` → `SearchFetch` → `worker.upsertThreads`) therefore ran every rule: connecting a second mailbox under a user-wide "from x.com → trash" or a block-sender rule (D-286) would enqueue provider ops for its whole history. The same path scheduled "New mail" pushes for backfilled inbox threads of a live account.
- Decision: a message counts as arrived when it is new to Convex, inbound, not a draft and its provider date (receive time) is at or after `mailAccounts.createdAt` (`arrivedSinceConnect`). Rules (step 5) and push notifications (step 8) use only arrived messages; `newInbound` (snooze early wake, reminders, managed triage with its own 14-day bound, D-390) is unchanged. No worker change is needed: the rule holds whichever sync step brings the message in. Tests: `convex/test/rules-arrival.test.ts` (window, backfill and search hits run no rule; a second mailbox's history does not trigger a user-wide rule; new mail runs it once; pushes only for new mail).
- Why: rules and notifications are about mail that arrives from now on; history the user already dealt with elsewhere must never be moved or announced.

### D-403 Devices and send tombstones are bounded; a push token belongs to one installation (**§4.2 `devices.*`, `sends.cancel`; §14.2**)

- Context: mobile sign-out left the device row and its Expo push token in place, and `devices.register` never took a token from another user, so the previous user's new-mail pushes kept reaching a handed-over phone. `devices.register` and `sends.cancel` inserted rows without caps.
- Decision: mobile sign-out calls `devices.remove` for this device first (bounded to 4 s, skipped offline so the call cannot run later under the next user's session). `devices.register` validates the token (≤ 256 characters) and clears it from every other row (new index `devices.by_push_token`, §3.2), whoever owns it; new devices are rate-limited (`deviceRegister`, 20/h per user) and capped at 50 per user, the least recently seen making room. `internal.notify.newMail` drops tokens Expo reports as `DeviceNotRegistered` (`internal.notify.dropPushTokens`). `sends.cancel` accepts only the worker's client send ids (8–64 of `[A-Za-z0-9-]`) and rate-limits tombstones (`sendTombstone`, 30/min, burst 60). §13.4 lists the limits.
- Why: notifications must follow the person signed in on the device, and no signed-up account may grow Convex without bound.

### D-404 Contract fixes: registry code in the watchdog, a generic `provider_unavailable`, the user's zone in provider search, §18 names (**§3.2, §4.3, §11.1, §15, §18; D-355**)

- `sync.watchdog` writes `statusDetail: "internal"` (our sync service is down, retryable) instead of the unregistered `worker_unavailable`.
- `provider_unavailable` is used by the worker for providers, actors and OAuth clients that are not configured or not running, not only by AI: its §15 meaning and the client copy are now generic ("This service is not available right now. Try again.").
- `worker.userContext` returns `timezone` (`profiles.timezone`, `"UTC"` without a profile; Rust field defaults to `"UTC"`), and `GET /v1/search` resolves `before:`/`after:` in it, like `search.threads` (test with Asia/Tokyo).
- §18: `SHARD_COUNT` is listed for the Convex deployment too (it fixes each account's shard and drives the watchdog; one `lib/shards.ts` helper reads it), plus `AUTH_TRUSTED_PROXIES` (D-401); `VITE_AKATSUKI_BACKEND` and `AKATSUKI_REPO_ROOT` are listed as development names; the landing site reads `NEXT_PUBLIC_APP_URL` (the app host its copy names) and `NEXT_PUBLIC_API_URL`, which nothing used, is gone. `convex/test/env-names.test.ts` checks that the table and `.env.example` list the same names and that every name a Convex function reads is in the Convex row.

### D-405 The Graph push hook checks notifications against the worker's own subscription table (**§7.2, §9.6; security**)

- Context: `POST /hooks/graph` and `/hooks/graph/lifecycle` ran one `worker.accountsForPush` query per notification (up to 1,000 per body) before checking `clientState`, so an anonymous client could drive about 1,000 Convex calls per request.
- Decision: each account actor publishes its live Graph subscription (`subscriptionId` → account and `sha256(clientState)`, from the claim and every renewal) in a `GraphSubscriptions` map on the worker's registry and removes it when the watch goes or the actor stops. The hook refuses batches over 50 notifications (`400`), drops a notification without `clientState` or with a non-GUID id, looks the id up in the map, compares the hash in constant time, and sends one delta hint per subscription and batch. It makes no Convex call at all (an id with no actor here could not be served anyway; one worker process per D-044). nginx gets a separate `akatsuki_sync_graph` zone (50 r/s, burst 200, 256 KB bodies) for `/hooks/graph`. Tests: forged batches never reach Convex; the actor publishes and clears its subscription.
- Why: the cheap gate comes before any work, as Gmail's OIDC check does for its hook.

### D-406 Managed AI charges what was streamed when a stream ends without usage (**§8, §12, §13.4**)

- Context: `streamFeature` reported usage only on success, and apps/api then charged an aborted or failed stream input only (`outputTokens: 0`): a client could read the deltas of a 2,000-token compose answer and disconnect before `done`.
- Decision: apps/api counts the characters it streamed per attempt (deltas, or the size of the latest partial object) and, when an attempt ends without reported usage (client gone, provider error), records the prompt (input vars plus the system prompt) and the streamed output at about four characters a token (`abortedUsage`). `packages/ai` `streamFeature` reports the usage the provider still delivered for a failed attempt (an error part followed by a finish) through `onUsage`, with a bounded wait. Tests: a client that disconnects after 20 of 40 deltas is charged at least those 20; an error part with a usage report charges that usage; a stream that dies charges the streamed estimate.
- Why: the managed budget must cost what the user received; reasoning tokens before the first delta stay an accepted gap (the provider does not report them for an aborted step).

### D-407 IMAP discovery refuses domains that URL parsing turns into IP literals (**§14.4; D-396, D-398**)

- Context: `email_domain` rejected only what `IpAddr::from_str` parses, so WHATWG IPv4 shorthands (`127.1`, `0x7f.1`, `169.254.43518`) passed, the `url` crate normalised them to internal addresses, and the guarded client connects to IP-literal hosts without calling its resolver.
- Decision: `email_domain` also rejects a domain whose last label is all digits or `0x` hex (the WHATWG "ends in a number" rule; no real TLD looks like that), and `discover` skips any template whose host comes from the user's domain when the parsed URL's host is not a DNS name. Tests cover the shorthands and that nothing is fetched for them.
- Why: discovery must reach public DNS names only, like every other user-chosen host (D-398).

### D-408 Mobile: sign-in handoff instead of a cookie in `akatsuki://`, per-message styles, thread AI, app rows in the palette (**§6.4, §3.2, §14.3, §12; BRIEF keyboard-first**)

- Sign-in: Android does not reserve custom schemes, and the Expo plugin put the session cookie into the `akatsuki://` redirect. Magic links and social sign-in now end on the deployment's https origin (`/mobile/auth/handoff?challenge=`), which stores Better Auth's 3-minute one-time token under a random single-use code (table `authHandoffs`, only the code's hash stored, §3.2) and sends the app only `akatsuki://auth/callback?code=`. The app redeems it at `POST /mobile/auth/redeem` with the verifier it kept in SecureStore (base64url sha256 must match; any attempt burns the code), the server exchanges the token for the session and returns the cookie in the response body. Better Auth refuses any non-http(s) sign-in callback (`rejectSchemeCallbacks`), so no flow can put a cookie into a custom-scheme URL. The same-device, 15-minute and same-address checks (D-335) remain. Verified on the local harness end to end (link → handoff → code → redeem → session for the requested address; second redeem and wrong code refused); not run on a device. Verified HTTPS App Links / Universal Links remain an owner option (they need the Apple team id and the Android signing certificate).
- Bodies: one thread is one WebView document, so each message's sanitized `<style>` (scoped only to `.akatsuki-body`) is narrowed to `.akatsuki-body.ak-m<index>` and its wrapper gets that class (`scopeMessageStyles`): one sender's CSS can no longer hide or restyle another message.
- AI: the thread screen has Summarize and Suggest replies (menu, `Shift+S`/`Shift+R`, palette) through packages/ai with the user's route (managed via apps/api, BYO from the device with the SecureStore key); a suggested reply opens the composer with its text; unavailable states show the web copy with "Open AI settings".
- Palette: app-level rows (other themes, density, Settings, Settings: AI, Keyboard shortcuts, Mailboxes, Sign out) next to the key-map commands.
- Copy and shape: sign-in strings come from `@akatsuki/ui/copy` on web and mobile; mobile buttons are pills like web; a disabled primary is an outline with muted text.

### D-409 Web, landing and desktop fixes from the round-2 UI review; image digests (**BRIEF design theme, keyboard-first, quality bar; §18**)

- Help dialog: the dialog is the only scroll container and takes focus on open (Arrow keys, Page Down and Space scroll it); a pinned Close button; F6 and Shift+F6 are listed under Navigation. `/settings/shortcuts` no longer scrolls inside the page.
- Focus: `--ak-focus-color` is `--accent-text` (≥ 4.5:1 in both modes, D-399): a 2px ring 2px off the element everywhere, a 2px field border for inputs, the same for settings links, switches, segmented controls and chips. Tests check the token contrast and that no focus rule uses the accent mid, tint, wash or glow. The landing site's outlines use `--accent-text`. The focused conversation list draws its ring on the active row, not around the column.
- Phones: settings are list-then-detail below 720 px (`/settings` is the list, each page has "All settings"); the composer is a full-screen sheet below 600 px; toasts drop in under the top bar below 720 px; keycap hints are hidden on touch-first devices (`hover: none` and `pointer: coarse`) except in the shortcuts list; AI note links are 24 px tall.
- Split view: replies, reply-all and forwards dock to the bottom of the reading pane (its width, at most half the height) with matching bottom padding, so the newest message stays readable; new messages keep the floating window. With a composer open, toasts sit above it (`--ak-compose-h`) instead of moving over the sidebar.
- Keyboard: after moving to another mailbox or search, focus goes to the list once it has rows, or to the empty state. Shift+S / Shift+R without AI show the reason with "Open AI settings"; an AI request is never replayed when a thread is reopened. Active-row metadata uses the secondary foreground (4.49:1 → above 4.5:1).
- Performance: every bundled font has a WOFF2 sibling listed first in `fonts.css` (about a third of the TTF size; no subsetting, the subsetter is not installed here); the web build preloads Geist Regular and Instrument Serif; everything behind sign-in (onboarding, mail shell, part-2 features) is one lazy chunk, fetched as soon as a session exists (mock entry 641 KB → 462 KB).
- Landing 404: its own title ("Page not found · Akatsuki"), no canonical, and only Next's `noindex` (`robots: null` drops the layout's "index, follow").
- Desktop: the palette lists shortcuts only in their context (thread rows with a thread open, compose rows with a composer open), never the four compose commands the alpha lacks; every toast ends with a full stop (one `sentence()` rule).
- Supply chain: every Dockerfile `FROM`, `*_IMAGE` default and Compose image is pinned by digest next to its tag (digests read from the registries on 2026-10-20), and `scripts/check-image-pins.py` runs in CI. Bumps are reviewed changes of tag and digest together.
- Not done: Latin-subset WOFF2 (needs fonttools); desktop settings pages beyond General, AI and Account (web only for now).

### D-410 IMAP threads go up partial; vanished messages are deleted by id (**§4.3 `worker.upsertThreads`, §4.4 `ThreadUpsert.complete`, §9.3; implements D-223**)
- **Context**: D-223 says IMAP `fetch_threads` returns only the messages this process has seen, so IMAP threads must be upserted with `complete:false`. The engine sent `complete:true` for every provider, so after a worker restart (or an eviction from the 50,000-message index) a new reply or a flag change on an existing IMAP thread deleted and tombstoned all its other messages in Convex and on the desktop mirrors. The engine also ignored `ChangePage.deleted_messages`, which was harmless only while the wrong `complete:true` swept vanished messages away by accident.
- **Decision**: `FetchedThread` carries `partial`. The IMAP adapter marks every `fetch_threads` result partial (index hits and `X-GM-THRID` searches alike: the search covers synced folders only); op post-states (every message of the op's thread ref at its new location) and Gmail/Graph threads stay complete. `ConvexSink::to_upserts` sends `complete: !partial`. `worker.upsertThreads` takes `deletedMessages?: string[]` (provider message ids, ≤ 400 per call): Convex finds each message by `by_account_provider`, adds it to its thread's `removedProviderMessageIds` (a thread not in the call gets an upsert with no messages, which recomputes it or deletes it when empty), and ignores ids it does not know or that the same call upserts. The engine passes the delta's `deleted_messages` (IMAP `VANISHED` and expunge scans, a move's old location) in chunks of 400 next to the thread batches, with the cursor on the last call.
- **Why**: partial lists must never imply deletions; deletions must be explicit, and only the provider's delta knows them. Tests: an engine test runs the real IMAP adapter against the in-process fake server (restart, a new reply, then an expunge), adapter tests check the flags, Convex tests check partial upserts and `deletedMessages`.

### D-411 A trial stays entitled for three days past `trialEnd` until Stripe says otherwise (**§13.3; refines D-122**)
- **Context**: `computeEntitlement` returned `lapsed` for `trialing` once `trialEnd <= now`, and `internal.billing.reevaluate` ran exactly at `trialEnd`. Stripe moves a converting trial to `active` (or `past_due`, `canceled`) only after `trialEnd`, and the webhook can be late or apps/api down, so every converting trial lapsed briefly: queued ops failed with `not_entitled`, `lapsedAt` and purge jobs were set, and the worker paused the mailboxes.
- **Decision**: `trialing` maps to `{state:"trialing", hostedSync, managedAi, until: trialEnd + 3 d}` (the same 3-day slack as `active`, `TRIAL_GRACE_MS`). Only a Stripe snapshot moves it to another state; without one it lapses 3 days after `trialEnd`. A trialing snapshot without `trialEnd` is `lapsed`. `billing.trialEnd` (not `entitlement.until`) remains what clients show as the trial end.
- **Why**: §13.3 maps `trialing` to entitled; `until` is a re-evaluation time, not the end of access. Three days matches `active` and covers webhook retries.

### D-412 Read-receipt rows are created with the draft and linked to the sent message (**§3.2 `readReceipts`, §4.3 `worker.recordDraft`, §4.2 `readReceipts.forMessage`; closes the D-239 gap**)
- **Context**: no function inserted `readReceipts` rows, so `worker.recordReadReceipt` found nothing and `forMessage` always answered null although the web app offered the opt-in.
- **Decision**: `worker.recordDraft` takes `readReceiptToken?` (22 base64url characters, refused otherwise). The worker sends it when the MIME it composed carries the pixel (draft opt-in and `prefs.readReceipts.enabled`). Convex keeps one row per `(userId, rfcMessageId)`: created or kept (token and `draftId` updated) when the token comes, deleted when a later save has no pixel and the row recorded no opens. The row gains `rfcMessageId` and the index `by_user_rfc`; `upsertThreads` sets `messageId` when an outbound, non-draft message with that `Message-ID` is inserted (a moved IMAP copy re-links). Account deletion now purges `readReceipts` through `by_user_rfc`.
- **Why**: the token is derived from `(userId, rfcMessageId)` (D-239), and the `Message-ID` is fixed when the draft is created, so it is the one key both the draft and the sent copy share. Schema change is additive; §3.2 updated.

### D-413 Conversations synced before triage was on become claimable (**§12.5, §3.4**)
- **Context**: `triageState` was set only at thread insert (`off` without a triage route), and nothing moved a thread out of `off`. With the usual onboarding order (checkout, connect, then AI settings) every existing conversation, and every new reply in it, stayed untriaged forever.
- **Decision**: (1) `upsertThreads` moves a thread from `off` to `none` when new inbound mail arrives in it while the user's triage route is on. (2) When `ai.saveSettings` turns triage on (no route before, a route after), it schedules `internal.ai.enableRecentTriage`, which moves the user's threads with mail from the last 14 days from `off` to `none` in batches of 200 through `by_user_triage` (at most 25 batches). The claim paths keep their own inbox and 14-day checks; the worker's periodic sweep and BYO clients pick them up.
- **Why**: §12.5 triages `newInbound` inbox threads of a managed-route user and lets BYO clients claim them; the 14-day window matches `worker.claimTriage`.

### D-414 IMAP passwords are sealed together with their server endpoints (**§14.1, §6.7; security**)
- **Context**: the worker decrypted `{username, password}` (AAD: userId, provider, kind, `username@host`) and connected to whatever `mailAccounts.imap.host/port` and `smtp_host/smtp_port` said. Anyone able to write to Convex could point SMTP (or IMAP) at their own server and collect IMAP passwords on the next send or reconnect, contradicting "a Convex breach yields ciphertext only".
- **Decision**: the IMAP plaintext is `{username, password, endpoints: {imapHost, imapPort, imapTls, smtpHost, smtpPort, smtpTls}}`, hosts lowercased without a trailing dot, written by `POST /v1/connect/imap` from the probed settings. The worker connects to the sealed endpoints and only when `mailAccounts.imap` names exactly the same ones; otherwise (or for a secret sealed before this change, which has no endpoints) the supervisor marks the account `reauth_required` (no hostnames logged) and hands the lease back, and a removing account is acknowledged without a provider. `rotate-secrets` re-seals the plaintext unchanged, endpoints included. §14.1 updated.
- **Why**: AEAD already binds the plaintext; putting the endpoints inside it lets the worker detect any change without a new key or AAD format. Existing IMAP secrets need one reconnect; nothing is deployed yet, so only local harness accounts are affected.

### D-415 The managed AI provider never takes its origin from synced settings (**§12.2, §12.3, §14.5; security**)
- **Context**: for an `akatsuki` profile the desktop sent the user's Convex JWT and the thread text to `profile.baseUrl` when set; settings came from Convex without `validate_settings`, and `ai.saveSettings` accepted a `baseUrl` on `akatsuki`. A stolen short-lived JWT could plant a profile that kept exfiltrating fresh JWTs and threads after the session was revoked.
- **Decision**: (1) `ai.saveSettings` refuses `baseUrl` on `kind: "akatsuki"` (the rule `zAiProfile` and `validate_settings` already had). (2) `crates/ai`: `Provider::for_profile` ignores `base_url` for the managed provider and uses `https://api.akatsuki.vc`; `AiClient::with_managed_api` sets the origin from the device's own configuration (`AKATSUKI_API_URL`, must be https or local http) and `provider_for`, `run_feature` and `dictate` use it. (3) The desktop passes AI settings from Convex through `validate_settings` and treats invalid ones as AI off; its settings form no longer writes the API origin into the profile. Web and mobile already used their build-time `VITE_API_URL`/`EXPO_PUBLIC_API_URL`.
- **Why**: the JWT is a bearer credential for Convex and the worker; where it goes must be decided by code and local configuration, never by data a session can write.

### D-416 Uploads stay in the spool for their 24 h (**§10.4 change**)
- **Context**: the worker deleted an upload as soon as it was attached to a provider draft. Every later save of the same draft (each autosave after attaching, the send, a draft reopened by undo-send) composes the whole MIME again and references the same upload id, which then failed with "An attachment expired. Attach it again."
- **Decision**: uploads are no longer deleted on attach; the sweeper deletes them 24 h after upload (per-user and spool-wide caps unchanged, D-397). PRIVACY.md and §10.4 say so. A draft edited more than 24 h after attaching still gets that error and the user attaches again.
- **Why**: a draft is re-composed on every save; tracking which provider draft already holds which file would need state the worker does not keep (§9.2). The file was going to sit at the provider anyway; the encrypted spool copy lives at most a day.

### D-417 Web: the palette gives focus back, the search field follows the list, labels can be created and managed, phone toasts at the bottom (**BRIEF keyboard-first, §4.2 labels.*, D-409**)
- **Palette**: `CommandPalette` remembers the focused element when it opens and returns focus to it when it closes (Escape, outside click or after a command that did not move focus itself). The mail shell's "focus the list when focus fell to the page" fallback waits one more tick, because Radix restores focus in a timeout after the content unmounts. Compose commands focus what they act on: Add Cc/Bcc the new field, Rewrite the body. Typing after Ctrl+K, Escape lands in the draft.
- **Search**: the field shows the query of the list on screen (the search's own on `/search`, empty elsewhere), reset whenever the route or account changes; the last search is the first suggestion in the empty field ("Last search").
- **Labels**: the label picker (`l`) offers "Create label “…”" when the typed name matches no label; it calls `labels.create` and adds the label to the targets (the thread op waits for the label op, §5.4). Settings has a Labels page (new, rename inline, delete with confirmation) through `labels.create/rename/remove`, with optimistic list updates on Convex. The move picker has no create row: `labels.create` makes non-exclusive labels, folders come from the provider. Mobile's label picker has the same create row; desktop labels and mobile rename/delete are follow-ups.
- **Toasts on phones** (≤ 720 px) dock at the bottom above the safe area as slim bars instead of under the top bar (replaces that part of D-409), so they never cover the search field or a thread's toolbar; with the full-screen composer (≤ 600 px) they sit above its footer (`--ak-compose-foot-h`).

### D-418 Attachments on desktop and mobile (**§10.4, §7.2 `POST /v1/uploads`**)
- **Desktop**: `compose.attach` (Ctrl/⌘+Shift+A) opens the system file dialog (xdg-desktop-portal on Linux; a toast suggests drag and drop when it is unavailable); files dropped on the composer are attached too. Each file shows as a chip ("Uploading…", its size, or the reason it failed) with a remove button; the backend reads it (≤ 25 MB, never read when larger), uploads it raw with `Content-Type` from the extension and the percent-encoded `X-Filename`, and the next autosave carries `{uploadId}`. Send waits for uploads and refuses failed ones or more than 25 MB. A send the worker refuses reopens the draft with its attachments. `compose.attach` left the desktop's unsupported list; send later, snippets and AI write stay there. `AKATSUKI_SCRIPT` accepts `attach=<path>` for smoke runs.
- **Mobile**: an Attach button and `compose.attach` open "Files" (`expo-document-picker` 58.0.4) or "Photos and videos" (`expo-image-picker` 58.0.10, photo library only, no camera or microphone permission); files are read with `expo-file-system` 58.0.10's `File` (a Blob `expo/fetch` streams) and uploaded through `WorkerClient.upload`, which now takes an explicit `contentType`. All three are MIT. The compose state keeps attachments in its local copy, so Undo reopens them.
- **Not done**: snippets and AI write in the desktop and mobile composers, desktop send later.

### D-132 Logo is a red dot (owner, 2026-10-20)

The owner set the brand mark: "Logo should be a red dot simply." The Akatsuki logo is a solid red dot in the brand accent (#C1272D on light; a dawn shade that keeps at least 3:1 contrast on dark). It is used for the site header, favicons, app icons, splash screens and OG images. "Akatsuki" set in Instrument Serif may still appear as the product name, but it is not the logo.

### D-430 The landing site is "First Light", and the red dot is its logo (**§18; replaces the first apps/landing; applies D-132**)
- **Context**: the first `apps/landing` (orbs, card grids, a feature list) was redesigned outside the repo over five rounds of review into "First Light": the page is the hour before dawn with one light source, a crimson horizon that comes off the top edge of a working demo inbox. The owner then set the brand mark (D-132): "Logo should be a red dot simply."
- **Decision**: `apps/landing` is replaced by the redesign; the old code is gone (no files kept). Structure:
  - Next.js 16.4.2 static export as before (same package name, scripts, `vercel.json`, sitemap, robots, OG), but **no React on the client**. Every page is static HTML; `scripts/finish.mjs` (after `next build`) bundles `src/client/` with esbuild 0.28.2 into `app.js` (home, ~20 kB gzipped: demo inbox, keyboard, timeline, model picker) and `site.js` (other pages, ~4 kB: palette, shortcuts sheet, menu, theme, contents), strips Next's client runtime and gives the generated images `.png` names. Everything renders in its final state without JavaScript, for crawlers and full-page captures, and with reduced motion.
  - Home: hero with the working inbox (keys go to it while it is half in view; Esc, Tab and Space are never captured), a typographic timeline for speed (key down, next frame, background, confirmed; it replaced a second, zoomed copy of the inbox), a quiet keyboard with one key lit and its command named under it (phones get six ruled specimen rows instead), the model picker, the "where your mail lives" night band, one enormous price, and the finale. No numbered section eyebrows; each section has its own composition.
  - The logo is the red dot alone: the header link (named "Akatsuki" for assistive tech), favicon, Apple touch icon, manifest icons and the OG/Twitter image (the headline in Instrument Serif with the dot rising on a horizon). The footer's giant gradient wordmark is replaced by the dot rising out of a full-bleed horizon (it rises once on first view; reduced motion shows it risen). Only the hero and the finale headlines end in a period, and that period is the dot; no other heading on the site has one.
  - Contrast: meta text is a 60% foreground mix (at least 4.7:1 even on the tinted surfaces), the headline's second tone a 48% mix (at least 3.6:1, used only at 40 px and up). axe reports no contrast failures on any page at 1440, 768 and 390 in either theme.
  - Content still comes from the repository at build time: `docs/PRIVACY.md`, `SECURITY.md`, `LICENSE` (`lib/repo.ts` refuses `.env*` and paths outside the repo); key labels and the demo's bindings from `@akatsuki/shortcuts`; colours from `@akatsuki/tokens` (D-431).
- **Why**: the owner chose this design; the dot is the brand everywhere (D-433). Shipping no framework runtime keeps the home page's script to the demo itself.

### D-431 Landing-only colour tokens and subset fonts live in apps/landing; `dawn` and `onAccent` are proposed for packages/tokens (**§18, tokens**)
- **Context**: the site needs three colours the token package does not have: the dot/horizon red in the dark theme (the dark `accent` #800000 is a fill, 1.8:1 on the dark background), text on accent fills, and the dark plane of the inverted band while the page is light. It also needs small fonts: the token package ships full woff2 files (about 250 kB for the six faces).
- **Decision**: `apps/landing/src/app/landing-tokens.css` is the only file with colour literals: `--dawn` (the light accent, #C1272D, in both themes: 5.8:1 on white, 3.4:1 on #0A0A0A), `--on-accent`, `--night-bg`, `--night-bg-lifted`, `--night-fg`. A unit test checks each against `packages/tokens/tokens.json`; everything else reads `tokens.css`. `public/fonts/*.woff2` are Latin subsets made from `packages/tokens/fonts/*.ttf` by `pnpm --filter @akatsuki/landing fonts` (about 67 kB for all six); the OG images read the TTFs from the token package. **Proposal**: add `color.light.dawn`/`color.dark.dawn` (= #C1272D) and `onAccent` (#FFFFFF) to `packages/tokens/tokens.json`, so web and mobile can draw the dot in the dark theme without their own literal (web sets `--ak-dot` from `color.light.accent` at runtime today, D-433); the landing file then shrinks to the night band.
- **Why**: the packages were not to be edited in this change; the test keeps the copies from drifting until the proposal is decided.

### D-432 The landing CSP allows its one inline script by hash (**§18, `apps/landing/vercel.json`; security**)
- **Context**: the old CSP needed `script-src 'unsafe-inline'` because a static export carries Next's inline RSC payload. The redesign strips that payload, so the only inline script left is the theme boot script (`src/lib/boot.ts`), which must run before first paint.
- **Decision**: `script-src 'self' 'sha256-…'` with the boot script's hash; `style-src` keeps `'unsafe-inline'` (each page's stylesheet is inlined for first paint, and the keyboard uses style attributes); `img-src 'self' data:` for the grain texture; `connect-src 'self'`; no third-party origin anywhere. `scripts/finish.mjs` fails the build if any page carries an inline script whose hash is not in `vercel.json`, and a unit test checks the hash against `BOOT_SCRIPT`. The Playwright suite runs under these headers (`scripts/serve.mjs` applies them) and fails on any console error, so a CSP violation fails CI.
- **Why**: removes `'unsafe-inline'` for scripts without a server; a changed boot script cannot ship with a stale policy.

### D-433 The red dot replaces the 暁 mark in every app (**brand, D-132**)
- **Decision**: one generator, `apps/landing/scripts/brand-icons.mjs` (ImageMagick 7; colours read from `packages/tokens/tokens.json`), writes the source shape `packages/tokens/brand/dot.svg` and every raster icon; the outputs are committed. Web: the favicon (an inline SVG built from the tokens in `main.tsx`) is the dot, and the sign-in, sidebar, settings and splash marks are plain dots (no glyph, no glow) in `--ak-dot`, which `main.tsx` sets to the light accent so the dot reads on the dark background; "Akatsuki" stays as the product name beside it. Mobile: `icon.png` (opaque, dot at 50% on white), `adaptive-icon.png` (dot inside the safe circle; `app.config.ts` now puts it on the light background instead of the accent, which would hide it), `splash-icon*.png`, `notification-icon.png` (a white dot Android tints with the accent) and `favicon.png`. Desktop: no bundle icon exists yet and its in-app wordmark is text, so nothing changed; a future bundle icon should come from the same script.

### D-133 Public wording: open source, free for personal use (owner, 2026-10-20)

- **Decision**: public copy describes Akatsuki as **"open source, free for personal use"**, always with the commercial-use caveat nearby: commercial use, including use by any organisation, needs written permission (sami@samihindi.com). Short UI strings carry the phrase alone (landing footer "Open source, free for personal use · ASAL-1.0 · No trackers on this site", the web and desktop sign-in footers, the mobile About footer); every page that names the licence also says commercial use needs permission. The pricing FAQ "Is Akatsuki open source?" now answers "Yes, and it’s free for personal use."
- **Supersedes**: the pending wording point in D-130/D-131 (the copy no longer says "source-available, not open source") and the BRIEF rule that forbade calling it open source (BRIEF "What we are building", CONTRIBUTING "Copy"), which now state the new rule.
- **Unchanged**: the licence keeps its name and identifier, the Akatsuki Source-Available License (ASAL-1.0); renaming it is a separate owner decision. In `LICENSE` only the header paragraph changed; every numbered term is as before.
- **Accuracy**: one plain sentence that ASAL-1.0 is not an OSI-approved licence, because it reserves commercial use, stays in the long-form places only: the README licence intro, the `LICENSE` header, the landing /license page ("What “free for personal use” means.", which replaces "Why not open source.") and CONTRIBUTING.
- **Enforced by**: `apps/landing/test/content.test.ts` (the phrase is the only way the site says "open source"; "source-available" survives only in the licence's name; every rendered page that names the licence carries the caveat), `apps/landing/e2e/smoke.spec.ts` (the /license page) and `apps/web/test/styles.test.ts`.

## Mobile app, part 2: the rest of the feature set (`apps/mobile`, 2026-10-20)

### D-460 Multi-select and bulk actions on mobile (**BRIEF keyboard-first, `select.*`; §5.2, §16.1**)
- **Decision**: a long press on a row (or `x` on a keyboard) starts selection mode with a haptic tick; taps then toggle rows, swipes and pull-to-refresh are off, Android Back and Escape leave it. Shift+J/K extend from the anchor, Ctrl/⌘+A selects every loaded row (again: none). A bar on top counts ("3 selected", "All 30 selected") and a bottom bar runs the first four actions of `bulkActions` (archive or move to inbox / restore / not spam, snooze, read or unread, trash) with the rest under More (star, label, move, spam). Mixed selections get the action that changes something for at least one row, as on web. The list's action keys (e, #, !, u, s, h, l, v) act on the selection while there is one. Bulk actions go through the existing `mail.apply` path per account, at most 500 ids per call, one Undo for all calls. The label, move and snooze pickers are routes, so a selection reaches them through `state/targets.ts` (`bulk=1`), never through route params; bulk labels and folders apply to the threads of their own mailbox, Inbox to all. Pure logic in `src/vm/selection.ts`.
- **Why**: the BRIEF's multi-select, with the touch idiom (long press) and the shared key map; route params cannot carry hundreds of ids.

### D-461 Dictation on mobile: expo-audio, an explicit Record, bytes-built multipart (**§12.1 `dictation`, §12.7 rules 1, 4, 6; D-128**)
- **Decision**: `expo-audio` 58.0.9 (MIT, the SDK's bundled version, exact pin) records mono AAC at 64 kbit/s (about 5 MB for the 10-minute cap; `.webm` in the web preview). The Dictate sheet in the composer is mounted only while open, shows the `dictation` egress label before anything happens ("Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring" for the managed route, "Stays on your local network (host)" for a Whisper server), and asks for the microphone on the first Record. The recording stops at 10 minutes, is transcribed with the user's dictation route and deleted from the phone afterwards (also on Cancel). Routes: managed → apps/api `POST /v1/ai/transcribe`; `openai` → `api.openai.com/v1/audio/transcriptions` with the SecureStore key; `openai_compatible` → the user's server. React Native cannot build a Blob from bytes (its BlobManager refuses ArrayBuffer views), which breaks both FormData-with-bytes and the AI SDK's `transcribe` (it wraps audio in a Blob), so mobile builds the multipart body as bytes (`multipartBody` in `src/vm/dictation.ts`, byte-exact test) and calls the OpenAI-style endpoint itself; route, model and consent still come from packages/ai (`resolveFeature("dictation")`, `AUDIO_MAX_*`). A default route on a provider without a transcription API yields the new status `no_dictation` ("Choose Akatsuki AI, OpenAI or a Whisper server for Dictation in Settings, AI"); Settings, AI gains a "Per feature" section (`routes[]` per feature, dictation limited to transcribing kinds) and a "Dictation model" field for OpenAI and OpenAI-compatible profiles (`models.transcription`). Native config: the `expo-audio` plugin with a microphone usage string; background recording and playback off. D-418's "no microphone permission" no longer holds.
- **Verified**: in the web preview with Chromium's fake microphone against a mock OpenAI-compatible server (the multipart reached it, the transcript landed at the cursor). Not run on a device.

### D-462 Write with AI and rewrite on mobile; egress labels on every AI surface; keys read when a feature runs (**§12.1 `compose`, `rewrite`; §12.2; §12.7 rule 7**)
- **Decision**: "Write with AI" (button, `compose.aiWrite` ⌘J, palette) opens a sheet with the instruction, quick prompts and the four rewrite styles; the `compose` / `rewrite` egress labels sit above them. The model's text streams into the body in place of what it replaces (rewrite: the selection, else the body), the body is read-only meanwhile with Stop, and afterwards "Written with AI. Read it before you send." offers Undo / Keep. A reply's transcript uses bodies already on the phone (`cachedBody`), else the snippets. One hook, `src/lib/ai.ts` `useAiEnv`, now serves thread AI, compose, Search with AI and dictation: the status check (`aiFeatureStatus`, web's states and copy), `egressText` for any feature, and the packages/ai context. BYO keys are read from SecureStore only when a feature runs (thread AI used to read them on mount), and a store failure becomes a sentence, not a spinner. The thread AI panel now shows where the summary and the suggested replies went (it had no label).

### D-463 Snippets on mobile; the snippet helpers move to `packages/mail-model/snippets` (**§4.2 `snippets.*`, D-295**)
- **Decision**: `@akatsuki/mail-model/snippets` (new, additive subpath) holds variable discovery and expansion, the built-in variables (`first_name`, `name`, `email`, `day`, `date` in the user's zone), the `;shortcut` trigger, ranking, `applySnippet`, the `snippets.save` checks and the plain text ↔ simple HTML conversion. Variable names follow Convex's rule (≤ 32 characters; web's copy allows 40, which Convex refuses). Mobile inserts snippets from a picker sheet (button, `compose.snippet` ⌘;) at the cursor or over the selection, or offers "Insert “Thanks” (;thx)" above the keyboard when a `;shortcut` before the cursor names one (a phone has no Tab); missing variables are named in a toast; `snippets.markUsed` orders them. Settings, Snippets lists, creates, edits and deletes them. **Follow-up (web owner)**: `apps/web/src/compose/text.ts` can import these instead of its own copy.

### D-464 Label rename and delete on mobile; user folders are rename-only (**§4.2 `labels.rename` / `labels.remove`; D-417 follow-up**)
- **Decision**: Settings, Labels lists the labels the user made per mailbox (web's `editableLabels` rule) with New, Rename and Delete (confirmation; optimistic list updates like web). User folders (exclusive, no role, not `Akatsuki/…`) are listed separately and can only be renamed: deleting a folder at an IMAP or Outlook provider deletes the mail in it, so that stays an act at the provider. Labels created on an IMAP server without keyword support come back from sync as folders (seen with the fake server), which is why they must show up here at all. Delete and rename go through the same ops as web.

### D-465 IMAP host autodetect on mobile; the discovery helpers move to `packages/wire` (**§6.7 step 5, D-396, D-407**)
- **Decision**: `packages/wire/src/imap.ts` (additive export) has `emailDomain`, `guessImap`, `parseImapDiscovery`, `discoverImapSettings` (worker `GET /v1/connect/imap/discover` first, then `imap.<domain>:993` / `smtp.<domain>:587`) and `tlsForPort`, moved from `apps/web/src/onboarding/imapDiscovery.ts` (web keeps its copy for now; follow-up to switch). The mobile Connect IMAP sheet asks 600 ms after the address stops changing, fills only fields the user has not edited, says whether the servers were found or guessed and when the provider needs an app password, and has a Security toggle (SSL/TLS ↔ STARTTLS) for unusual ports; form logic in `src/vm/imapForm.ts`.

### D-466 Scheduled messages and Search with AI on mobile; the new palette-only commands (**§4.2 `sends.*`, §10.2, §5.6, §11 NL search; D-283 parity**)
- **Decision**: a Scheduled screen (Mailboxes, Settings, palette `sends.scheduled`) lists later sends with Change time (the send-later time picker → `sends.reschedule`) and Cancel, failed sends, and unknown sends with "It was sent" / "It was not sent" (`sends.resolveUnknown`). Search has "Search with AI" (button when the query has words, palette `search.ai`): `nlSearchToQuery` with the user's label and split names, the result replaces the query (editable, shown as chips), with the model's explanation and the `nl_search` egress label. The palette-only ids another agent added to `packages/shortcuts` are implemented where they make sense: `labels.create/rename/remove` open Settings, Labels; `compose.continueDraft` opens Drafts; `compose.rewrite` / `compose.dictate` open their sheets; `search.ai` / `search.fullMailbox` run in Search. Still not on mobile: `thread.openOriginal`, `ops.retry` (failure toasts already have Retry), `ai.extractActions`.

### D-467 Startup states never spin forever: secure-storage failures and the Convex token (**§6.4; owner's iOS simulator note**)
- **Decision**: every SecureStore call is guarded (`src/lib/secure.ts`); the first failure is recorded and the root layout shows "Secure storage is unavailable" with the platform's message and Try again (for example a simulator build without a keychain entitlement) instead of the app. Nothing reads the store on import. The signed-in shell now waits for `useConvexAuth()` before any query runs: right after sign-in the Convex client could lag the session by a moment, queries went out unauthenticated and `me.get` threw (seen in the web preview); after 12 s the wait says "Still connecting to Akatsuki" with Sign out. A new message opened before the identities query answered now gets its From when they arrive (it stayed "Connect a mailbox to send").

### D-468 The Docker-run local harness cannot verify user JWTs (**DEV.md harness; follow-up for the infra owner**)
- **Context**: now that `loki` can reach the Docker daemon, `make convex-up` picks Docker. Inside the container, `CONVEX_SITE_URL=http://127.0.0.1:3411` is the container's own loopback, so the backend cannot fetch `/api/auth/convex/jwks` ("Could not fetch JWKS from URL"), every Better Auth JWT is rejected and clients see `unauthenticated` after a successful sign-in.
- **Decision (for now)**: the mobile smoke ran on `AKATSUKI_CONVEX_RUNTIME=native` (the verified path). **Follow-up**: give the container a site URL it can reach (for example its internal port) or default the harness to native.

### D-469 First Light on mobile: ink, hairlines and one red (**app-design SPEC §1, §2, §4; mobile counterpart of D-480–D-484**)
- **Decision**: `Orbs.tsx` and every glow are deleted (sign-in, auth callback, empty states, palette, buttons, the FAB). `src/theme/theme.ts` maps the §2.3 roles from the tokens as opaque blends of fg over bg (`surface` 4%, `surfaceStrong` 7%, `tertiary` 0.7, `muted`, the three line weights) plus `dawn` and `onAccent` (D-481); the light `surface` is no longer the accent wash, and `selected` is `surfaceStrong`. Mobile text sizes are set directly (15 body, 16/1.55 reading, 17/600 header titles, 15/600 only for the unread sender) instead of the ×1.15 scale; `label` is now the sentence-case section style and `crumb` the mono caps (thread crumb, AI provenance, row chips only). Buttons follow D-483: `primary` ink, `brand` accent (the sign-in code button and Sign in only), `secondary` hairline, `ghost` tertiary text, `destructive` accent text. Switches and radios are ink (`Toggle`, `Choice`), settings groups sit on `surface` with radius lg and inset hairlines, sheets use radius xl, the palette and toasts are the only lifted things (`theme.shadow.pop` from `effect.popOpacity*`). Carets and selection are dawn.
- **Screens**: the inbox header is "Inbox ⌄" (the mailbox sheet) with search and compose icon buttons; the accent FAB is gone (a long press on search opens the command palette, so every command stays reachable without a keyboard). Split tabs are ink-underlined with mono counts. Rows are 64pt, two lines (sender and mono time; subject with a muted snippet), a 6pt ink unread dot, an ink star, a paperclip, mono-caps Draft / Needs reply chips, a 2pt dawn bar for the iPad keyboard cursor. Swipe right is an ink layer, swipe left neutral (Snooze), a long left swipe the one accent layer (Trash); a light impact haptic on commit; springs back with damping 20 / stiffness 300. The thread has a chevron, star and More icons, the mono crumb (`INBOX · 2 MESSAGES`), the serif subject, a one-line "N images hidden · Show · Always", and a toolbar of icon-over-label actions with one ink Reply pill. Compose has Close / centred title / ink Send, 48pt fields, "Cc, Bcc, From" collapsed, a `surface` accessory bar (attach, send later, snippets, dictate, Write with AI, discard), and swipe-to-dismiss on iOS with "Keep this draft?" when something is written (`usePreventRemove`). Sign-in drops the wordmark: the 12pt dot, "Mail at the speed of thought" with a dawn-dot period (the accessible name keeps the "."), the brand pill, a static horizon (`react-native-svg` gradients; RN's `experimental_backgroundImage` does not render in the web build) and the open-source footer. Inbox zero gets the horizon and serif 36; other empty states are serif 32 with one muted line. Toasts are one ink pill (an info toast replaces the previous one; up to two errors stack, each with the `!` glyph instead of a red border). The WebView document uses 16/1.55 `secondary`, ink underlined links, 15/500 names, mono times, an 8px paper radius.
- **Reduced motion**: toasts fade in 80 ms without the slide, every sheet appears without sliding (`SheetModal`), and swipes neither slide out nor spring.
- **Checked**: three critique rounds on the Expo web export against the local harness at 390×844, light and dark; final screenshots in the session scratchpad `app-design/mobile-final/` (40 images). The functional smoke (14 flows) passes on the restyled UI.

### D-470 Mobile icons come from the shared SVG set through a generated module (**SPEC §2.11**)
- **Decision**: `react-native-svg` 15.16.0 (MIT, the SDK's bundled version, added with `npx expo install`). `apps/mobile/scripts/icons.mjs` reads the 33 stroke SVGs in `apps/desktop/assets/icons` (the single source until the set moves to `packages/ui/icons`, SPEC §2.11) plus four mobile-only glyphs in `apps/mobile/assets/icons` (mic, chevron-left, reply, alert-circle) and writes `src/components/_generated/iconData.ts`; `test/icons.test.ts` fails when the file is stale. `Icon` draws them at 20pt with a 1.75 stroke in the text colour. When the set moves, only the script's source paths change.

### D-471 AI provenance on mobile uses mono-caps short forms with the full sentence as the label (**SPEC §3.8, §4.4, §4.9; §12.2**)
- **Decision**: results (summary, suggested replies, Search with AI) carry a mono-caps line such as `SUMMARY · ON YOUR NETWORK` (`src/vm/egress.ts`), whose accessible label is the full `egressText` sentence; where the user decides to send something (the Dictate and Write with AI sheets, AI settings) the full sentence is shown under it. Managed dictation's short form keeps the retention ("OpenAI via Akatsuki · kept up to 30 days", D-128). No tinted fills: the egress badge lost its accent wash.

### D-472 Mobile design items left for later (**SPEC §4**)
- Not done in this pass: the ≥ 768pt (iPad) three-pane shell; the "Settings › Mail › Swipe actions" picker; the header hairline that appears only once content scrolls under it; the label picker's check column and mixed state; the snooze sheet's "Bring back only if nobody replies" row (needs a Convex argument); loading Geist into the WebView (the CSP allows `data:` fonts only, so it would mean inlining the font per document; the stack falls back to the system UI font); 84pt rows with previews (no "Show previews" setting yet).

### D-480 First Light in the web app: no decoration, one horizon (**app-design SPEC §1, §2.10, §3; web**)
- **Decision**: the web app and `packages/ui` follow the landing's demo inbox (`apps/landing/src/app/inbox.css`). Deleted: `packages/ui/src/Orbs.tsx` and every `.ak-orb*` rule (`usePrefersReducedMotion` moved to `packages/ui/src/motion.ts`), every `backdrop-filter`/`blur()`, the button glow, accent wash/tint fills, red unread dots, red counts, red stars, the red tab pill, red links, tinted avatars and message cards. The only decoration left is the horizon (a 1px dawn line with a 64px band, `.ak-horizon*`) on sign-in and the once-a-day inbox-zero dawn (`.ak-zero__dawn`, 800ms `cine`, final state under reduced motion). `test/styles.test.ts` (web and ui) now fail on blur, orbs, accent tints, and radial gradients outside the two horizon bands.

### D-481 Tokens: First Light additions; the decoration tokens are deprecated, not deleted (**SPEC §2.12; D-431 accepted**)
- **Decision**: `packages/tokens/tokens.json` adds `color.{light,dark}.dawn` (#C1272D both) and `onAccent` (#FFFFFF) — `PALETTE_KEYS`, the TS `Palette`/`ColorName` and the Rust `Palette` struct (`dawn`, `on_accent`) grow with them —, `opacity.foreground.{tertiary 0.7, surfaceStrong 0.07, surface 0.04}`, `text.app.section` (Geist 12/1.4/500), `motion.duration.quick` 140, `motion.easing.cine` [0.16, 1, 0.3, 1], `layout.{sidebarWidth 216, toolbarHeight 44, statusBarHeight 28, rowHeightTouch 64}` and `effect.{popOpacityLight 0.14, popOpacityDark 0.9}`. Changed: `text.app.label` is Geist Mono (the mono-caps crumb), `bodyStrong` weight 600 → 500, `subject` 24 → 30 from 768px, line 1.15, tracking −0.01. The generator now also emits every `effect` value as a unitless `--effect-*` variable (the level-1 shadow uses them, no colour literals) and zeroes every duration, not a fixed list, under reduced motion. **Deprecated** (listed in the new root `$deprecated` block, validated to exist, and named in the headers of `tokens.css`, `tokens.ts` and `generated.rs`): `motion.orb.*`, `effect.glowBlur`, `effect.glowOpacity`, `effect.cardBlur`, `opacity.accent.{core, midStrong, mid, glowStrong, glow, tint}`. They stay until desktop and mobile stop using them (both still do today), then go in one change.

### D-482 Type scopes: serif once per screen, mono caps in three places, sentence-case sections (**SPEC §2.1**)
- **Decision**: Instrument Serif only for the thread subject, empty states, the sign-in/onboarding headline, the settings page title and the billing plan; never in rows, menus, pickers, dialogs (titles are Geist 15/500), toasts or AI output. Mono caps (`.ak-crumb`, `Crumb`) only for the reader crumb, the AI provenance line and label/triage chips (`.ak-badge--label`, 0.04em). `Eyebrow` is now the section style (12/500, sentence case, tertiary); every former caps label (sidebar groups, form labels, palette and picker headings, compose labels, settings headers) is sentence case.

### D-483 Buttons: `primary` is ink, `brand` is the accent, in exactly three places (**SPEC §2.3–§2.4**)
- **Decision**: `Button` variants are flat pills: `primary`/`ink` (foreground fill, background text, inverse keycaps), `brand` (accent fill, `onAccent` text: sign-in "Email me a code"/"Sign in", onboarding "Connect", the billing trial/subscribe buttons), `secondary` (lineStrong hairline), `ghost` (tertiary text), `danger` (secondary pill with an accentText label). Existing `variant="primary"` call sites therefore became ink without edits. `var(--accent)` appears only in `::selection` and `.ak-button--brand`. Dawn (#C1272D, both themes) marks only the logo dot, the list cursor bar, carets, the horizon and the recording dot.

### D-484 Focus is a 2px ring in the text colour (**SPEC §2.8; supersedes D-399/D-409's accent focus colour**)
- **Decision**: `--ak-focus-color: var(--foreground)`; `:focus-visible` is `outline: 2px solid` 2px off the element (≥17:1 on the page in both themes). Inputs rest on lineStrong, go to the stroke on hover and to a 1px ink border when focused, with a dawn caret; no outer ring. The list never rings itself: the 2px dawn cursor bar on the active row is its focus, and drops to the stroke grey when another pane has the keys. The D-409 tests in `packages/ui/test/styles.test.ts` and `apps/web/test/styles.test.ts` now assert the foreground focus colour and that no focus indicator is red (the cursor bar excepted).

### D-485 The web shell: 216 sidebar, 44px top bar, 28px status bar; one ink toast bottom-left (**SPEC §3.1, §3.9**)
- **Decision**: grid `216 | minmax(0,1fr) | clamp(420px, 38vw, 720px)` × `44 | 1fr | 28`; two panes with the thread as a sheet from 769 to 1099px; one column with a drawer at 768 and below. Sidebar: the "All inboxes ⌄" account menu (accounts, Add account, Settings, theme, Sign out — the four bare icon buttons moved there), nav rows with plain mono keys and counts and no icons, sentence-case groups, hairline label marks, mono account tiles, and a foot of key hints (Commands, Shortcuts, Settings — Settings kept as a visible button so the phone drawer still has it). Top bar: split tabs with an ink underline (the mailbox name as the one tab elsewhere), the bulk bar or search summary in their place, the thread's icon actions, the search field resting as "Search /" and taking the whole bar when focused (so F6 still reaches it), "Compose C". Status bar (`StatusBar.tsx`): where you are, a four-second echo of the last action (`statusEcho.ts`, also the inbox-zero tally), the last three keys, sync state. Toasts: an ink pill bottom-left of the list pane, one info toast at a time and at most two errors above it (`ToastProvider singleInfo`), 4s for info, a hairline running out along undo toasts; the "Mailbox connected" toast became a status echo.

### D-486 The row and the reader copy the landing (**SPEC §3.1.4–§3.1.5**)
- **Decision**: rows are `16 | 168 | 1fr | auto | 72` grids (sender 120 below 1100px, two lines at phone width), ink unread dot, mono times, at most two chips then "+N", the cursor row shows `[E][H][R]` in place of the time, and a hovered row shows Archive/Snooze/Label in the time's place. Those hover actions are pointer-only `aria-hidden` spans (the keys do the same), so options keep one accessible name and no nested interactive content. Reader: mono caps crumb, serif subject with chips inline, a from-line, messages without cards or avatars (40px collapsed rows, hairlines between), one-line image notice, a sticky 40px reply box. HTML bodies keep the light paper inset only where it shows (dark theme, coloured mail); otherwise the frame's text lines up with the reader at 15/1.6 secondary with ink, underlined links. Message times are compact (`messageWhen`).

### D-487 Compose takes the reader slot; rewrite and More are popovers (**SPEC §3.4**)
- **Decision**: new mail docks into the reading pane's rectangle (a right-hand sheet when no pane is on screen); replies, reply-all and forwards expand at the foot of the thread, full reader width (`useReplyDock` now docks every mode above 768px). Anatomy: a 13/500 title with Draft saved and `[Esc]`, 36px hairline rows with a 56px sentence-case label column, Cc/Bcc as quiet text on the To row, the body at 15/1.6 with a dawn caret, then an ink Send, Send later, Write with AI and a chevron that opens the rewrite/dictate popover; follow-up, read receipt, snippets and discard moved into More. The Write-with-AI prompt is a 36px row above the footer. The full-window form (Ctrl+Shift+F) is not built yet.

### D-488 Pickers are anchored popovers (**SPEC §3.7**)
- **Decision**: `Dialog` takes `anchor` (+ `width`): no scrim, placed under the row it acts on (the hovered row, the cursor row, or the composer's Send later button) and flipped above near the bottom edge (`popoverPosition`). It stays a Radix modal dialog, so focus trapping, Escape and focus return are unchanged and the pickers keep their dialog names. Snooze/send later show the live parse and preset times in mono with number keys; labels show a check column and the hairline mark, with "Create label" last. The month grid ("Pick a date…") is not built yet; typed dates cover it.

### D-489 Settings: the Linear pattern; AI provenance lines (**SPEC §3.8, §3.10**)
- **Decision**: settings pages have one serif title (SettingsApp; a section with the page's own name shows only its description, via `SettingsPageTitle`), 13/500 group headers, and consecutive rows drawn as one hairline-bordered group with radius lg (pure CSS); card lists became ruled rows in one group. Controls: ink 28×16 switch, segmented on a surface track, 28px selects, meters in ink. AI: the summary block (hairline, mono caps "SUMMARY · <MODEL> · ON THIS DEVICE"/"AKATSUKI · NOT STORED"/"LEAVES THIS DEVICE · <PROVIDER>" from the new `useAi().provenance`, Hide, Geist 13/20, dawn streaming caret) sits under the from-line (the AI footer's children join the thread column with `display: contents` and `order`); suggested replies are secondary pills picked with 1–3; errors read "Couldn't … · Retry". The full egress sentence stays visible beside the AI actions. Provider rows use the model-picker anatomy; "No provider yet" is one plain row with a secondary button.

## Desktop feature completion (`apps/desktop`, 2026-10-20)

### D-440 Desktop client bindings for the rest of §4.2; no Convex change (**§4.2, §7.2; D-092, D-310 extension**)
- **Decision**: `core::convex::client` and `ClientApi` gain `me.get` (`MeView`: profile, prefs, entitlement; billing is not decoded), `me.updatePrefs` (`PrefsPatch`, every field optional, absent fields omitted), `labels.create/rename/remove`, `snippets.list/save/remove/markUsed` (`SnippetView`, new `SnippetId`), `ops.retry`, `search.threads` (the parsed `ast` is not decoded), `sends.list/reschedule/resolveUnknown` and `drafts.discard`. The desktop's worker client gains `GET /v1/search` (query built with `url`, since reqwest 0.13's `query` feature is off in the workspace). Every name is checked by the existing `every_bound_function_is_exported_by_convex` test, and a mock-transport test pins the argument shapes. `convex/` is unchanged: every function already existed.
- **Why**: one typed place for every call (D-092), and the backend contract the web already uses.

### D-441 New shared key-map entries are palette commands without default keys (**BRIEF keyboard-first; D-289, D-409**)
- **Context**: the desktop needed ids for actions the key map did not have: retry a failed action, create/rename/delete a label, show scheduled messages, continue a draft, find action items, rewrite, dictate, search the full mailbox, search with AI. Every free combo collides with something: browser and devtools keys on the web (Ctrl+Shift+R/J/E/M), AltGr layouts for Ctrl+Alt, typed characters for Alt on macOS, IBus for Ctrl+Shift+U.
- **Decision**: `packages/shortcuts/shortcuts.json` gains `ops.retry`, `labels.create`, `labels.rename`, `labels.remove`, `sends.scheduled`, `compose.continueDraft`, `ai.extractActions`, `compose.rewrite`, `compose.dictate`, `search.fullMailbox` and `search.ai`, all `palette: true` with `keys.default: []` (like `thread.openOriginal`), regenerated into `crates/shortcuts`. The desktop binds the existing keys `compose.sendLater` (Ctrl/⌘+Shift+L), `compose.snippet` (Ctrl/⌘+;) and `compose.aiWrite` (Ctrl/⌘+J), which were palette-hidden in the alpha. Web and mobile are unaffected: the web palette lists only ids it has handlers for, the help sheets skip key-less entries, and mobile derives its unsupported set from the map. A key can be assigned later in one place.
- **Why**: one id, label, group and keyword list per action across clients, without shipping keys that break a browser or a keyboard layout.

### D-442 Desktop settings sync to `prefs` with device-pending fields (**§4.2 `me.updatePrefs`; replaces the "not yet synced" part of D-314; D-400 rule**)
- **Decision**: theme, density, key map, the undo-send window and "keep messages on this device" are synced; motion and endpoints stay per device. Every change on the device marks the field pending in `desktop.json` (`pendingPrefs`) and sends only the changed fields with `me.updatePrefs`; the pending mark clears when the server confirms the value still shown. On sign-in and every minute `me.get` is read and the server wins for every field that is not pending, so a choice made offline or just before a restart survives. Dates, snippets' `{{day}}` and the search grammar use `profiles.timezone` once `me.get` arrives (§1.3).

### D-443 The desktop compose store is the mirror's `local_drafts` table (**§10.1 step 4, §16.1; D-281**)
- **Decision**: `crates/core::store` gains `local_drafts(client_draft_id, doc, updated_at)` (a new `CREATE TABLE IF NOT EXISTS`, no schema-version bump, wiped on sign-out, untouched by a feed reset). The open draft is written there on open, on every autosave tick and when the composer closes; a send deletes it once it can no longer be undone; Discard deletes it and, when an autosave had created a provider draft, calls `drafts.discard`. Closing an empty composer leaves nothing behind. "Continue a draft" (palette) reopens any of them after a restart; uploads that were in flight come back as "Attach it again" (uploaded ones stay valid for 24 h, D-416). A `409 draft_version_conflict` pauses autosave and offers "Keep this one" (save over the other copy with the reported version); "Load that copy" (adopt) is not built on the desktop yet.

### D-444 Send later and scheduled messages on the desktop (**§10 "Send later", §10.3; D-283, D-297**)
- **Decision**: Ctrl/⌘+Shift+L opens a picker with the presets and a typed time ("fri 3pm", "in 2h", "nov 3 at 9"), parsed by `vm::when`, the Rust twin of the web's `parseSnoozeInput` (also used by snooze and reminders now). `POST /v1/send` carries `sendAt` and no undo window; the toast "Scheduled for …" offers Undo (`sends.cancel`) for 10 s. A later send must be at least 30 s away. "Show scheduled messages" lists `sends.list` with Change time (`sends.reschedule`), Cancel the send (the draft stays in Drafts), and for `unknown` sends "It was sent" / "It was not sent" (`sends.resolveUnknown`).

### D-445 Dictation records through the system's recorder, not a linked audio library (**§12.1 `dictation`, §8 limits, §12.7**)
- **Context**: native capture needs `cpal`, which needs ALSA headers to build on Linux (the CI desktop job installs none) and CoreAudio bindgen on macOS, which cannot be built or tested from this machine.
- **Decision**: `apps/desktop/src/dictation.rs` runs the first recorder found on `PATH` (`pw-record`, `parecord`, `arecord`, then `ffmpeg` with PulseAudio; macOS: `ffmpeg` with AVFoundation) to record 16 kHz mono 16-bit WAV into `<data dir>/dictation/` (file created 0600 before recording), stops it with SIGINT through `kill(1)` (the workspace forbids `unsafe`), repairs the WAV sizes, deletes the file and transcribes it with `akatsuki_ai::dictate` on the user's route (managed apps/api → OpenAI, BYO OpenAI, or a local OpenAI-compatible Whisper server). Recording starts only after consent, a resolved dictation route, a key on this device (BYO OpenAI) or the Pro entitlement (managed), and stops at 10 minutes. While recording and transcribing, the composer shows the route's egress sentence ("Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring" for managed). Windows has no recorder yet; without one the composer says what to install.
- **Why**: no new native build dependency, and the audio path is short and visible.

### D-446 AI in the desktop composer, action items and AI search (**§12.1 `compose`, `rewrite`, `extract_actions`, `nl_search`; §16.2 AI cache**)
- **Decision**: Ctrl/⌘+J opens an instruction row; `compose` gets the instruction, the text above the quoted reply or forward (`vm::ai::editable_region`, `crates/mime::quote`), and for replies the thread transcript; the answer streams into that region and the quote stays. Rewrite (picker or "Rewrite: …" rows) works on the selection, else the same region. Escape stops a running request and restores the body; "Undo the AI change" restores the text before the last finished one. "Find action items" lists `extract_actions` with "Remind me", which sets an `always` reminder with the item's title as the note on its due date at 08:00 (tomorrow without one). "Search with AI" turns a sentence into a grammar query (only the sentence and label and split names are sent) that lands in the search field for editing; Enter runs it. Summaries and action items are cached on the device (`ai_cache`, key: feature, SHA-256 of the vars, prompt version, profile and model). Every surface shows its egress line.

### D-447 BYO auto-triage on the desktop (**§12.5 "desktop always"; D-293**)
- **Decision**: while signed in, the desktop checks every 60 s (up to four batches in a row when there is a backlog) whether it should triage: `consent.grantedAt` and `consent.autoTriage` are set, the triage route resolves to a BYO or local profile (never the managed one, which the worker runs), and the profile's key is on this device (local profiles need none). It claims up to 25 conversations with `ai.claimTriage`, sends their headers and snippets only (`render_triage_items`) to the provider from the device, maps the keyed answer back (unknown keys, categories and duplicates dropped) and writes `ai.setTriage`. A failed batch is left to its 2-minute lease. Settings → AI providers gains the "Triage new mail" switch (`consent.autoTriage`); list rows show the category and "Reply".

### D-448 Op failures on the desktop: Retry and Dismiss (**§5.7**)
- **Decision**: `ops.failures` is kept as a list instead of being acknowledged at once. Each failure is a toast, shown once per op id, "Couldn't update 'Subject' in me@…: <reason>", with Retry (`ops.retry`; not offered for sends, which go through Show scheduled messages) and Dismiss (`ops.ackFailure`), for 10 s. "Retry failed action" in the palette retries the newest while any wait. The demo's "Staging is green again" fails at its "provider" the first time it changes, so Retry can be seen and is tested.

### D-449 Desktop search beyond the local index (**§11.2**)
- **Decision**: a search shows the local FTS5 hits at once, then runs `search.threads` (nowAt rounded to the minute) and appends its hits by thread id. When Convex reports `unsupported` grammar, the status line under the title says so and offers "Search the full mailbox" (also a palette command), which runs `GET /v1/search` per account whose provider searches (up to 3 pages each); the worker upserts the hits into Convex, they reach the mirror through the feed, and the list shows them as they land.

### D-450 Labels and snippets on the desktop (**§4.2 `labels.*`, `snippets.*`; D-417, D-295**)
- **Labels**: the label picker (`l`) offers "Create label “…”" when the typed name matches no label of the threads' account (created with `labels.create`, then added to the threads; the server orders the thread op after the label op, §5.4). "Create label", "Rename label" and "Delete label" are palette commands with pickers, and Settings → Labels lists each account's user labels with Rename (inline) and Delete (second click confirms). Folders still come from the provider.
- **Snippets**: Ctrl/⌘+; opens a picker ordered by use, and `;shortcut` then Tab expands in the body. `{{first_name}}`, `{{name}}`, `{{email}}`, `{{day}}`, `{{date}}` are filled (zone: the profile's); others stay and a toast says which to fill; a snippet subject fills an empty subject; `snippets.markUsed` counts the use. Settings → Snippets creates, edits and deletes them; bodies are stored as the web's simple HTML (`text_to_html`) and edited as text (`html_to_text`).

### D-451 Demo and smoke-run additions (**D-316, D-314**)
- **Decision**: the demo serves the new functions: prefs, three invented snippets, label create/rename/delete through the feed (labels now carry their own revision), a flaky thread for Retry, scheduled sends that fire at their time, BYO triage claims, Convex-style header search that reports `unsupported` fields, and a provider search over whole messages. `AKATSUKI_SCRIPT` gains `to=`, `subject=`, `body=`, `instruction=`, `query=` (palette or picker text), `pick=<row value>` and `wait=<ms>`.

### D-452 The desktop wears "First Light" (**design spec §5, §6 desktop checklist; BRIEF design theme; D-321, D-409 superseded for the desktop**)
- **Decision**: `theme.rs` gains the spec's roles from the tokens (`surface` 4 %, `surfaceStrong` 7 %, `line`, `lineStrong`, `stroke`, `tertiary`, `dawn`, `onAccent`) and a two-shadow level-1 `popover()`; `glow()`, `orb()`, the accent tint/wash/glow fields and the hair-space `tracked()` hack are gone, and so are the orbs (sign-in, scrim, empty states). Red is only the cursor bar, carets, the streaming caret, the recording dot, error text (`accentText`), the accent fill of the sign-in buttons, the horizon and text selection (dawn at 22 % behind ink, since GPUI cannot recolour selected glyphs). Unread dots, counts, stars, tabs, links and switches are ink. Instrument Serif is used only for the thread subject (30), empty states, the sign-in headline and the settings page title.
- **Shell**: sidebar 216 on the `surface` plane (account switcher, mailboxes with their keys as mono text and counts, sentence-case "Splits", "Labels", "Accounts", letter tiles, footer key hints; no logo, no Compose button, no folder icons); a 44 px top bar (ink-underline split tabs or the mailbox name, the bulk-selection bar in place of the tabs, "Search /" and "Compose C" hints; `/` turns it into the search field with the provider-search link); a 28 px status bar (list summary or a 4 s echo of the last action, a key HUD of the last three keys, the sync state). From 1100 px the list and the reader sit side by side (reader `clamp(420, 38 %, 720)`), and the composer takes the reader slot; narrower windows keep the reader replacing the list and a pop-out composer.
- **List**: fixed tracks `16 | 168 | 1fr | auto | 72`, hairline rows, ink dot and 500 sender for unread, a check for picked rows, the cursor as `surfaceStrong` plus an absolute 2 px dawn bar (grey while the composer or an overlay owns the keys), key hints `[E][H][R]` in place of the time on the cursor row, hover actions (archive, snooze, label) through `group_hover`, mono caps chips for triage, pending rows at 60 % opacity; quiet empty states and inbox zero with the horizon rising once (still under reduced motion).
- **Reader**: mono-caps crumb, serif subject with chips inline, a from line, the AI block (hairline, radius lg, provenance line `SUMMARY · MODEL · ON THIS DEVICE` from the egress sentence, sans 13/20 text, a 1 px dawn caret while streaming, "Hide"), messages without cards (collapsed 40 px rows, hairlines between), one tertiary notice line ("1 image hidden · 1 tracker removed · Show images in the original"), and the 40 px reply box with its keys.
- **Overlays**: a 560 px palette with a 52 px input, 40 px rows, section-style groups and a footer of keycaps; pickers are 280/320/400 px popovers without a scrim, under the cursor row (snooze, remind, label, move) or above the composer footer (send later, snippets, rewrite), other pickers centred; keycaps are separate kbds, never "then"; toasts are one ink pill bottom-left of the list (errors with a circled "!" stack above, at most two); help is an 880 px sheet of keycaps. Overlays fade in for 140 ms; reduced motion removes it.
- **Sign-in and settings**: no card; the 12 px dot top-left, "Mail at the speed of thought" with a dawn-dot period, a 360 px form with accent pills only there, the horizon drawn from the centre once at about 72 % height and the footer below it. Settings has "Back to mail Esc", a serif page title and description, and hairline groups of 52 px rows with segmented controls (paper on a surface track), ink switches and destructive labels in `accentText`.
- **macOS**: the title bar is transparent with the traffic lights at (16, 16); the account switcher starts clear of them. Untested here (no macOS host was used).
- **Why**: the spec, in its priority order; wherever it is silent the landing demo's `inbox.css` decided.

### D-453 GPUI limits and how the desktop meets the spec anyway (**design spec §5.2**)
- **Decision**: no letter-spacing, so mono caps carry no extra tracking; two-stop gradients only, so the horizon is two half-width 1 px lines fading from transparent dawn (fading from transparent black looked grey) and has no band; no inset shadows, so the cursor bar is an absolute child; popover anchoring is computed from the cursor row (`toolbar + (cursor + 1) × row height + list scroll offset`, clamped to the window) instead of `anchored()`, which needs element bounds the uniform list does not expose; the AI suggested replies and picker presets show no digit keycaps because no digit keys are bound (the spec's `[1]`–`[9]` are left for a later key-map change). The focus ring of buttons and rows (2 px ink, keyboard focus only) is not drawn yet: GPUI has no `focus-visible`, and the cursor bar and field borders carry focus today.

### D-454 Headless screenshots of the desktop (**DEV-RUST "Screens without a keyboard"**)
- **Decision**: design and smoke checks run the app in a private headless sway (`WLR_BACKENDS=headless`, GPU renderer; pixman cannot create GPUI's Vulkan surface) with its own short `XDG_RUNTIME_DIR` (the Wayland socket path must stay under 108 bytes), `WAYLAND_DISPLAY`, `DISPLAY` and `DBUS_SESSION_BUS_ADDRESS` unset (so neither the real compositor nor the real Secret Service is touched), and `grim` against that socket. `AKATSUKI_SCRIPT` drives the states (D-451), and a local OpenAI-compatible mock serves the AI surfaces. Nothing opens on the user's screen.

### D-134 Default keys for the D-441 commands (owner, 2026-10-20; **amends D-441**)
- **Context**: D-441 left eleven new commands palette-only because every free modifier combo collides with something. The owner asked for keys anyway.
- **Decision**: outside text fields, single keys and sequences, which no browser or layout claims: `.` retry the last failed action, `Shift+L` create a label, `G W` scheduled messages, `Shift+C` continue a draft, `Shift+O` open original, `I` show images, `Shift+A` find action items. Inside the composer and search, which need a modifier: rewrite `Ctrl/⌘+Shift+Y`, dictate `Ctrl/⌘+Shift+D`, search the full mailbox `Ctrl/⌘+Shift+F`, search with AI `Ctrl/⌘+/`. None of these is a browser-reserved or devtools key; the page's handler wins over the browser's bookmark, find and download defaults it replaces. `labels.rename` and `labels.remove` stay palette-only: they act on a whole label and are rare, and delete must never be one stray key away. The shortcuts test keeps the map conflict-free on every platform and keymap; a client only binds ids it has a handler for.
