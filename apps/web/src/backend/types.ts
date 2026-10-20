// The data layer the UI talks to. Two implementations: `convex.tsx` (Convex live queries + Better Auth +
// the sync worker, production) and `mock/` (in memory, for Vitest, Playwright and offline demos). The UI
// never imports Convex or the worker client directly, so every screen runs against both (D-262).
import type { ThreadAction } from "@akatsuki/mail-model";
import type {
  AaspEvent,
  AdoptResponse,
  AiFeatureRequest,
  DraftPut,
  DraftSaved,
  MessageBody,
  QuotaResponse,
  SendAccepted,
  SendRequest,
  StreamedAiFeature,
  TranscribeRequest,
  TranscribeResponse,
  UploadResponse,
} from "@akatsuki/wire";
import type {
  AiSettingsView,
  BillingView,
  DeviceView,
  DraftMeta,
  Entitlement,
  Id,
  IdentityView,
  LabelView,
  MailAccountView,
  MessageMeta,
  OpFailure,
  OpSummary,
  Prefs,
  ProfileView,
  Rule,
  RuleView,
  ScheduledSendView,
  SnippetView,
  SplitView,
  ThreadRow,
  TimerView,
  TriageInputRow,
} from "@akatsuki/wire/convex";

export type {
  AiSettingsView,
  BillingView,
  DeviceView,
  DraftMeta,
  Entitlement,
  IdentityView,
  LabelView,
  MailAccountView,
  MessageBody,
  MessageMeta,
  OpFailure,
  Prefs,
  Rule,
  RuleView,
  ScheduledSendView,
  SnippetView,
  SplitView,
  ThreadRow,
  TimerView,
  TriageInputRow,
};
export type AccountId = Id<"mailAccounts">;
export type ThreadId = Id<"threads">;
export type MessageId = Id<"messages">;
export type TimerId = Id<"timers">;
export type SplitId = Id<"splits">;
export type RuleId = Id<"rules">;
export type SnippetId = Id<"snippets">;
export type IdentityId = Id<"identities">;
export type LabelId = Id<"labels">;

export type SocialProvider = "google" | "microsoft";

export type AuthState =
  | { status: "loading" }
  | { status: "signed_out" }
  | { status: "signed_in"; user: { id: string; email: string; name?: string | null } };

export interface AuthApi {
  /** Sends a sign-in link that comes back to `callbackURL` (absolute, this origin). */
  sendMagicLink(email: string, callbackURL: string): Promise<void>;
  sendOtp(email: string): Promise<void>;
  verifyOtp(email: string, otp: string): Promise<void>;
  /** Redirects to the provider; resolves only on failure. */
  signInSocial(provider: SocialProvider, callbackURL: string): Promise<void>;
  signOut(): Promise<void>;
  /** Better Auth sessions of this user (Settings → Devices). */
  listSessions(): Promise<SessionInfo[]>;
  /** Revokes one session (another browser, a desktop app). */
  revokeSession(token: string): Promise<void>;
  /** Better Auth `deleteUser` (the last step of account deletion, §4.2 `me.deleteAccount`). */
  deleteUser(): Promise<void>;
}

export interface SessionInfo {
  id: string;
  /** Opaque; only passed back to `revokeSession`. */
  token: string;
  userAgent: string | null;
  createdAt: number;
  expiresAt: number;
  current: boolean;
}

/** `ee.billing.entitlement` (§4.2): the entitlement, the billing view and this month's managed-AI use. */
export interface BillingInfo {
  entitlement: Entitlement;
  billing: BillingView;
  aiUsage: { period: string; usedMicros: number; limitMicros: number };
}

export interface MeView {
  userId: string;
  email: string;
  name?: string;
  profile: ProfileView;
  entitlement: Entitlement;
}

export interface ThreadDetail {
  thread: ThreadRow;
  account: {
    _id: AccountId;
    provider: "gmail" | "graph" | "imap";
    email: string;
    caps: { multiLabel: boolean; categories: boolean; serverSearch: boolean; deleteForever: boolean };
  };
  messages: MessageMeta[];
  truncated: boolean;
  pendingOps: OpSummary[];
  timers: TimerView[];
}

export type ListStatus = "loading" | "more" | "loading_more" | "done";

export interface ListResult {
  rows: ThreadRow[];
  status: ListStatus;
  loadMore(n: number): void;
}

export interface SearchResult extends ListResult {
  /** Grammar features Convex search could not apply (§11.2); offer "Search the full mailbox". */
  unsupported: string[];
}

export interface ListViewArgs {
  labelKey: string;
  accountId?: AccountId;
  unreadOnly?: boolean;
}

export interface SearchArgs {
  q: string;
  /** Rounded down to the minute (§11.1). */
  nowAt: number;
  accountId?: AccountId;
}

export interface Mutations {
  apply(a: { clientOpId: string; threadIds: ThreadId[]; action: ThreadAction }): Promise<unknown>;
  undo(a: {
    clientOpId: string;
    undoClientOpId: string;
  }): Promise<{ mode: "cancelled" | "inverse" | "noop" }>;
  snooze(a: {
    clientOpId: string;
    threadIds: ThreadId[];
    until: number;
    returnUnread: boolean;
  }): Promise<unknown>;
  unsnooze(a: { clientOpId: string; threadIds: ThreadId[] }): Promise<unknown>;
  setReminder(a: {
    threadId: ThreadId;
    at: number;
    condition: "always" | "if_no_reply";
    note?: string;
  }): Promise<unknown>;
  bootstrap(a: { timezone: string }): Promise<unknown>;
  updatePrefs(a: { patch: Partial<Prefs> }): Promise<unknown>;
  updateProfile(a: {
    name?: string;
    timezone?: string;
    onboarding?: "checkout" | "connect" | "done";
  }): Promise<unknown>;
  ackFailure(a: { opIds: Id<"ops">[] }): Promise<unknown>;
  retryOp(a: { opId: Id<"ops">; clientOpId: string }): Promise<unknown>;
  setSenderPref(a: {
    sender: string;
    allowRemoteImages?: boolean;
    vip?: boolean;
    blocked?: boolean;
  }): Promise<unknown>;
  registerDevice(a: { deviceId: string; kind: "web"; name: string; appVersion: string }): Promise<unknown>;

  // ───────────── part 2: compose, settings, AI (§4.2) ─────────────
  cancelReminder(a: { timerId: TimerId }): Promise<unknown>;
  discardDraft(a: { clientOpId: string; clientDraftId: string }): Promise<unknown>;
  cancelSend(a: { clientSendId: string }): Promise<{ result: "cancelled" | "too_late" }>;
  rescheduleSend(a: { clientSendId: string; sendAt: number }): Promise<unknown>;
  resolveUnknownSend(a: { clientSendId: string; resolution: "was_sent" | "discard" }): Promise<unknown>;
  saveSplit(a: {
    splitId?: SplitId;
    name: string;
    rule: Rule;
    showCount: boolean;
  }): Promise<{ splitId: SplitId }>;
  removeSplit(a: { splitId: SplitId }): Promise<unknown>;
  reorderSplits(a: { order: SplitId[] }): Promise<unknown>;
  saveRule(a: {
    ruleId?: RuleId;
    accountId?: AccountId;
    name: string;
    enabled: boolean;
    order: number;
    rule: Rule;
    actions: ThreadAction[];
  }): Promise<{ ruleId: RuleId }>;
  removeRule(a: { ruleId: RuleId }): Promise<unknown>;
  saveSnippet(a: {
    snippetId?: SnippetId;
    name: string;
    shortcut?: string;
    subject?: string;
    bodyHtml: string;
    variables: string[];
  }): Promise<{ snippetId: SnippetId }>;
  removeSnippet(a: { snippetId: SnippetId }): Promise<unknown>;
  /** `labels.create` (§4.2): a label (Gmail), category (Outlook) or keyword (IMAP); the provider side follows. */
  createLabel(a: { clientOpId: string; accountId: AccountId; name: string }): Promise<{
    labelId: LabelId;
    labelKey: string;
  }>;
  renameLabel(a: { clientOpId: string; labelId: LabelId; name: string }): Promise<unknown>;
  removeLabel(a: { clientOpId: string; labelId: LabelId }): Promise<unknown>;
  markSnippetUsed(a: { snippetId: SnippetId }): Promise<unknown>;
  updateIdentity(a: {
    identityId: IdentityId;
    name?: string;
    replyTo?: string;
    signatureHtml?: string;
    isDefault?: boolean;
  }): Promise<unknown>;
  updateAccount(a: {
    accountId: AccountId;
    displayName?: string;
    color?: string;
    sortOrder?: number;
  }): Promise<unknown>;
  setBackfillHorizon(a: { accountId: AccountId; horizonAt: number }): Promise<unknown>;
  removeAccount(a: { accountId: AccountId }): Promise<unknown>;
  saveAiSettings(a: AiSettingsPatch): Promise<unknown>;
  removeDevice(a: { deviceId: string }): Promise<unknown>;
  deleteAccount(a: { confirm: "DELETE" }): Promise<unknown>;
  claimTriage(a: { deviceId: string; limit: number }): Promise<TriageInputRow[]>;
  setTriage(a: {
    results: {
      threadId: ThreadId;
      category: NonNullable<ThreadRow["triage"]>["category"];
      needsReply: boolean;
      model: string;
      promptVersion: number;
    }[];
  }): Promise<unknown>;
}

/** `ai.saveSettings` arguments (§4.2); every field optional, `voiceProfile: null` clears it. */
export interface AiSettingsPatch {
  profiles?: AiSettingsView["profiles"];
  routes?: AiSettingsView["routes"];
  consent?: AiSettingsView["consent"];
  voiceProfile?: string | null;
}

/** The mutation names the durable outbox can replay (§16.1). */
export type OutboxFn = "apply" | "undo" | "snooze" | "unsnooze";

export interface ImapEndpoint {
  host: string;
  port: number;
  tls: "implicit" | "starttls";
}

export interface ImapDiscovery {
  imap: ImapEndpoint;
  smtp: ImapEndpoint;
  username: string;
  source: "well_known" | "autoconfig" | "ispdb" | "guess";
  appPasswordRequired: boolean;
}

export interface WorkerApi {
  /** Bodies of a thread, newest first, as the worker fetches them (§7.2). */
  threadBodies(threadId: ThreadId, signal?: AbortSignal): AsyncGenerator<MessageBody>;
  getMessage(messageId: MessageId, signal?: AbortSignal): Promise<MessageBody>;
  /** An inline or attached part as a Blob (cid images become data: URLs). */
  getPartBlob(messageId: MessageId, partId: string, signal?: AbortSignal): Promise<Blob>;
  /** A short-lived absolute download URL (`POST /v1/links`). */
  createLink(messageId: MessageId, partId: string): Promise<string>;
  connectStart(
    provider: "google" | "microsoft",
    body: { returnTo: string; loginHint?: string; accountId?: AccountId },
  ): Promise<string>;
  /** Redeems the `?grant=` the OAuth callback left in the URL (§6.7, D-395). */
  connectComplete(grant: string): Promise<{ accountId: AccountId; reconnected: boolean }>;
  connectImap(body: {
    email: string;
    username: string;
    password: string;
    imap: ImapEndpoint;
    smtp: ImapEndpoint;
  }): Promise<AccountId>;
  /** Host autodetect for the IMAP form (D-264). Never throws: falls back to the built-in table. */
  discoverImap(email: string, signal?: AbortSignal): Promise<ImapDiscovery | null>;
  /** Provider-side search (§11.2 backend 2). */
  search(accountId: AccountId, q: string, signal?: AbortSignal): Promise<ThreadId[]>;
  syncAccount(accountId: AccountId): Promise<void>;
  /** `POST /v1/uploads` (≤ 25 MB, encrypted spool for 24 h, §10.4). */
  upload(file: Blob, filename: string, signal?: AbortSignal): Promise<UploadResponse>;
  /** Autosave (§10.1). Throws `draft_version_conflict` (with `version`) on a stale `baseVersion`. */
  putDraft(clientDraftId: string, body: DraftPut): Promise<DraftSaved>;
  /** Opens a provider draft (`messageId` is its Convex message) for editing under `clientDraftId`. */
  adoptDraft(clientDraftId: string, messageId: MessageId): Promise<AdoptResponse>;
  deleteDraft(clientDraftId: string): Promise<void>;
  /** `POST /v1/send` (§10.2): the worker saves the final draft, then schedules it. */
  send(body: SendRequest): Promise<SendAccepted>;
}

/** apps/api (§8): managed AI, transcription, billing. `configured` is false without `VITE_API_URL`. */
export interface ApiService {
  configured: boolean;
  streamFeature(
    feature: StreamedAiFeature,
    body: AiFeatureRequest,
    signal?: AbortSignal,
  ): AsyncGenerator<AaspEvent>;
  transcribe(req: TranscribeRequest, signal?: AbortSignal): Promise<TranscribeResponse>;
  quota(): Promise<QuotaResponse>;
  checkout(): Promise<{ url: string }>;
  portal(): Promise<{ url: string }>;
  cancel(body: { immediately: boolean }): Promise<void>;
}

export interface Backend {
  kind: "convex" | "mock";
  /** Wraps the app (Convex + Better Auth providers, or the mock store). */
  Provider: (props: { children: React.ReactNode }) => React.ReactNode;
  auth: AuthApi;
  loginProviders: SocialProvider[];
  worker: WorkerApi;
  api: ApiService;
  /** One-shot `mail.getThread` (reply from the list, voice samples); `null` when gone. */
  fetchThread(threadId: ThreadId): Promise<ThreadDetail | null>;
  /** One-shot `drafts.get` (conflict reload, follow-up reminders on new mail). */
  fetchDraft(clientDraftId: string): Promise<DraftMeta | null>;
  useAuthState(): AuthState;
  /** `undefined` while loading, `null` until the user rows exist (D-057). */
  useMe(): MeView | null | undefined;
  useAccounts(): MailAccountView[] | undefined;
  useLabels(): LabelView[] | undefined;
  useSplits(): SplitView[] | undefined;
  /** `null` view = nothing to load. */
  useThreadList(view: ListViewArgs | null): ListResult;
  /** Rows by id (reminders, provider search hits). Missing ids are dropped. */
  useThreadRows(ids: readonly ThreadId[]): ThreadRow[] | undefined;
  useThread(threadId: ThreadId | null): ThreadDetail | null | undefined;
  /** `mail.unreadCounts` (each capped at 100). */
  useUnreadCounts(labelKeys: readonly string[], accountId?: AccountId): Record<string, number> | undefined;
  useSearch(args: SearchArgs | null): SearchResult;
  useTimers(kind: "snooze" | "reminder"): TimerView[] | undefined;
  useOpFailures(): OpFailure[] | undefined;
  useSenderPref(sender: string | null): { allowRemoteImages: boolean; vip: boolean } | null | undefined;
  useContacts(q: string | null): { email: string; name?: string }[] | undefined;
  useMutations(): Mutations;
  /** False while the live connection is down (shown as an offline banner). */
  useConnected(): boolean;

  // ───────────── part 2 ─────────────
  useIdentities(accountId?: AccountId): IdentityView[] | undefined;
  useSnippets(): SnippetView[] | undefined;
  useRules(): RuleView[] | undefined;
  useDevices(): DeviceView[] | undefined;
  /** `null` until the row exists: treat as no profiles, no routes, no consent (D-353). */
  useAiSettings(): AiSettingsView | null | undefined;
  useBilling(): BillingInfo | undefined;
  useSends(
    states?: ("scheduled" | "due" | "sending" | "failed" | "unknown")[],
  ): ScheduledSendView[] | undefined;
  /** The first page (50) of `drafts.list`, newest first. */
  useDrafts(): DraftMeta[] | undefined;
  useReadReceipt(
    messageId: MessageId | null,
  ): { opens: { at: number; proxied: boolean }[] } | null | undefined;
  /** The full `senderPrefs.get` row (blocked included). */
  useSenderPrefFull(
    sender: string | null,
  ): { sender: string; allowRemoteImages: boolean; vip: boolean; blocked: boolean } | null | undefined;
}
