// Sync worker HTTP API wire types (ARCHITECTURE §7.3, routes §7.2). Serde twins: crates/core::wire::worker.
// Fixtures: fixtures/worker/*.json ({type, value}); every value must round-trip unchanged on both sides.
//
// Conventions: fields camelCase, enum values snake_case, instants are Unix ms, ids are Convex ids. Optional
// fields are omitted when absent (never `null`), except where the type says `| null`.
import type { Id, UnixMs } from "./ids.ts";

export type Addr = { name?: string; email: string };

export type TlsMode = "implicit" | "starttls";

/** `POST /v1/connect/{google|microsoft}/start` body. `returnTo` must be allow-listed by the worker. */
export type ConnectStart = { returnTo: string; loginHint?: string; accountId?: Id<"mailAccounts"> };
/** `POST /v1/connect/{provider}/start` 200 body. */
export type ConnectStartResponse = { authorizeUrl: string };

/**
 * `POST /v1/connect/complete` body (D-395). The OAuth callback sends the browser back to `returnTo` with
 * `?status=pending&grant=<handle>`; the client that started the flow redeems the handle with its own JWT.
 */
export type ConnectComplete = { grant: string };
/** `POST /v1/connect/complete` 201 body. A grant started by another user is refused (`invalid_argument`). */
export type ConnectCompleteResponse = { accountId: Id<"mailAccounts">; reconnected: boolean };

/** `GET /v1/connect/imap/discover?email=` 200 body (D-396); `404 not_found` when nothing is known. */
export type ImapDiscovery = {
  imap: { host: string; port: number; tls: TlsMode };
  smtp: { host: string; port: number; tls: TlsMode };
  /** The login name to pre-fill. */
  username: string;
  source: "well_known" | "autoconfig" | "ispdb";
  appPasswordRequired: boolean;
};

/** `POST /v1/connect/imap` body. The password crosses TLS once and is never echoed or logged. */
export type ImapConnect = {
  email: string;
  username: string;
  password: string;
  imap: { host: string; port: number; tls: TlsMode };
  smtp: { host: string; port: number; tls: TlsMode };
};
/** `POST /v1/connect/imap` 201 body. Failures: 422 `imap_auth_failed | smtp_auth_failed | tls_failed | unreachable`. */
export type ImapConnectResponse = { accountId: Id<"mailAccounts"> };

/** `partId` is opaque to clients (an IMAP-style section path from crates/mime, D-095). */
export type AttachmentMeta = {
  partId: string;
  filename: string;
  mimeType: string;
  size: number;
  inline: boolean;
  contentId?: string;
};

/**
 * Where quoted history starts, for collapsing it. **Offsets are UTF-8 byte offsets** into `html` / `text`
 * (the worker is Rust). JavaScript strings are UTF-16: convert with `utf8OffsetToUtf16()` or use
 * `splitQuoted()` from `./offsets.ts` before slicing. `null` = no quoted history found.
 */
export type QuotedOffsets = { htmlStart: number | null; textStart: number | null };

export type ListUnsubscribe = { http?: string; mailto?: string; oneClick: boolean };
export type AuthSummary = { spf: string; dkim: string; dmarc: string };

/** A sanitized message body (§14.3). `GET /v1/messages/{id}`, NDJSON lines of bodies and prefetch. */
export type MessageBody = {
  messageId: Id<"messages">;
  providerMessageId: string;
  /** Bumping it invalidates device caches; the body cache key is `(messageId, sanitizerVersion)`. */
  sanitizerVersion: number;
  /** Sanitized HTML: remote images as `data-akatsuki-src`, cid images as `data-akatsuki-cid`. */
  html: string | null;
  text: string | null;
  /** UTF-8 byte offsets (see `QuotedOffsets`). */
  quoted: QuotedOffsets;
  attachments: AttachmentMeta[];
  remoteImageCount: number;
  trackersRemoved: number;
  headers: {
    /** Full lists (Convex caps them at 100). */
    to: Addr[];
    cc: Addr[];
    replyTo: Addr[];
    listUnsubscribe: ListUnsubscribe | null;
    auth: AuthSummary | null;
  };
};

/** `?format=` of `GET /v1/messages/{id}` and `POST /v1/prefetch`. `raw` is desktop only. */
export type BodyFormat = "sanitized" | "raw";

/** `POST /v1/prefetch` body (at most 100 ids). Response: NDJSON of `MessageBody` or `RawMessageLine`. */
export type PrefetchRequest = { messageIds: Id<"messages">[]; format: BodyFormat };
export type RawMessageLine = { messageId: Id<"messages">; rawBase64: string };

/** `POST /v1/links`: a 5-minute capability URL for `<a download>` and share sheets. */
export type LinkRequest = { messageId: Id<"messages">; partId: string };
export type LinkResponse = { url: string; expiresAt: UnixMs };

/** `POST /v1/uploads` 201 body (raw body, `Content-Type`, URL-encoded `X-Filename`; at most 25 MB). */
export type UploadResponse = { uploadId: string; size: number; sha256: string };

export type DraftAttachment = { uploadId: string } | { fromMessageId: Id<"messages">; partId: string };

export type DraftContent = {
  accountId: Id<"mailAccounts">;
  /** Must be one of the account's identities. */
  identityEmail: string;
  to: Addr[];
  cc: Addr[];
  bcc: Addr[];
  subject: string;
  /** Composed HTML; the worker re-sanitizes. `null` for plain text. */
  html: string | null;
  /** Plain-text alternative, always present. */
  text: string;
  /** Sets In-Reply-To/References/threading. */
  inReplyToMessageId?: Id<"messages">;
  forwardOfMessageId?: Id<"messages">;
  attachments: DraftAttachment[];
  /** Honoured only if `prefs.readReceipts.enabled`. */
  readReceipt?: boolean;
};

/** `PUT /v1/drafts/{clientDraftId}` body. `baseVersion` 0 = new. 409 `draft_version_conflict` carries `version`. */
export type DraftPut = { baseVersion: number; draft: DraftContent; deviceId: string };
export type DraftSaved = {
  draftId: Id<"drafts">;
  version: number;
  providerDraftId: string;
  rfcMessageId: string;
};

/** `POST /v1/drafts/{clientDraftId}/adopt`: opens a provider-native draft for editing. */
export type AdoptRequest = { messageId: Id<"messages"> };
export type AdoptResponse = { draft: DraftContent; version: number };

/** `POST /v1/send` body. `clientSendId` is the idempotency key (UUIDv7). */
export type SendRequest = {
  clientSendId: string;
  clientDraftId: string;
  baseVersion: number;
  /** Final content; saved before scheduling. */
  draft?: DraftContent;
  /** Absent = now + undoSeconds. */
  sendAt?: UnixMs;
  /** `prefs.undoSendSeconds`. */
  undoSeconds: number;
  deviceId: string;
};
export type SendAccepted = {
  sendId: Id<"scheduledSends">;
  sendAt: UnixMs;
  state: "scheduled" | "due" | "cancelled";
};

/** `GET /v1/search?accountId&q&cursor`: provider-side search; hits are upserted into Convex first. */
export type SearchQuery = { accountId: Id<"mailAccounts">; q: string; cursor?: string };
export type SearchResponse = { threadIds: Id<"threads">[]; nextCursor: string | null };

/** `GET /healthz`. */
export type Health = { ok: true };

/** Names of the fixture types in fixtures/worker/*.json. */
export type WorkerWireTypeName =
  | "ConnectStart"
  | "ConnectComplete"
  | "ConnectCompleteResponse"
  | "ImapDiscovery"
  | "ImapConnect"
  | "MessageBody"
  | "DraftPut"
  | "DraftSaved"
  | "SendRequest"
  | "SendAccepted";
