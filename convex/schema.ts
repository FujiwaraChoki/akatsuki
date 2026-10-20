// Convex schema (ARCHITECTURE §3.2). Must match the contract exactly; change it only together with
// docs/ARCHITECTURE.md and a docs/DECISIONS.md entry.
import { defineSchema, defineTable } from "convex/server";
import { v } from "convex/values";
import {
  vAddr,
  vAiFeature,
  vAiProviderKind,
  vAppError,
  vEntitlement,
  vLabelKey,
  vOpPayload,
  vOpState,
  vPrefs,
  vProvider,
  vRule,
  vThreadAction,
  vTriage,
} from "./lib/validators";

export default defineSchema({
  // ───────────── users & devices ─────────────
  profiles: defineTable({
    userId: v.string(),
    email: v.string(),
    name: v.optional(v.string()),
    timezone: v.string(), // IANA, e.g. "Europe/Zurich"
    onboarding: v.union(v.literal("checkout"), v.literal("connect"), v.literal("done")),
    prefs: vPrefs,
    createdAt: v.number(),
    updatedAt: v.number(),
  }).index("by_user", ["userId"]),

  devices: defineTable({
    userId: v.string(),
    deviceId: v.string(),
    kind: v.union(v.literal("web"), v.literal("desktop"), v.literal("mobile")),
    name: v.string(), // "Firefox on Linux", user-editable
    appVersion: v.string(),
    expoPushToken: v.optional(v.string()), // one row per token: register takes it from any other row (D-403)
    lastSeenAt: v.number(),
  })
    .index("by_user", ["userId"])
    .index("by_user_device", ["userId", "deviceId"])
    .index("by_push_token", ["expoPushToken"]),

  // ───────────── mail accounts & sync state ─────────────
  mailAccounts: defineTable({
    // client-visible; no secrets, no cursors
    userId: v.string(),
    provider: vProvider,
    providerAccountId: v.string(), // gmail: lowercased address; graph: AAD object id; imap: "user@host"
    email: v.string(),
    displayName: v.optional(v.string()),
    color: v.optional(v.string()),
    sortOrder: v.number(),
    shard: v.number(), // fixed at insert; SHARD_COUNT=1 in v1
    status: v.union(
      v.literal("connecting"),
      v.literal("initial_sync"),
      v.literal("live"),
      v.literal("degraded"),
      v.literal("reauth_required"),
      v.literal("paused"),
      v.literal("removing"),
    ),
    statusDetail: v.optional(v.string()), // error code (§15), never provider text
    syncPhase: v.union(
      v.literal("baseline"),
      v.literal("inbox_first"),
      v.literal("window"),
      v.literal("backfill"),
      v.literal("complete"),
    ),
    progress: v.object({ threadsDone: v.number(), threadsEstimated: v.optional(v.number()) }), // written ≤ every 5 s
    windowStartAt: v.number(), // connect time − 90 d
    backfillHorizonAt: v.number(), // default connect − 365 d; 0 = everything
    oldestSyncedAt: v.optional(v.number()),
    caps: v.object({
      multiLabel: v.boolean(), // gmail true; graph/imap false (exclusive folders)
      categories: v.boolean(),
      serverSearch: v.boolean(),
      deleteForever: v.boolean(), // false for gmail (scope gmail.modify)
    }),
    imap: v.optional(
      v.object({
        host: v.string(),
        port: v.number(),
        tls: v.union(v.literal("implicit"), v.literal("starttls")),
        smtpHost: v.string(),
        smtpPort: v.number(),
        smtpTls: v.union(v.literal("implicit"), v.literal("starttls")),
        capabilities: v.array(v.string()),
        sentAutoSaved: v.boolean(),
      }),
    ),
    createdAt: v.number(),
    lastSyncAt: v.optional(v.number()),
  })
    .index("by_user", ["userId", "sortOrder"])
    .index("by_provider_account", ["provider", "providerAccountId"]) // unique: one Akatsuki user per mailbox
    .index("by_provider_email", ["provider", "email"]) // Gmail Pub/Sub fan-in
    .index("by_shard_status", ["shard", "status"]),

  accountSecrets: defineTable({
    // ciphertext only; worker service identity only
    accountId: v.id("mailAccounts"),
    kind: v.union(v.literal("oauth"), v.literal("imap")),
    keyId: v.string(), // which TOKEN_ENC_KEYS entry sealed it
    nonce: v.bytes(), // 24 bytes (XChaCha20)
    ciphertext: v.bytes(),
    version: v.number(), // CAS counter (refresh-token rotation)
    updatedAt: v.number(),
  }).index("by_account_kind", ["accountId", "kind"]),

  syncState: defineTable({
    // worker-only; 1:1 with mailAccounts
    accountId: v.id("mailAccounts"),
    shard: v.number(),
    gmailHistoryId: v.optional(v.string()), // decimal u64
    leaseOwner: v.optional(v.string()), // workerId
    leaseExpiresAt: v.optional(v.number()),
    watchKind: v.optional(
      v.union(
        v.literal("gmail_watch"),
        v.literal("graph_subscription"),
        v.literal("imap_idle"),
        v.literal("poll"),
      ),
    ),
    watchExternalId: v.optional(v.string()), // Graph subscriptionId
    watchExpiresAt: v.optional(v.number()),
    watchClientStateHash: v.optional(v.string()), // hex sha256 of Graph clientState
    backfillBeforeAt: v.optional(v.number()), // date cursor for backfill
    consecutiveFailures: v.number(),
    lastError: v.optional(vAppError),
    updatedAt: v.number(),
  })
    .index("by_account", ["accountId"])
    .index("by_watch_external", ["watchExternalId"])
    .index("by_shard_lease", ["shard", "leaseExpiresAt"]),

  folderCursors: defineTable({
    // graph + imap: one row per remote folder
    accountId: v.id("mailAccounts"),
    remoteFolderId: v.string(), // graph folder id | imap mailbox name (UTF-8)
    labelKey: vLabelKey,
    uidValidity: v.optional(v.number()),
    uidNext: v.optional(v.number()),
    highestModSeq: v.optional(v.string()), // decimal u64
    deltaLink: v.optional(v.string()),
    lastSyncedAt: v.number(),
  }).index("by_account_folder", ["accountId", "remoteFolderId"]),

  accountRevs: defineTable({
    // per-account change counter (desktop mirror feed)
    accountId: v.id("mailAccounts"),
    rev: v.number(),
    gcRev: v.optional(v.number()), // highest tombstone rev deleted by tombstones.gc (D-391)
  }).index("by_account", ["accountId"]),

  tombstones: defineTable({
    // deletions for the mirror feed; GC after 30 d
    accountId: v.id("mailAccounts"),
    rev: v.number(),
    table: v.union(v.literal("threads"), v.literal("messages"), v.literal("labels")),
    docId: v.string(),
    at: v.number(),
  })
    .index("by_account_rev", ["accountId", "rev"])
    .index("by_at", ["at"]),

  identities: defineTable({
    // send-as addresses (Gmail sendAs, aliases)
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    email: v.string(),
    name: v.optional(v.string()),
    replyTo: v.optional(v.string()),
    signatureHtml: v.optional(v.string()), // user-authored; sanitized on write
    isDefault: v.boolean(),
  })
    .index("by_account", ["accountId"])
    .index("by_user", ["userId"]),

  // ───────────── mail metadata (never bodies) ─────────────
  labels: defineTable({
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    providerLabelId: v.optional(v.string()), // undefined until label_create op completes
    name: v.string(),
    kind: v.union(
      v.literal("system"),
      v.literal("user"),
      v.literal("folder"),
      v.literal("category"),
      v.literal("keyword"),
    ),
    role: v.optional(
      v.union(
        v.literal("inbox"),
        v.literal("sent"),
        v.literal("drafts"),
        v.literal("trash"),
        v.literal("spam"),
        v.literal("starred"),
        v.literal("important"),
        v.literal("unread"),
        v.literal("snoozed"),
        v.literal("archive"),
        v.literal("all"),
      ),
    ),
    // Rows whose role is one of the 9 sys roles are addressed as `sys:<role>`; all others as `l:<_id>`.
    exclusive: v.boolean(), // folder semantics
    color: v.optional(v.string()),
    hidden: v.boolean(),
    pendingDelete: v.boolean(),
    rev: v.number(),
  })
    .index("by_account", ["accountId"])
    .index("by_account_provider", ["accountId", "providerLabelId"])
    .index("by_account_role", ["accountId", "role"])
    .index("by_account_rev", ["accountId", "rev"]),

  threads: defineTable({
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    providerThreadId: v.string(),
    subject: v.string(),
    snippet: v.string(), // ≤ 200 chars, latest non-draft message
    participants: v.array(vAddr), // ≤ 10, most recent first
    lastFrom: v.optional(vAddr),
    isBulk: v.boolean(), // List-Id / List-Unsubscribe / Precedence: bulk
    listId: v.optional(v.string()),
    listUnsubscribe: v.optional(
      v.object({ http: v.optional(v.string()), mailto: v.optional(v.string()), oneClick: v.boolean() }),
    ),
    lastMessageAt: v.number(),
    lastInboundAt: v.optional(v.number()),
    lastOutboundAt: v.optional(v.number()),
    sortAt: v.number(), // max(lastMessageAt, returnedAt)
    messageCount: v.number(),
    unreadCount: v.number(),
    draftCount: v.number(),
    hasAttachments: v.boolean(),
    syncedLabelKeys: v.array(vLabelKey), // last provider-confirmed state
    labelKeys: v.array(vLabelKey), // visible = applyOps(synced, pending); never written directly
    split: v.optional(v.string()), // "split:<id>" | "split:other"; set iff labelKeys ∋ sys:inbox
    pendingOpCount: v.number(),
    deletedPending: v.boolean(), // a queued/inflight delete_forever hides it
    providerVersion: v.optional(v.int64()), // gmail thread historyId; graph max lastModified ms
    snoozedUntil: v.optional(v.number()),
    returnedAt: v.optional(v.number()), // set when a snooze/reminder brought it back
    reminderAt: v.optional(v.number()),
    triage: v.optional(vTriage),
    triageState: v.union(v.literal("none"), v.literal("pending"), v.literal("done"), v.literal("off")),
    triageLeaseUntil: v.optional(v.number()),
    rev: v.number(),
    updatedAt: v.number(),
  })
    .index("by_account_provider", ["accountId", "providerThreadId"])
    .index("by_user_time", ["userId", "lastMessageAt"]) // "All mail", unified
    .index("by_account_time", ["accountId", "lastMessageAt"])
    .index("by_account_rev", ["accountId", "rev"]) // desktop mirror feed
    .index("by_user_triage", ["userId", "triageState", "lastMessageAt"]),

  // List membership: one row per (thread, visible labelKey) plus one split row for inbox threads.
  threadLabels: defineTable({
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    threadId: v.id("threads"),
    labelKey: vLabelKey, // incl. "split:*"
    sortAt: v.number(), // thread.sortAt; for sys:snoozed = snoozedUntil
    isUnread: v.boolean(),
  })
    .index("by_user_label_sort", ["userId", "labelKey", "sortAt"])
    .index("by_user_label_unread_sort", ["userId", "labelKey", "isUnread", "sortAt"])
    .index("by_account_label_sort", ["accountId", "labelKey", "sortAt"])
    .index("by_account_label_unread_sort", ["accountId", "labelKey", "isUnread", "sortAt"])
    .index("by_thread", ["threadId"]),

  // Search text lives apart from threads so list renders do not read it (DB bandwidth).
  threadSearch: defineTable({
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    threadId: v.id("threads"),
    text: v.string(), // subject + participant names/emails + last 3 snippets, ≤ 1500
    inInbox: v.boolean(),
    isUnread: v.boolean(),
    isStarred: v.boolean(),
    hasAttachments: v.boolean(),
    isOutbound: v.boolean(), // thread contains a sent message
    lastMessageAt: v.number(),
  })
    .index("by_thread", ["threadId"])
    .searchIndex("search_text", {
      searchField: "text",
      filterFields: [
        "userId",
        "accountId",
        "inInbox",
        "isUnread",
        "isStarred",
        "hasAttachments",
        "isOutbound",
      ],
    }),

  messages: defineTable({
    // headers only. No body, ever.
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    threadId: v.id("threads"),
    providerMessageId: v.string(),
    rfcMessageId: v.optional(v.string()),
    inReplyTo: v.optional(v.string()),
    references: v.array(v.string()), // last 20
    from: v.optional(vAddr),
    to: v.array(vAddr),
    cc: v.array(vAddr),
    bcc: v.array(vAddr),
    replyTo: v.array(vAddr),
    subject: v.string(),
    snippet: v.string(),
    date: v.number(),
    labelKeys: v.array(vLabelKey), // provider state only (sys:* and l:*)
    isUnread: v.boolean(),
    isDraft: v.boolean(),
    isOutbound: v.boolean(), // from one of the account's identities
    hasAttachments: v.boolean(), // best effort, §9.5; exact list comes with the body
    sizeBytes: v.optional(v.number()),
    listId: v.optional(v.string()),
    authSummary: v.optional(v.object({ spf: v.string(), dkim: v.string(), dmarc: v.string() })),
    imapLocations: v.optional(
      v.array(v.object({ remoteFolderId: v.string(), uidValidity: v.number(), uid: v.number() })),
    ),
    rev: v.number(),
  })
    .index("by_thread_date", ["threadId", "date"])
    .index("by_account_provider", ["accountId", "providerMessageId"])
    .index("by_account_rfc", ["accountId", "rfcMessageId"])
    .index("by_account_rev", ["accountId", "rev"]),

  contacts: defineTable({
    // derived from headers, for autocomplete
    userId: v.string(),
    email: v.string(),
    name: v.optional(v.string()),
    sentCount: v.number(),
    receivedCount: v.number(),
    lastAt: v.number(),
    searchText: v.string(),
  })
    .index("by_user_email", ["userId", "email"])
    .index("by_user_last", ["userId", "lastAt"])
    .searchIndex("search_contacts", { searchField: "searchText", filterFields: ["userId"] }),

  // ───────────── ops outbox ─────────────
  ops: defineTable({
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    shard: v.number(),
    threadId: v.optional(v.id("threads")), // set for kind "thread"
    labelId: v.optional(v.id("labels")), // set for label_* kinds and thread ops that add an unsynced l: key
    clientOpId: v.string(), // one per user gesture; shared by its per-thread ops
    payload: vOpPayload,
    effect: v.optional(v.object({ added: v.array(vLabelKey), removed: v.array(vLabelKey) })), // diff at enqueue, for undo
    inverseOf: v.optional(v.id("ops")),
    state: vOpState,
    attempt: v.number(),
    maxAttempts: v.number(),
    availableAt: v.number(), // not claimable before this instant
    leaseOwner: v.optional(v.string()),
    leaseExpiresAt: v.optional(v.number()),
    sendStartedAt: v.optional(v.number()), // send only: set just before the provider call
    verifyOnly: v.optional(v.boolean()), // send only: outcome ambiguous; never re-send
    outcome: v.optional(v.union(v.literal("applied"), v.literal("moot"))),
    lastError: v.optional(vAppError),
    failureAcked: v.boolean(),
    createdAt: v.number(),
    completedAt: v.optional(v.number()),
  })
    .index("by_user_clientOp", ["userId", "clientOpId"])
    .index("by_account_state", ["accountId", "state"]) // + implicit _creationTime = FIFO
    .index("by_shard_state_available", ["shard", "state", "availableAt"])
    .index("by_thread_state", ["threadId", "state"])
    .index("by_label_state", ["labelId", "state"])
    .index("by_state_lease", ["state", "leaseExpiresAt"])
    .index("by_user_state_acked", ["userId", "state", "failureAcked"])
    .index("by_state_completed", ["state", "completedAt"]),

  // ───────────── compose, drafts, sends ─────────────
  drafts: defineTable({
    // metadata only; text lives in the provider draft + devices
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    clientDraftId: v.string(),
    threadId: v.optional(v.id("threads")),
    replyToMessageId: v.optional(v.id("messages")),
    providerDraftId: v.optional(v.string()),
    providerMessageId: v.optional(v.string()),
    rfcMessageId: v.string(), // fixed at creation; send dedupe key
    version: v.number(), // monotonic, assigned by the worker (CAS)
    subject: v.string(),
    to: v.array(vAddr),
    cc: v.array(vAddr),
    bcc: v.array(vAddr),
    state: v.union(
      v.literal("editing"),
      v.literal("scheduled"),
      v.literal("sending"),
      v.literal("sent"),
      v.literal("discarded"),
    ),
    updatedByDeviceId: v.optional(v.string()),
    updatedAt: v.number(),
  })
    .index("by_user_client", ["userId", "clientDraftId"])
    .index("by_user_updated", ["userId", "updatedAt"])
    .index("by_thread", ["threadId"]),

  scheduledSends: defineTable({
    userId: v.string(),
    accountId: v.optional(v.id("mailAccounts")), // absent on a cancel tombstone
    clientSendId: v.string(),
    draftId: v.optional(v.id("drafts")), // absent on a cancel tombstone
    kind: v.union(v.literal("undo"), v.literal("later")),
    sendAt: v.number(),
    state: v.union(
      v.literal("scheduled"),
      v.literal("cancelled"),
      v.literal("due"),
      v.literal("sending"),
      v.literal("sent"),
      v.literal("failed"),
      v.literal("unknown"),
    ),
    scheduledFnId: v.optional(v.id("_scheduled_functions")),
    opId: v.optional(v.id("ops")),
    preview: v.object({ subject: v.string(), to: v.array(vAddr) }),
    sentProviderMessageId: v.optional(v.string()),
    error: v.optional(vAppError),
    createdAt: v.number(),
  })
    .index("by_user_client", ["userId", "clientSendId"])
    .index("by_user_state_sendAt", ["userId", "state", "sendAt"])
    .index("by_state_sendAt", ["state", "sendAt"]),

  // ───────────── timers (snooze, reminders) ─────────────
  timers: defineTable({
    userId: v.string(),
    accountId: v.id("mailAccounts"),
    threadId: v.id("threads"),
    kind: v.union(v.literal("snooze"), v.literal("reminder")),
    fireAt: v.number(),
    condition: v.union(v.literal("always"), v.literal("if_no_reply")), // reminders; snooze = "always"
    anchorAt: v.number(), // set time; "reply" = inbound message after this
    returnUnread: v.boolean(),
    note: v.optional(v.string()), // user-typed, ≤ 500 chars
    state: v.union(v.literal("active"), v.literal("fired"), v.literal("skipped"), v.literal("cancelled")),
    scheduledFnId: v.optional(v.id("_scheduled_functions")),
    createdAt: v.number(),
  })
    .index("by_thread_kind_state", ["threadId", "kind", "state"])
    .index("by_user_kind_state_fire", ["userId", "kind", "state", "fireAt"])
    .index("by_state_fire", ["state", "fireAt"]),

  // ───────────── user configuration ─────────────
  splits: defineTable({
    userId: v.string(),
    name: v.string(),
    order: v.number(),
    rule: vRule,
    showCount: v.boolean(),
  }).index("by_user_order", ["userId", "order"]),

  rules: defineTable({
    // filters applied to newly arrived inbound mail
    userId: v.string(),
    accountId: v.optional(v.id("mailAccounts")),
    name: v.string(),
    enabled: v.boolean(),
    order: v.number(),
    rule: vRule,
    actions: v.array(vThreadAction), // no unsubscribe/delete_forever
  }).index("by_user_order", ["userId", "order"]),

  snippets: defineTable({
    // user-authored templates, not message text
    userId: v.string(),
    name: v.string(),
    shortcut: v.optional(v.string()), // e.g. "thx"
    subject: v.optional(v.string()),
    bodyHtml: v.string(), // ≤ 20 KB, sanitized on write
    variables: v.array(v.string()), // "first_name", ... used as {{first_name}}
    searchText: v.string(),
    useCount: v.number(),
    lastUsedAt: v.optional(v.number()),
    updatedAt: v.number(),
  })
    .index("by_user_name", ["userId", "name"])
    .index("by_user_shortcut", ["userId", "shortcut"])
    .searchIndex("search_snippets", { searchField: "searchText", filterFields: ["userId"] }),

  senderPrefs: defineTable({
    userId: v.string(),
    sender: v.string(), // "a@b.com" or "@b.com"
    allowRemoteImages: v.boolean(),
    vip: v.boolean(),
    blocked: v.boolean(),
  }).index("by_user_sender", ["userId", "sender"]),

  readReceipts: defineTable({
    // opt-in only (BRIEF decision 8)
    userId: v.string(),
    token: v.string(), // 22-char base64url random
    draftId: v.optional(v.id("drafts")),
    messageId: v.optional(v.id("messages")),
    rfcMessageId: v.optional(v.string()), // links the sent message (D-412)
    opens: v.array(v.object({ at: v.number(), proxied: v.boolean() })), // ≤ 100
    createdAt: v.number(),
  })
    .index("by_token", ["token"])
    .index("by_message", ["messageId"])
    .index("by_user_rfc", ["userId", "rfcMessageId"]),

  // ───────────── AI (no keys, no message text) ─────────────
  aiSettings: defineTable({
    userId: v.string(),
    profiles: v.array(
      v.object({
        profileId: v.string(), // UUIDv4
        label: v.string(),
        kind: vAiProviderKind,
        baseUrl: v.optional(v.string()), // https://… or http://localhost/LAN; no userinfo, no query
        models: v.object({
          fast: v.optional(v.string()),
          default: v.optional(v.string()),
          heavy: v.optional(v.string()),
          transcription: v.optional(v.string()),
        }),
        keyOnDevices: v.array(v.string()), // deviceIds whose keychain holds a key; never the key
      }),
    ),
    routes: v.array(
      v.object({
        feature: v.union(vAiFeature, v.literal("default")),
        profileId: v.string(),
        model: v.optional(v.string()),
        enabled: v.boolean(),
      }),
    ),
    consent: v.object({
      grantedAt: v.optional(v.number()),
      version: v.optional(v.number()), // consent text version shown
      autoTriage: v.boolean(),
      syncVoiceProfile: v.boolean(),
    }),
    voiceProfile: v.optional(v.string()), // ≤ 2048 chars; only if consent.syncVoiceProfile
    updatedAt: v.number(),
  }).index("by_user", ["userId"]),

  aiUsage: defineTable({
    // managed AI only
    userId: v.string(),
    period: v.string(), // "YYYY-MM" (UTC)
    costMicros: v.number(),
    requests: v.number(),
    inputTokens: v.number(),
    outputTokens: v.number(),
    audioSeconds: v.number(),
    updatedAt: v.number(),
  }).index("by_user_period", ["userId", "period"]),

  // ───────────── billing (ee) ─────────────
  billing: defineTable({
    userId: v.string(),
    stripeCustomerId: v.optional(v.string()),
    stripeSubscriptionId: v.optional(v.string()),
    status: v.union(
      v.literal("none"),
      v.literal("trialing"),
      v.literal("active"),
      v.literal("past_due"),
      v.literal("canceled"),
      v.literal("unpaid"),
      v.literal("incomplete"),
      v.literal("incomplete_expired"),
      v.literal("paused"),
    ),
    priceId: v.optional(v.string()),
    trialEnd: v.optional(v.number()),
    currentPeriodEnd: v.optional(v.number()),
    cancelAtPeriodEnd: v.boolean(),
    pastDueSince: v.optional(v.number()),
    trialUsedAt: v.optional(v.number()), // one trial per user, ever
    lapsedAt: v.optional(v.number()),
    entitlement: vEntitlement,
    reevaluateFnId: v.optional(v.id("_scheduled_functions")),
    purgeFnId: v.optional(v.id("_scheduled_functions")),
    snapshotFetchedAt: v.optional(v.number()),
    updatedAt: v.number(),
  })
    .index("by_user", ["userId"])
    .index("by_customer", ["stripeCustomerId"]),

  stripeEvents: defineTable({ eventId: v.string(), type: v.string(), processedAt: v.number() })
    .index("by_event", ["eventId"])
    .index("by_processed", ["processedAt"]),

  trialLedger: defineTable({
    // one trial per person, ever: survives account deletion (D-393)
    emailHash: v.string(), // hex sha256 of the lowercased login email
    trialUsedAt: v.number(),
  }).index("by_email_hash", ["emailHash"]),

  authRateLimits: defineTable({
    // Better Auth per-IP counters (customStorage, D-401)
    key: v.string(), // "<ip>|<path>"
    count: v.number(),
    lastRequest: v.number(),
  })
    .index("by_key", ["key"])
    .index("by_last_request", ["lastRequest"]),

  authHandoffs: defineTable({
    // mobile sign-in handoff (D-408): one row per redirect, single use, 3 minutes
    codeHash: v.string(), // base64url sha256 of the code sent to akatsuki://auth/callback
    challenge: v.string(), // base64url sha256 of the app's verifier
    ott: v.string(), // Better Auth one-time token (3 min)
    expiresAt: v.number(),
  })
    .index("by_code_hash", ["codeHash"])
    .index("by_expires", ["expiresAt"]),
});
