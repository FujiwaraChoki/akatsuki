// worker.* (ARCHITECTURE §4.3, §5.4, §9, §10): service functions for the Rust sync worker, behind
// workerQuery/workerMutation (iss === WORKER_ISSUER). Every function taking an accountId also takes
// workerId; mutations other than claimAccount throw lease_lost unless the worker holds the lease.
import { type Infer, v } from "convex/values";
import { internal } from "./_generated/api";
import type { Doc, Id } from "./_generated/dataModel";
import type { MutationCtx } from "./_generated/server";
import { scheduleReauthReminder } from "./accounts";
import { TRIAGE_LEASE_MS, triageClaimable, triageInputOf, triageRouteKind } from "./ai";
import { loadEntitlement, workerMutation, workerQuery } from "./lib/auth";
import { appError, appErrorValue } from "./lib/errors";
import { parseLabelKey, type ThreadActionLike } from "./lib/model";
import { tombstone } from "./lib/revs";
import { project, vFolderCursor, vTriageInput } from "./lib/rows";
import { shardCount } from "./lib/shards";
import { isSysRole, recomputeThread } from "./lib/threads";
import {
  vAddr,
  vAppError,
  vEntitlement,
  vLabelKey,
  vOpPayload,
  vPrefs,
  vProvider,
  vTriageCategory,
} from "./lib/validators";
import {
  applyCursor,
  type CursorUpdate,
  requireLease,
  upsertLabelsImpl,
  upsertThreadsImpl,
} from "./lib/workerSync";
import schema from "./schema";
import { fireSend } from "./sends";

const DAY = 24 * 60 * 60 * 1000;
const MAX_LEASE_MS = 120_000;

// ───────────── validators ─────────────

const accountFields = schema.tables.mailAccounts.validator.fields;
const vCaps = accountFields.caps;
const vImap = accountFields.imap; // optional object
const vSecretKind = v.union(v.literal("oauth"), v.literal("imap"));
const vSyncState = schema.tables.syncState.validator.extend({
  _id: v.id("syncState"),
  _creationTime: v.number(),
});

const vRemoteLabelIn = v.object({
  providerLabelId: v.string(),
  name: v.string(),
  kind: schema.tables.labels.validator.fields.kind,
  role: schema.tables.labels.validator.fields.role,
  exclusive: v.boolean(),
  color: v.optional(v.string()),
  hidden: v.boolean(),
});

const vImapLocation = v.object({ remoteFolderId: v.string(), uidValidity: v.number(), uid: v.number() });
const vListUnsubscribe = v.object({
  http: v.optional(v.string()),
  mailto: v.optional(v.string()),
  oneClick: v.boolean(),
});

const vMessageUpsert = v.object({
  providerMessageId: v.string(),
  rfcMessageId: v.optional(v.string()),
  inReplyTo: v.optional(v.string()),
  references: v.array(v.string()),
  from: v.optional(vAddr),
  to: v.array(vAddr),
  cc: v.array(vAddr),
  bcc: v.array(vAddr),
  replyTo: v.array(vAddr),
  subject: v.string(),
  snippet: v.string(),
  date: v.number(),
  labelKeys: v.array(v.string()),
  isUnread: v.boolean(),
  isDraft: v.boolean(),
  isOutbound: v.boolean(),
  hasAttachments: v.boolean(),
  sizeBytes: v.optional(v.number()),
  listId: v.optional(v.string()),
  listUnsubscribe: v.optional(vListUnsubscribe),
  authSummary: v.optional(v.object({ spf: v.string(), dkim: v.string(), dmarc: v.string() })),
  imapLocations: v.optional(v.array(vImapLocation)),
});

const vThreadUpsert = v.object({
  providerThreadId: v.string(),
  providerVersion: v.optional(v.int64()),
  complete: v.boolean(),
  messages: v.array(vMessageUpsert),
  removedProviderMessageIds: v.optional(v.array(v.string())),
});

const vSyncPhase = accountFields.syncPhase;
const vCursorUpdate = v.union(
  v.object({ kind: v.literal("gmail"), historyId: v.string() }),
  v.object({
    kind: v.literal("folder"),
    remoteFolderId: v.string(),
    labelKey: v.string(),
    uidValidity: v.optional(v.number()),
    uidNext: v.optional(v.number()),
    highestModSeq: v.optional(v.string()),
    deltaLink: v.optional(v.string()),
  }),
  v.object({ kind: v.literal("backfill"), beforeAt: v.number() }),
  v.object({ kind: v.literal("phase"), syncPhase: vSyncPhase }),
);

const vOpResult = v.union(
  v.object({
    opId: v.id("ops"),
    ok: v.literal(true),
    outcome: v.union(v.literal("applied"), v.literal("moot")),
    providerLabelId: v.optional(v.string()),
    sentProviderMessageId: v.optional(v.string()),
  }),
  v.object({
    opId: v.id("ops"),
    ok: v.literal(false),
    error: vAppError,
    retryAfterMs: v.optional(v.number()),
    countAttempt: v.boolean(),
  }),
);

const vClaimedOp = v.object({
  op: v.object({
    _id: v.id("ops"),
    clientOpId: v.string(),
    payload: vOpPayload,
    attempt: v.number(),
    verifyOnly: v.boolean(),
    sendElapsedMs: v.optional(v.number()),
  }),
  thread: v.optional(
    v.object({
      providerThreadId: v.string(),
      syncedLabelKeys: v.array(vLabelKey),
      messages: v.array(
        v.object({
          providerMessageId: v.string(),
          labelKeys: v.array(vLabelKey),
          imapLocations: v.optional(v.array(vImapLocation)),
        }),
      ),
    }),
  ),
  labels: v.record(
    v.string(),
    v.object({
      providerLabelId: v.optional(v.string()),
      name: v.string(),
      exclusive: v.boolean(),
      role: v.optional(v.string()),
    }),
  ),
  send: v.optional(
    v.object({
      draft: v.object({
        providerDraftId: v.string(),
        providerMessageId: v.optional(v.string()),
        rfcMessageId: v.string(),
      }),
      kind: v.union(v.literal("undo"), v.literal("later")),
      sendAt: v.number(),
    }),
  ),
  // Not in §5.4: delete_draft ops need the provider draft to delete (recorded in docs/DECISIONS.md).
  draft: v.optional(
    v.object({ providerDraftId: v.optional(v.string()), providerMessageId: v.optional(v.string()) }),
  ),
});

const vTriageResult = v.object({
  threadId: v.id("threads"),
  category: vTriageCategory,
  needsReply: v.boolean(),
  model: v.string(),
  promptVersion: v.number(),
});

/** Stable shard for a mailbox (FNV-1a). SHARD_COUNT=1 in v1, so this is 0 (D-044). */
function shardFor(key: string, shardCount: number): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < key.length; i++) {
    h ^= key.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h % shardCount;
}

function checkLeaseMs(leaseMs: number) {
  if (!(leaseMs > 0 && leaseMs <= MAX_LEASE_MS))
    throw appError("invalid_argument", "leaseMs is 1 to 120000.");
}

async function syncStateOf(ctx: Pick<MutationCtx, "db">, accountId: Id<"mailAccounts">) {
  return await ctx.db
    .query("syncState")
    .withIndex("by_account", (q) => q.eq("accountId", accountId))
    .unique();
}

// ───────────── accounts and leases ─────────────

export const assignedAccounts = workerQuery({
  args: { shard: v.number() },
  returns: v.array(
    v.object({
      accountId: v.id("mailAccounts"),
      userId: v.string(),
      provider: vProvider,
      providerAccountId: v.string(),
      email: v.string(),
      status: accountFields.status,
      entitled: v.boolean(),
      serverTriage: v.boolean(),
      imap: vImap,
      caps: vCaps,
    }),
  ),
  handler: async (ctx, { shard }) => {
    const rows: Doc<"mailAccounts">[] = [];
    for (const status of accountFields.status.members.map((m) => m.value)) {
      rows.push(
        ...(await ctx.db
          .query("mailAccounts")
          .withIndex("by_shard_status", (q) => q.eq("shard", shard).eq("status", status))
          .take(5000)),
      );
    }
    const out = [];
    const perUser = new Map<string, { entitled: boolean; serverTriage: boolean }>();
    for (const a of rows) {
      let facts = perUser.get(a.userId);
      if (!facts) {
        const ent = await loadEntitlement(ctx, a.userId);
        const ai = await ctx.db
          .query("aiSettings")
          .withIndex("by_user", (q) => q.eq("userId", a.userId))
          .unique();
        facts = {
          entitled: ent.hostedSync,
          serverTriage: ent.managedAi && triageRouteKind(ai) === "akatsuki",
        };
        perUser.set(a.userId, facts);
      }
      out.push({
        accountId: a._id,
        userId: a.userId,
        provider: a.provider,
        providerAccountId: a.providerAccountId,
        email: a.email,
        status: a.status,
        entitled: facts.entitled,
        serverTriage: facts.serverTriage,
        imap: a.imap,
        caps: a.caps,
      });
    }
    return out;
  },
});

export const claimAccount = workerMutation({
  args: { accountId: v.id("mailAccounts"), workerId: v.string(), leaseMs: v.number() },
  returns: v.union(
    v.object({ ok: v.literal(true), syncState: vSyncState, folderCursors: v.array(vFolderCursor) }),
    v.object({ ok: v.literal(false), owner: v.string() }),
  ),
  handler: async (ctx, { accountId, workerId, leaseMs }) => {
    checkLeaseMs(leaseMs);
    const account = await ctx.db.get(accountId);
    if (!account) throw appError("not_found");
    const now = Date.now();
    let state = await syncStateOf(ctx, accountId);
    if (state?.leaseOwner && state.leaseOwner !== workerId && (state.leaseExpiresAt ?? 0) > now) {
      return { ok: false as const, owner: state.leaseOwner };
    }
    if (!state) {
      const id = await ctx.db.insert("syncState", {
        accountId,
        shard: account.shard,
        leaseOwner: workerId,
        leaseExpiresAt: now + leaseMs,
        consecutiveFailures: 0,
        updatedAt: now,
      });
      state = (await ctx.db.get(id))!;
    } else {
      await ctx.db.patch(state._id, { leaseOwner: workerId, leaseExpiresAt: now + leaseMs, updatedAt: now });
      state = (await ctx.db.get(state._id))!;
    }
    const cursors = await ctx.db
      .query("folderCursors")
      .withIndex("by_account_folder", (q) => q.eq("accountId", accountId))
      .collect();
    return {
      ok: true as const,
      syncState: state,
      folderCursors: cursors.map((c) => project(vFolderCursor, c)),
    };
  },
});

export const heartbeat = workerMutation({
  args: { workerId: v.string(), accountIds: v.array(v.id("mailAccounts")), leaseMs: v.number() },
  returns: v.object({ lost: v.array(v.id("mailAccounts")) }),
  handler: async (ctx, { workerId, accountIds, leaseMs }) => {
    checkLeaseMs(leaseMs);
    const now = Date.now();
    const lost: Id<"mailAccounts">[] = [];
    for (const accountId of accountIds) {
      const state = await syncStateOf(ctx, accountId);
      if (!state || state.leaseOwner !== workerId) {
        lost.push(accountId);
        continue;
      }
      await ctx.db.patch(state._id, { leaseExpiresAt: now + leaseMs, updatedAt: now });
    }
    return { lost };
  },
});

export const releaseAccount = workerMutation({
  args: { accountId: v.id("mailAccounts"), workerId: v.string() },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId }) => {
    const { state } = await requireLease(ctx, accountId, workerId);
    await ctx.db.patch(state._id, { leaseExpiresAt: Date.now(), updatedAt: Date.now() });
    return null;
  },
});

// ───────────── secrets (ciphertext only; §14.1) ─────────────

export const getSecret = workerQuery({
  args: { accountId: v.id("mailAccounts"), kind: vSecretKind },
  returns: v.union(
    v.object({ keyId: v.string(), nonce: v.bytes(), ciphertext: v.bytes(), version: v.number() }),
    v.null(),
  ),
  handler: async (ctx, { accountId, kind }) => {
    const row = await ctx.db
      .query("accountSecrets")
      .withIndex("by_account_kind", (q) => q.eq("accountId", accountId).eq("kind", kind))
      .unique();
    return row
      ? { keyId: row.keyId, nonce: row.nonce, ciphertext: row.ciphertext, version: row.version }
      : null;
  },
});

export const putSecret = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    kind: vSecretKind,
    keyId: v.string(),
    nonce: v.bytes(),
    ciphertext: v.bytes(),
    expectedVersion: v.number(),
  },
  returns: v.object({ version: v.number() }),
  handler: async (ctx, { accountId, workerId, kind, keyId, nonce, ciphertext, expectedVersion }) => {
    await requireLease(ctx, accountId, workerId);
    if (nonce.byteLength !== 24) throw appError("invalid_argument", "nonce must be 24 bytes.");
    const row = await ctx.db
      .query("accountSecrets")
      .withIndex("by_account_kind", (q) => q.eq("accountId", accountId).eq("kind", kind))
      .unique();
    const now = Date.now();
    if (!row) {
      if (expectedVersion !== 0) throw appError("version_conflict");
      await ctx.db.insert("accountSecrets", {
        accountId,
        kind,
        keyId,
        nonce,
        ciphertext,
        version: 1,
        updatedAt: now,
      });
      return { version: 1 };
    }
    if (row.version !== expectedVersion) throw appError("version_conflict");
    await ctx.db.patch(row._id, { keyId, nonce, ciphertext, version: row.version + 1, updatedAt: now });
    return { version: row.version + 1 };
  },
});

const vIdentityIn = v.object({
  email: v.string(),
  name: v.optional(v.string()),
  replyTo: v.optional(v.string()),
  isDefault: v.boolean(),
});

async function upsertIdentitiesImpl(
  ctx: MutationCtx,
  account: Doc<"mailAccounts">,
  identities: { email: string; name?: string; replyTo?: string; isDefault: boolean }[],
) {
  const existing = await ctx.db
    .query("identities")
    .withIndex("by_account", (q) => q.eq("accountId", account._id))
    .collect();
  const byEmail = new Map(existing.map((i) => [i.email, i]));
  const seen = new Set<string>();
  for (const i of identities.slice(0, 100)) {
    const email = i.email.trim().toLowerCase();
    seen.add(email);
    const row = byEmail.get(email);
    if (row) {
      // Keeps the user-edited signature and name.
      await ctx.db.patch(row._id, {
        replyTo: row.replyTo ?? i.replyTo,
        isDefault: i.isDefault,
        name: row.name ?? i.name,
      });
    } else {
      await ctx.db.insert("identities", {
        userId: account.userId,
        accountId: account._id,
        email,
        name: i.name,
        replyTo: i.replyTo,
        isDefault: i.isDefault,
      });
    }
  }
  for (const row of existing) if (!seen.has(row.email)) await ctx.db.delete(row._id);
}

export const createAccount = workerMutation({
  args: {
    userId: v.string(),
    provider: vProvider,
    providerAccountId: v.string(),
    email: v.string(),
    displayName: v.optional(v.string()),
    imap: vImap,
    caps: vCaps,
    identities: v.array(vIdentityIn),
    secret: v.object({ kind: vSecretKind, keyId: v.string(), nonce: v.bytes(), ciphertext: v.bytes() }),
    // Set for a reauth (`ConnectStart.accountId`): the grant must be for that same mailbox (D-395).
    reauthAccountId: v.optional(v.id("mailAccounts")),
  },
  returns: v.object({ accountId: v.id("mailAccounts"), reconnected: v.boolean() }),
  handler: async (ctx, args) => {
    const ent = await loadEntitlement(ctx, args.userId);
    if (!ent.hostedSync) throw appError("not_entitled");
    // The worker seals `secret` with an AAD that contains this normalised id (§14.1, D-127; Rust twin:
    // crates/core::convex::worker::normalize_provider_account_id). Changing it orphans every sealed secret.
    const providerAccountId =
      args.provider === "gmail" ? args.providerAccountId.trim().toLowerCase() : args.providerAccountId;
    const email = args.email.trim().toLowerCase();
    const now = Date.now();
    const existing = await ctx.db
      .query("mailAccounts")
      .withIndex("by_provider_account", (q) =>
        q.eq("provider", args.provider).eq("providerAccountId", providerAccountId),
      )
      .first();
    if (existing && existing.userId !== args.userId && existing.status !== "removing") {
      throw appError("already_connected");
    }
    if (args.reauthAccountId !== undefined && existing?._id !== args.reauthAccountId) {
      throw appError("invalid_argument", "Sign in to the mailbox you are reconnecting.");
    }
    let accountId: Id<"mailAccounts">;
    let reconnected = false;
    if (existing && existing.userId === args.userId && existing.status !== "removing") {
      reconnected = true;
      accountId = existing._id;
      await ctx.db.patch(accountId, {
        status: "connecting",
        statusDetail: undefined,
        email,
        displayName: args.displayName ?? existing.displayName,
        imap: args.imap,
        caps: args.caps,
      });
    } else {
      const siblings = await ctx.db
        .query("mailAccounts")
        .withIndex("by_user", (q) => q.eq("userId", args.userId))
        .collect();
      if (siblings.length >= 20) throw appError("invalid_argument", "At most 20 mailboxes per user.");
      accountId = await ctx.db.insert("mailAccounts", {
        userId: args.userId,
        provider: args.provider,
        providerAccountId,
        email,
        displayName: args.displayName,
        sortOrder: siblings.length,
        shard: shardFor(providerAccountId, shardCount()), // fixed at insert
        status: "connecting",
        syncPhase: "baseline",
        progress: { threadsDone: 0 },
        windowStartAt: now - 90 * DAY,
        backfillHorizonAt: now - 365 * DAY,
        caps: args.caps,
        imap: args.imap,
        createdAt: now,
      });
    }
    // The same user reconnecting replaces the secret (a reauth gets a fresh grant).
    const old = await ctx.db
      .query("accountSecrets")
      .withIndex("by_account_kind", (q) => q.eq("accountId", accountId).eq("kind", args.secret.kind))
      .unique();
    if (old) {
      await ctx.db.patch(old._id, { ...args.secret, version: old.version + 1, updatedAt: now });
    } else {
      await ctx.db.insert("accountSecrets", { accountId, ...args.secret, version: 1, updatedAt: now });
    }
    const account = (await ctx.db.get(accountId))!;
    await upsertIdentitiesImpl(ctx, account, args.identities);
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", args.userId))
      .unique();
    if (profile && profile.onboarding !== "done")
      await ctx.db.patch(profile._id, { onboarding: "done", updatedAt: now });
    return { accountId, reconnected };
  },
});

export const setAccountStatus = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    status: v.optional(accountFields.status),
    statusDetail: v.optional(v.union(v.string(), v.null())),
    syncPhase: v.optional(vSyncPhase),
    progress: v.optional(accountFields.progress),
    oldestSyncedAt: v.optional(v.number()),
    lastSyncAt: v.optional(v.number()),
  },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId, statusDetail, ...rest }) => {
    const { account } = await requireLease(ctx, accountId, workerId);
    if (statusDetail !== undefined && statusDetail !== null && !/^[a-z_]{1,64}$/.test(statusDetail)) {
      throw appError("invalid_argument", "statusDetail must be an error code.");
    }
    const patch: Partial<Doc<"mailAccounts">> = {};
    for (const [k, val] of Object.entries(rest))
      if (val !== undefined) (patch as Record<string, unknown>)[k] = val;
    if (statusDetail !== undefined) patch.statusDetail = statusDetail ?? undefined;
    if (account.status === "removing" && patch.status && patch.status !== "removing") delete patch.status;
    await ctx.db.patch(accountId, patch);
    if (patch.status === "reauth_required" && account.status !== "reauth_required")
      await scheduleReauthReminder(ctx, accountId);
    return null;
  },
});

export const upsertIdentities = workerMutation({
  args: { accountId: v.id("mailAccounts"), workerId: v.string(), identities: v.array(vIdentityIn) },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId, identities }) => {
    const { account } = await requireLease(ctx, accountId, workerId);
    await upsertIdentitiesImpl(ctx, account, identities);
    return null;
  },
});

export const upsertLabels = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    labels: v.array(vRemoteLabelIn),
    complete: v.boolean(),
  },
  returns: v.object({ labelKeyByProviderId: v.record(v.string(), v.string()) }),
  handler: async (ctx, { accountId, workerId, labels, complete }) => {
    const { account } = await requireLease(ctx, accountId, workerId);
    return { labelKeyByProviderId: await upsertLabelsImpl(ctx, account, labels, complete) };
  },
});

export const upsertThreads = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    threads: v.array(vThreadUpsert),
    deletes: v.optional(v.array(v.string())),
    // Provider message ids gone at the provider (IMAP expunges and moves; D-410), ≤ 400.
    deletedMessages: v.optional(v.array(v.string())),
    cursor: v.optional(vCursorUpdate),
  },
  returns: v.object({
    written: v.number(),
    skippedStale: v.number(),
    newInbound: v.array(v.object({ threadId: v.id("threads"), isInbox: v.boolean() })),
  }),
  handler: async (ctx, { accountId, workerId, threads, deletes, deletedMessages, cursor }) => {
    const { account, state } = await requireLease(ctx, accountId, workerId);
    const total = threads.reduce((n, t) => n + t.messages.length, 0);
    if (total > 400) throw appError("invalid_argument", "At most 400 messages per call.");
    const result = await upsertThreadsImpl(ctx, account, threads, deletes ?? [], deletedMessages ?? []);
    if (cursor) await applyCursor(ctx, state, account, cursor as CursorUpdate);
    return result;
  },
});

export const setCursor = workerMutation({
  args: { accountId: v.id("mailAccounts"), workerId: v.string(), cursor: vCursorUpdate },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId, cursor }) => {
    const { account, state } = await requireLease(ctx, accountId, workerId);
    await applyCursor(ctx, state, account, cursor as CursorUpdate);
    return null;
  },
});

export const setWatch = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    watchKind: v.union(
      v.literal("gmail_watch"),
      v.literal("graph_subscription"),
      v.literal("imap_idle"),
      v.literal("poll"),
    ),
    externalId: v.optional(v.string()),
    expiresAt: v.optional(v.number()),
    clientStateHash: v.optional(v.string()),
  },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId, watchKind, externalId, expiresAt, clientStateHash }) => {
    const { state } = await requireLease(ctx, accountId, workerId);
    if (clientStateHash !== undefined && !/^[0-9a-f]{64}$/.test(clientStateHash)) {
      throw appError("invalid_argument", "clientStateHash must be hex sha256.");
    }
    await ctx.db.patch(state._id, {
      watchKind,
      watchExternalId: externalId,
      watchExpiresAt: expiresAt,
      watchClientStateHash: clientStateHash,
      updatedAt: Date.now(),
    });
    return null;
  },
});

export const accountsForPush = workerQuery({
  // Contract shape: { by: "gmail_email", email } | { by: "graph_subscription", subscriptionId }.
  args: {
    by: v.union(v.literal("gmail_email"), v.literal("graph_subscription")),
    email: v.optional(v.string()),
    subscriptionId: v.optional(v.string()),
  },
  returns: v.array(v.object({ accountId: v.id("mailAccounts"), clientStateHash: v.optional(v.string()) })),
  handler: async (ctx, q) => {
    if (q.by === "gmail_email") {
      if (!q.email) throw appError("invalid_argument", "email is required.");
      const email = q.email.trim().toLowerCase();
      const rows = await ctx.db
        .query("mailAccounts")
        .withIndex("by_provider_email", (x) => x.eq("provider", "gmail").eq("email", email))
        .take(10);
      return rows.filter((a) => a.status !== "removing").map((a) => ({ accountId: a._id }));
    }
    if (!q.subscriptionId) throw appError("invalid_argument", "subscriptionId is required.");
    const subscriptionId = q.subscriptionId;
    const rows = await ctx.db
      .query("syncState")
      .withIndex("by_watch_external", (x) => x.eq("watchExternalId", subscriptionId))
      .take(10);
    return rows.map((s) => ({ accountId: s.accountId, clientStateHash: s.watchClientStateHash }));
  },
});

// ───────────── ops (§5.4–5.6) ─────────────

export const pendingOpAccounts = workerQuery({
  args: { shard: v.number() },
  returns: v.array(v.object({ accountId: v.id("mailAccounts"), minAvailableAt: v.number() })),
  handler: async (ctx, { shard }) => {
    const rows = await ctx.db
      .query("ops")
      .withIndex("by_shard_state_available", (q) => q.eq("shard", shard).eq("state", "queued"))
      .take(1000);
    const min = new Map<Id<"mailAccounts">, number>();
    for (const op of rows) {
      const cur = min.get(op.accountId);
      if (cur === undefined || op.availableAt < cur) min.set(op.accountId, op.availableAt);
    }
    return [...min].map(([accountId, minAvailableAt]) => ({ accountId, minAvailableAt }));
  },
});

type LabelInfo = { providerLabelId?: string; name: string; exclusive: boolean; role?: string };

function actionKeys(a: ThreadActionLike): string[] {
  if (a.kind === "modify") return [...a.add, ...a.remove];
  if (a.kind === "move") return [a.to];
  return [];
}

export const claimOps = workerMutation({
  args: { accountId: v.id("mailAccounts"), workerId: v.string(), limit: v.number(), leaseMs: v.number() },
  returns: v.array(vClaimedOp),
  handler: async (ctx, { accountId, workerId, limit, leaseMs }) => {
    checkLeaseMs(leaseMs);
    if (limit < 1 || limit > 100) throw appError("invalid_argument", "limit is 1 to 100.");
    const { account } = await requireLease(ctx, accountId, workerId);
    // Queued ops on an account that needs reauth (or is paused) wait without consuming attempts.
    if (account.status === "reauth_required" || account.status === "paused" || account.status === "removing")
      return [];
    const now = Date.now();

    // Everything already inflight blocks its thread/label (FIFO per thread and per label, D-036).
    const inflight = await ctx.db
      .query("ops")
      .withIndex("by_account_state", (q) => q.eq("accountId", accountId).eq("state", "inflight"))
      .take(500);
    const blockedThreads = new Set<string>(inflight.flatMap((o) => (o.threadId ? [o.threadId] : [])));
    const blockedLabels = new Set<string>(
      inflight.flatMap((o) => (o.labelId && o.payload.kind !== "thread" ? [o.labelId] : [])),
    );

    const queued = await ctx.db
      .query("ops")
      .withIndex("by_account_state", (q) => q.eq("accountId", accountId).eq("state", "queued"))
      .take(500);

    const labelsByKey = new Map<string, Doc<"labels">>();
    const allLabels = await ctx.db
      .query("labels")
      .withIndex("by_account", (q) => q.eq("accountId", accountId))
      .collect();
    for (const l of allLabels) {
      labelsByKey.set(isSysRole(l.role) ? `sys:${l.role}` : `l:${l._id}`, l);
    }
    const info = (l: Doc<"labels">): LabelInfo => ({
      providerLabelId: l.providerLabelId,
      name: l.name,
      exclusive: l.exclusive,
      role: l.role,
    });
    // System/role folders are always useful to adapters (trash, spam, archive, snoozed locations).
    const baseLabels: Record<string, LabelInfo> = {};
    for (const [key, l] of labelsByKey) if (l.role) baseLabels[key] = info(l);

    const claimed: Doc<"ops">[] = [];
    for (const op of queued) {
      if (claimed.length >= limit) break;
      // FIFO per thread is for thread ops only (§5.4); sends and draft ops never wait on them.
      const threadKey = op.payload.kind === "thread" ? op.threadId : undefined;
      const labelKey = op.payload.kind !== "thread" ? op.labelId : undefined;
      const blocked =
        (threadKey !== undefined && blockedThreads.has(threadKey)) ||
        (labelKey !== undefined && blockedLabels.has(labelKey));
      let skip = blocked || op.availableAt > now;
      if (!skip && op.payload.kind === "thread" && op.labelId) {
        const label = await ctx.db.get(op.labelId);
        if (label && label.providerLabelId === undefined) skip = true; // waits for its label_create
      }
      if (skip) {
        if (threadKey) blockedThreads.add(threadKey);
        if (labelKey) blockedLabels.add(labelKey);
        continue;
      }
      claimed.push(op);
    }

    const out = [];
    for (const op of claimed) {
      await ctx.db.patch(op._id, {
        state: "inflight",
        leaseOwner: workerId,
        leaseExpiresAt: now + leaseMs,
        attempt: op.attempt + 1,
      });
      const labels: Record<string, LabelInfo> = { ...baseLabels };
      let thread: Infer<typeof vClaimedOp>["thread"];
      if (op.payload.kind === "thread" && op.threadId) {
        const t = await ctx.db.get(op.threadId);
        if (t) {
          const msgs = await ctx.db
            .query("messages")
            .withIndex("by_thread_date", (q) => q.eq("threadId", t._id))
            .take(500);
          thread = {
            providerThreadId: t.providerThreadId,
            syncedLabelKeys: t.syncedLabelKeys,
            messages: msgs.map((m) => ({
              providerMessageId: m.providerMessageId,
              labelKeys: m.labelKeys,
              imapLocations: m.imapLocations,
            })),
          };
          for (const key of [...actionKeys(op.payload.action as ThreadActionLike), ...t.syncedLabelKeys]) {
            const l = labelsByKey.get(key);
            if (l && parseLabelKey(key)) labels[key] = info(l);
          }
        }
      }
      if (
        op.payload.kind === "label_create" ||
        op.payload.kind === "label_rename" ||
        op.payload.kind === "label_delete"
      ) {
        const l = await ctx.db.get(op.payload.labelId);
        if (l) labels[`l:${l._id}`] = info(l);
      }
      let send: Infer<typeof vClaimedOp>["send"];
      if (op.payload.kind === "send") {
        const s = await ctx.db.get(op.payload.scheduledSendId);
        const d = s?.draftId ? await ctx.db.get(s.draftId) : null;
        if (s && d?.providerDraftId) {
          send = {
            draft: {
              providerDraftId: d.providerDraftId,
              providerMessageId: d.providerMessageId,
              rfcMessageId: d.rfcMessageId,
            },
            kind: s.kind,
            sendAt: s.sendAt,
          };
        }
      }
      let draft: Infer<typeof vClaimedOp>["draft"];
      if (op.payload.kind === "delete_draft") {
        const d = await ctx.db.get(op.payload.draftId);
        if (d) draft = { providerDraftId: d.providerDraftId, providerMessageId: d.providerMessageId };
      }
      out.push({
        op: {
          _id: op._id,
          clientOpId: op.clientOpId,
          payload: op.payload,
          attempt: op.attempt + 1,
          verifyOnly: op.verifyOnly ?? false,
          // Verification of an ambiguous send is timed from the send start, on Convex's clock, so a worker
          // restart or a skewed worker clock keeps the schedule (§5.6).
          sendElapsedMs: op.sendStartedAt !== undefined ? Math.max(0, now - op.sendStartedAt) : undefined,
        },
        thread,
        labels,
        send,
        draft,
      });
    }
    return out;
  },
});

export const markSendStarted = workerMutation({
  args: { opId: v.id("ops"), workerId: v.string() },
  returns: v.null(),
  handler: async (ctx, { opId, workerId }) => {
    const op = await ctx.db.get(opId);
    if (op?.state !== "inflight" || op.leaseOwner !== workerId) throw appError("lease_lost");
    if (op.payload.kind !== "send") throw appError("invalid_argument", "Not a send op.");
    await ctx.db.patch(opId, { sendStartedAt: Date.now() });
    await ctx.db.patch(op.payload.scheduledSendId, { state: "sending" });
    return null;
  },
});

/** min(15 min, 2 s · 2^(attempt−1)) · uniform(0.5, 1.0) (§5.5). */
export function backoffMs(attempt: number, rand = Math.random()): number {
  const base = Math.min(15 * 60_000, 2000 * 2 ** Math.max(0, attempt - 1));
  return Math.round(base * (0.5 + 0.5 * rand));
}

async function finishSend(
  ctx: MutationCtx,
  op: Doc<"ops">,
  state: "sent" | "failed" | "unknown",
  extra: Partial<Doc<"scheduledSends">>,
) {
  if (op.payload.kind !== "send") return;
  const send = await ctx.db.get(op.payload.scheduledSendId);
  if (!send) return;
  await ctx.db.patch(send._id, { state, ...extra });
  if (send.draftId) {
    const draft = await ctx.db.get(send.draftId);
    if (draft)
      await ctx.db.patch(draft._id, { state: state === "sent" ? "sent" : "editing", updatedAt: Date.now() });
  }
}

export const completeOps = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    results: v.array(vOpResult),
    threads: v.optional(v.array(vThreadUpsert)),
    labels: v.optional(v.array(vRemoteLabelIn)),
  },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId, results, threads, labels }) => {
    const { account } = await requireLease(ctx, accountId, workerId);
    if (results.length > 100) throw appError("invalid_argument", "At most 100 results.");
    if (labels && labels.length > 0) await upsertLabelsImpl(ctx, account, labels, false);
    const now = Date.now();
    const touched = new Set<Id<"threads">>();
    for (const r of results) {
      const op = await ctx.db.get(r.opId);
      // Idempotent: results for ops that moved on (reaped, cancelled, already completed) are ignored.
      if (!op || op.accountId !== accountId || op.state !== "inflight" || op.leaseOwner !== workerId)
        continue;
      if (op.threadId) touched.add(op.threadId);
      const clearLease = { leaseOwner: undefined, leaseExpiresAt: undefined };
      if (r.ok) {
        await ctx.db.patch(op._id, {
          state: "done",
          outcome: r.outcome,
          completedAt: now,
          lastError: undefined,
          ...clearLease,
        });
        if (op.payload.kind === "label_create" && r.providerLabelId) {
          await ctx.db.patch(op.payload.labelId, { providerLabelId: r.providerLabelId });
        } else if (op.payload.kind === "label_delete") {
          const label = await ctx.db.get(op.payload.labelId);
          if (label) {
            await ctx.db.delete(label._id);
            await tombstone(ctx, accountId, "labels", label._id);
          }
        } else if (op.payload.kind === "send") {
          await finishSend(ctx, op, "sent", {
            sentProviderMessageId: r.sentProviderMessageId,
            error: undefined,
          });
        } else if (op.payload.kind === "delete_draft") {
          const d = await ctx.db.get(op.payload.draftId);
          if (d && d.state === "discarded")
            await ctx.db.patch(d._id, { providerDraftId: undefined, updatedAt: now });
        }
        continue;
      }
      // Failure (§5.5)
      const attempt = r.countAttempt ? op.attempt : Math.max(0, op.attempt - 1);
      if (r.error.code === "reauth_required" && account.status !== "reauth_required") {
        await ctx.db.patch(accountId, { status: "reauth_required", statusDetail: "reauth_required" });
        await scheduleReauthReminder(ctx, accountId);
        account.status = "reauth_required";
      }
      const exhausted = r.countAttempt && attempt >= op.maxAttempts;
      if (r.error.retryable && !exhausted) {
        await ctx.db.patch(op._id, {
          state: "queued",
          attempt,
          availableAt: now + Math.max(r.retryAfterMs ?? 0, r.countAttempt ? backoffMs(attempt) : 0),
          lastError: r.error,
          ...clearLease,
        });
        continue;
      }
      if (!r.countAttempt && !exhausted) {
        // Auth and rate-limit errors never consume attempts; the op waits.
        await ctx.db.patch(op._id, {
          state: "queued",
          attempt,
          availableAt: now + (r.retryAfterMs ?? backoffMs(1)),
          lastError: r.error,
          ...clearLease,
        });
        continue;
      }
      const final = op.payload.kind === "send" && op.verifyOnly ? "unknown" : "failed";
      const error = final === "unknown" ? appErrorValue("send_unconfirmed") : r.error;
      await ctx.db.patch(op._id, {
        state: final,
        attempt,
        completedAt: now,
        lastError: error,
        ...clearLease,
      });
      if (op.payload.kind === "send") await finishSend(ctx, op, final, { error });
      if (op.payload.kind === "label_create") {
        // Thread ops waiting for this label would wait forever: they fail with it (and roll back).
        const waiting = await ctx.db
          .query("ops")
          .withIndex("by_label_state", (q) => q.eq("labelId", op.labelId).eq("state", "queued"))
          .take(500);
        for (const w of waiting) {
          if (w.payload.kind !== "thread") continue;
          await ctx.db.patch(w._id, { state: "failed", completedAt: now, lastError: error });
          if (w.threadId) touched.add(w.threadId);
        }
      }
    }
    // Post-op provider state (and fresh truth after a permanent failure, D-011) in the same transaction.
    if (threads && threads.length > 0) await upsertThreadsImpl(ctx, account, threads);
    for (const t of touched) await recomputeThread(ctx, t);
    return null;
  },
});

// ───────────── drafts and sends (§10) ─────────────

const RECEIPT_TOKEN = /^[A-Za-z0-9_-]{22}$/;

/**
 * Keeps the draft's `readReceipts` row in step with the composed draft (D-412): the worker passes the token when
 * the MIME it saved carries the pixel. The row is keyed by the draft's fixed `rfcMessageId` (the token is derived
 * from it, so it is stable across autosaves); `upsertThreads` links `messageId` once the sent message arrives.
 * A draft saved without the pixel drops a row that has recorded no opens.
 */
async function syncReadReceipt(
  ctx: MutationCtx,
  userId: string,
  draftId: Id<"drafts">,
  rfcMessageId: string,
  token: string | undefined,
) {
  const row = await ctx.db
    .query("readReceipts")
    .withIndex("by_user_rfc", (q) => q.eq("userId", userId).eq("rfcMessageId", rfcMessageId))
    .first();
  if (token === undefined) {
    if (row && row.opens.length === 0 && row.messageId === undefined) await ctx.db.delete(row._id);
    return;
  }
  if (row) {
    if (row.token !== token || row.draftId !== draftId) await ctx.db.patch(row._id, { token, draftId });
    return;
  }
  await ctx.db.insert("readReceipts", {
    userId,
    token,
    draftId,
    rfcMessageId,
    opens: [],
    createdAt: Date.now(),
  });
}

export const recordDraft = workerMutation({
  args: {
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    clientDraftId: v.string(),
    baseVersion: v.number(),
    providerDraftId: v.string(),
    providerMessageId: v.string(),
    rfcMessageId: v.string(),
    threadId: v.optional(v.id("threads")),
    replyToMessageId: v.optional(v.id("messages")),
    subject: v.string(),
    to: v.array(vAddr),
    cc: v.array(vAddr),
    bcc: v.array(vAddr),
    deviceId: v.optional(v.string()),
    // Present when the composed draft carries the read-receipt pixel (D-239, D-412).
    readReceiptToken: v.optional(v.string()),
  },
  returns: v.union(
    v.object({ draftId: v.id("drafts"), version: v.number() }),
    v.object({ conflict: v.literal(true), version: v.number() }),
  ),
  handler: async (ctx, args) => {
    if (args.readReceiptToken !== undefined && !RECEIPT_TOKEN.test(args.readReceiptToken))
      throw appError("invalid_argument", "Bad read-receipt token.");
    const account = await ctx.db.get(args.accountId);
    if (!account || account.userId !== args.userId) throw appError("not_found");
    if (args.threadId) {
      const t = await ctx.db.get(args.threadId);
      if (!t || t.userId !== args.userId) throw appError("not_found");
    }
    const existing = await ctx.db
      .query("drafts")
      .withIndex("by_user_client", (q) => q.eq("userId", args.userId).eq("clientDraftId", args.clientDraftId))
      .unique();
    const now = Date.now();
    const fields = {
      accountId: args.accountId,
      threadId: args.threadId,
      replyToMessageId: args.replyToMessageId,
      providerDraftId: args.providerDraftId,
      providerMessageId: args.providerMessageId,
      subject: args.subject.slice(0, 998),
      to: args.to.slice(0, 100),
      cc: args.cc.slice(0, 100),
      bcc: args.bcc.slice(0, 100),
      updatedByDeviceId: args.deviceId,
      updatedAt: now,
    };
    if (!existing) {
      if (args.baseVersion !== 0) return { conflict: true as const, version: 0 };
      const rfcMessageId = args.rfcMessageId.replace(/^<|>$/g, "");
      const draftId = await ctx.db.insert("drafts", {
        userId: args.userId,
        clientDraftId: args.clientDraftId,
        rfcMessageId,
        version: 1,
        state: "editing",
        ...fields,
      });
      await syncReadReceipt(ctx, args.userId, draftId, rfcMessageId, args.readReceiptToken);
      return { draftId, version: 1 };
    }
    if (existing.version !== args.baseVersion || existing.state === "sent" || existing.state === "sending") {
      return { conflict: true as const, version: existing.version };
    }
    const version = existing.version + 1;
    await ctx.db.patch(existing._id, {
      ...fields,
      version,
      state: existing.state === "discarded" ? "editing" : existing.state,
    });
    await syncReadReceipt(ctx, args.userId, existing._id, existing.rfcMessageId, args.readReceiptToken);
    return { draftId: existing._id, version };
  },
});

export const scheduleSend = workerMutation({
  args: {
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    clientSendId: v.string(),
    draftId: v.id("drafts"),
    kind: v.union(v.literal("undo"), v.literal("later")),
    sendAt: v.number(),
    preview: v.object({ subject: v.string(), to: v.array(vAddr) }),
  },
  returns: v.object({
    sendId: v.id("scheduledSends"),
    state: v.union(v.literal("scheduled"), v.literal("due"), v.literal("cancelled")),
  }),
  handler: async (ctx, args) => {
    const existing = await ctx.db
      .query("scheduledSends")
      .withIndex("by_user_client", (q) => q.eq("userId", args.userId).eq("clientSendId", args.clientSendId))
      .unique();
    if (existing) {
      // A cancel tombstone (Undo pressed before /v1/send finished) or a replay.
      const state =
        existing.state === "cancelled" ? "cancelled" : existing.state === "scheduled" ? "scheduled" : "due";
      return { sendId: existing._id, state: state as "cancelled" | "scheduled" | "due" };
    }
    const account = await ctx.db.get(args.accountId);
    const draft = await ctx.db.get(args.draftId);
    if (!account || account.userId !== args.userId || !draft || draft.userId !== args.userId)
      throw appError("not_found");
    if (draft.accountId !== args.accountId)
      throw appError("invalid_argument", "Draft belongs to another account.");
    const now = Date.now();
    const sendId = await ctx.db.insert("scheduledSends", {
      userId: args.userId,
      accountId: args.accountId,
      clientSendId: args.clientSendId,
      draftId: args.draftId,
      kind: args.kind,
      sendAt: args.sendAt,
      state: "scheduled",
      preview: { subject: args.preview.subject.slice(0, 998), to: args.preview.to.slice(0, 10) },
      createdAt: now,
    });
    await ctx.db.patch(draft._id, { state: "scheduled", updatedAt: now });
    if (args.sendAt <= now) {
      await fireSend(ctx, (await ctx.db.get(sendId))!);
      return { sendId, state: "due" as const };
    }
    const scheduledFnId = await ctx.scheduler.runAt(args.sendAt, internal.sends.fire, { sendId });
    await ctx.db.patch(sendId, { scheduledFnId });
    return { sendId, state: "scheduled" as const };
  },
});

// ───────────── resolution and context (D-049) ─────────────

export const resolveMessages = workerQuery({
  args: { userId: v.string(), messageIds: v.array(v.id("messages")) },
  returns: v.array(
    v.object({
      messageId: v.id("messages"),
      accountId: v.id("mailAccounts"),
      threadId: v.id("threads"),
      providerMessageId: v.string(),
      providerThreadId: v.string(),
      imapLocations: v.optional(v.array(vImapLocation)),
      isDraft: v.boolean(),
    }),
  ),
  handler: async (ctx, { userId, messageIds }) => {
    if (messageIds.length > 100) throw appError("invalid_argument", "At most 100 message ids.");
    const out = [];
    const threads = new Map<string, Doc<"threads"> | null>();
    for (const id of messageIds) {
      const m = await ctx.db.get(id);
      if (!m || m.userId !== userId) continue; // ids the user does not own are omitted
      if (!threads.has(m.threadId)) threads.set(m.threadId, await ctx.db.get(m.threadId));
      const t = threads.get(m.threadId);
      if (!t) continue;
      out.push({
        messageId: m._id,
        accountId: m.accountId,
        threadId: m.threadId,
        providerMessageId: m.providerMessageId,
        providerThreadId: t.providerThreadId,
        imapLocations: m.imapLocations,
        isDraft: m.isDraft,
      });
    }
    return out;
  },
});

export const resolveThread = workerQuery({
  args: { userId: v.string(), threadId: v.id("threads") },
  returns: v.union(
    v.object({
      accountId: v.id("mailAccounts"),
      providerThreadId: v.string(),
      messages: v.array(
        v.object({
          messageId: v.id("messages"),
          providerMessageId: v.string(),
          date: v.number(),
          isDraft: v.boolean(),
          imapLocations: v.optional(v.array(vImapLocation)),
        }),
      ),
    }),
    v.null(),
  ),
  handler: async (ctx, { userId, threadId }) => {
    const t = await ctx.db.get(threadId);
    if (!t || t.userId !== userId) return null;
    const msgs = await ctx.db
      .query("messages")
      .withIndex("by_thread_date", (q) => q.eq("threadId", threadId))
      .order("desc")
      .take(500);
    return {
      accountId: t.accountId,
      providerThreadId: t.providerThreadId,
      messages: msgs.map((m) => ({
        messageId: m._id,
        providerMessageId: m.providerMessageId,
        date: m.date,
        isDraft: m.isDraft,
        imapLocations: m.imapLocations,
      })),
    };
  },
});

export const userContext = workerQuery({
  args: { userId: v.string() },
  returns: v.object({
    entitlement: vEntitlement,
    prefs: v.object({ remoteImages: vPrefs.fields.remoteImages, readReceipts: vPrefs.fields.readReceipts }),
    accountIds: v.array(v.id("mailAccounts")),
    // IANA zone for search dates (§11.1), so provider search resolves before:/after: like search.threads (D-404).
    timezone: v.string(),
  }),
  handler: async (ctx, { userId }) => {
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    const accounts = await ctx.db
      .query("mailAccounts")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .take(100);
    return {
      entitlement: await loadEntitlement(ctx, userId),
      prefs: {
        remoteImages: profile?.prefs.remoteImages ?? "block",
        readReceipts: profile?.prefs.readReceipts ?? { enabled: false },
      },
      accountIds: accounts.filter((a) => a.status !== "removing").map((a) => a._id),
      timezone: profile?.timezone ?? "UTC",
    };
  },
});

/** Managed triage only looks at mail this recent; older inbox threads stay untriaged (D-390). */
export const MANAGED_TRIAGE_MAX_AGE_MS = 14 * DAY;

/**
 * Managed triage (§12.5, D-390): the worker claims inbox threads of an account it holds the lease on,
 * for a user whose triage route is `akatsuki` and who is entitled to managed AI. With `threadIds` it
 * claims those threads (the `newInbound` of `upsertThreads`); without, it sweeps the account's untriaged
 * and lease-expired threads, newest first, so a batch that apps/api did not answer is retried. Claimed
 * threads get the same 2-minute lease as `ai.claimTriage`.
 */
export const claimTriage = workerMutation({
  args: {
    accountId: v.id("mailAccounts"),
    workerId: v.string(),
    threadIds: v.optional(v.array(v.id("threads"))),
    limit: v.number(),
  },
  returns: v.array(vTriageInput),
  handler: async (ctx, { accountId, workerId, threadIds, limit }) => {
    if (limit < 1 || limit > 25) throw appError("invalid_argument", "limit is 1 to 25.");
    if (threadIds && threadIds.length > 100) throw appError("invalid_argument", "At most 100 threads.");
    const { account } = await requireLease(ctx, accountId, workerId);
    const ent = await loadEntitlement(ctx, account.userId);
    const ai = await ctx.db
      .query("aiSettings")
      .withIndex("by_user", (q) => q.eq("userId", account.userId))
      .unique();
    if (!ent.managedAi || triageRouteKind(ai) !== "akatsuki") return [];
    const now = Date.now();
    const cutoff = now - MANAGED_TRIAGE_MAX_AGE_MS;
    let pool: Doc<"threads">[];
    if (threadIds) {
      pool = [];
      for (const id of threadIds) {
        const t = await ctx.db.get(id);
        if (t) pool.push(t);
      }
    } else {
      const byState = async (state: "none" | "pending") =>
        await ctx.db
          .query("threads")
          .withIndex("by_user_triage", (q) =>
            q.eq("userId", account.userId).eq("triageState", state).gt("lastMessageAt", cutoff),
          )
          .order("desc")
          .take(limit * 8);
      pool = [...(await byState("none")), ...(await byState("pending"))];
    }
    const out = [];
    for (const t of pool) {
      if (out.length >= limit) break;
      if (t.accountId !== accountId || t.lastMessageAt < cutoff || !triageClaimable(t, now)) continue;
      await ctx.db.patch(t._id, { triageState: "pending", triageLeaseUntil: now + TRIAGE_LEASE_MS });
      out.push(triageInputOf(t));
    }
    return out;
  },
});

export const applyTriage = workerMutation({
  args: { results: v.array(vTriageResult) },
  returns: v.null(),
  handler: async (ctx, { results }) => {
    if (results.length > 50) throw appError("invalid_argument", "At most 50 results.");
    const now = Date.now();
    for (const r of results) {
      const t = await ctx.db.get(r.threadId);
      if (!t) continue;
      await ctx.db.patch(t._id, {
        triage: {
          category: r.category,
          needsReply: r.needsReply,
          source: "managed",
          model: r.model.slice(0, 200),
          promptVersion: r.promptVersion,
          at: now,
        },
        triageState: "done",
        triageLeaseUntil: undefined,
      });
      await recomputeThread(ctx, t._id);
    }
    return null;
  },
});

export const recordReadReceipt = workerMutation({
  args: { token: v.string(), at: v.number(), proxied: v.boolean() },
  returns: v.null(),
  handler: async (ctx, { token, at, proxied }) => {
    if (!RECEIPT_TOKEN.test(token)) return null;
    const row = await ctx.db
      .query("readReceipts")
      .withIndex("by_token", (q) => q.eq("token", token))
      .unique();
    if (!row || row.opens.length >= 100) return null;
    await ctx.db.patch(row._id, { opens: [...row.opens, { at, proxied }] });
    return null;
  },
});

export const ackAccountRemoved = workerMutation({
  args: { accountId: v.id("mailAccounts"), workerId: v.string() },
  returns: v.null(),
  handler: async (ctx, { accountId, workerId }) => {
    const { account } = await requireLease(ctx, accountId, workerId);
    if (account.status !== "removing")
      throw appError("invalid_argument", "The account is not being removed.");
    await ctx.scheduler.runAfter(0, internal.accounts.purge, { accountId, table: "threads", cursor: null });
    return null;
  },
});

// ───────────── provider search hits (D-236, additive) ─────────────

/** Maps provider thread ids of one account to Convex thread ids, so `GET /v1/search` can answer
 *  with `Id<"threads">` after it upserted the provider's hits. Unknown ids are omitted. */
export const threadIdsByProvider = workerQuery({
  args: { accountId: v.id("mailAccounts"), providerThreadIds: v.array(v.string()) },
  returns: v.array(v.object({ providerThreadId: v.string(), threadId: v.id("threads") })),
  handler: async (ctx, { accountId, providerThreadIds }) => {
    if (providerThreadIds.length > 100) throw appError("invalid_argument", "At most 100 ids.");
    const out: { providerThreadId: string; threadId: Id<"threads"> }[] = [];
    for (const providerThreadId of providerThreadIds) {
      const t = await ctx.db
        .query("threads")
        .withIndex("by_account_provider", (q) =>
          q.eq("accountId", accountId).eq("providerThreadId", providerThreadId),
        )
        .unique();
      if (t) out.push({ providerThreadId, threadId: t._id });
    }
    return out;
  },
});
