// Typed clients for the sync worker (§7.2) and apps/api (§8). Every call sends the user's JWT and throws
// `WireError` with a §15 code on failure. Web and mobile use these instead of hand-written fetches.
import { type AaspEvent, parseAaspStream } from "./aasp.ts";
import type {
  AiFeatureRequest,
  CancelRequest,
  QuotaResponse,
  StreamedAiFeature,
  TranscribeRequest,
  TranscribeResponse,
  UrlResponse,
} from "./api.ts";
import { WireError } from "./errors.ts";
import { createHttpClient, type HttpClientOptions, readNdjson, seg } from "./http.ts";
import type { Id } from "./ids.ts";
import type {
  AdoptResponse,
  ConnectComplete,
  ConnectCompleteResponse,
  ConnectStart,
  ConnectStartResponse,
  DraftPut,
  DraftSaved,
  ImapConnect,
  ImapConnectResponse,
  ImapDiscovery,
  LinkResponse,
  MessageBody,
  SearchQuery,
  SearchResponse,
  SendAccepted,
  SendRequest,
  UploadResponse,
} from "./worker.ts";

export interface WorkerClient {
  connectStart(provider: "google" | "microsoft", body: ConnectStart): Promise<ConnectStartResponse>;
  /** Redeems the `grant` the OAuth callback put in `returnTo` (D-395). */
  connectComplete(body: ConnectComplete): Promise<ConnectCompleteResponse>;
  connectImap(body: ImapConnect): Promise<ImapConnectResponse>;
  /** IMAP/SMTP settings for an address (D-396); throws `not_found` when nothing is known. */
  discoverImap(email: string, opts?: { signal?: AbortSignal }): Promise<ImapDiscovery>;
  /** Manual refresh (10 s debounce on the worker). */
  syncAccount(accountId: Id<"mailAccounts">): Promise<void>;
  /** One sanitized body. */
  getMessage(messageId: Id<"messages">, opts?: { signal?: AbortSignal }): Promise<MessageBody>;
  /** The raw `message/rfc822` stream (desktop only; web and mobile never need it). */
  getRawMessage(messageId: Id<"messages">, opts?: { signal?: AbortSignal }): Promise<Response>;
  /** All bodies of a thread, newest first, as the worker fetches them. */
  threadBodies(threadId: Id<"threads">, opts?: { signal?: AbortSignal }): AsyncGenerator<MessageBody>;
  /** Up to 100 sanitized bodies (mobile prefetch on Wi-Fi, §16.2). */
  prefetch(messageIds: Id<"messages">[], opts?: { signal?: AbortSignal }): AsyncGenerator<MessageBody>;
  /** An attachment or inline part (bytes, `Range` optional). */
  getPart(
    messageId: Id<"messages">,
    partId: string,
    opts?: { range?: string; signal?: AbortSignal },
  ): Promise<Response>;
  /** A 5-minute capability link for `<a download>` / share sheets; `url` is made absolute. */
  createLink(messageId: Id<"messages">, partId: string): Promise<LinkResponse>;
  /** Uploads one attachment (at most 25 MB) to the encrypted spool (24 h). */
  /** `contentType` overrides `file.type` (React Native file blobs often carry none). */
  upload(
    file: Blob,
    filename: string,
    opts?: { signal?: AbortSignal; contentType?: string },
  ): Promise<UploadResponse>;
  /** Autosave; throws `WireError` `draft_version_conflict` (with `version`) on a stale `baseVersion`. */
  putDraft(clientDraftId: string, body: DraftPut): Promise<DraftSaved>;
  adoptDraft(clientDraftId: string, messageId: Id<"messages">): Promise<AdoptResponse>;
  deleteDraft(clientDraftId: string): Promise<void>;
  send(body: SendRequest): Promise<SendAccepted>;
  /** Provider-side search ("Search the full mailbox", §11.2 backend 2). */
  search(q: SearchQuery, opts?: { signal?: AbortSignal }): Promise<SearchResponse>;
}

export function createWorkerClient(opts: HttpClientOptions): WorkerClient {
  const http = createHttpClient(opts);
  const empty = async (p: Promise<Response>) => {
    await (await p).body?.cancel();
  };
  const ndjson = async function* (p: Promise<Response>): AsyncGenerator<MessageBody> {
    const res = await p;
    if (!res.body) return;
    yield* readNdjson<MessageBody>(res.body);
  };
  return {
    connectStart: (provider, body) => http.json("POST", `/v1/connect/${seg(provider)}/start`, { json: body }),
    connectComplete: (body) => http.json("POST", "/v1/connect/complete", { json: body }),
    connectImap: (body) => http.json("POST", "/v1/connect/imap", { json: body }),
    discoverImap: (email, o) =>
      http.json("GET", "/v1/connect/imap/discover", { query: { email }, signal: o?.signal }),
    syncAccount: (accountId) => empty(http.request("POST", `/v1/accounts/${seg(accountId)}/sync`)),
    getMessage: (messageId, o) =>
      http.json("GET", `/v1/messages/${seg(messageId)}`, {
        query: { format: "sanitized" },
        signal: o?.signal,
      }),
    getRawMessage: (messageId, o) =>
      http.request("GET", `/v1/messages/${seg(messageId)}`, { query: { format: "raw" }, signal: o?.signal }),
    threadBodies: (threadId, o) =>
      ndjson(http.request("GET", `/v1/threads/${seg(threadId)}/bodies`, { signal: o?.signal })),
    prefetch(messageIds, o) {
      if (messageIds.length > 100)
        throw WireError.of("invalid_argument", "At most 100 messages per prefetch.");
      return ndjson(
        http.request("POST", "/v1/prefetch", {
          json: { messageIds, format: "sanitized" },
          signal: o?.signal,
        }),
      );
    },
    getPart: (messageId, partId, o) =>
      http.request("GET", `/v1/messages/${seg(messageId)}/parts/${seg(partId)}`, {
        headers: o?.range ? { range: o.range } : undefined,
        signal: o?.signal,
      }),
    async createLink(messageId, partId) {
      const r = await http.json<LinkResponse>("POST", "/v1/links", { json: { messageId, partId } });
      return { ...r, url: http.url(r.url) };
    },
    upload: (file, filename, o) =>
      http.json("POST", "/v1/uploads", {
        body: file,
        headers: {
          "content-type": o?.contentType || file.type || "application/octet-stream",
          "x-filename": encodeURIComponent(filename),
        },
        signal: o?.signal,
      }),
    putDraft: (clientDraftId, body) => http.json("PUT", `/v1/drafts/${seg(clientDraftId)}`, { json: body }),
    adoptDraft: (clientDraftId, messageId) =>
      http.json("POST", `/v1/drafts/${seg(clientDraftId)}/adopt`, { json: { messageId } }),
    deleteDraft: (clientDraftId) => empty(http.request("DELETE", `/v1/drafts/${seg(clientDraftId)}`)),
    send: (body) => http.json("POST", "/v1/send", { json: body }),
    search: (q, o) =>
      http.json("GET", "/v1/search", {
        query: { accountId: q.accountId, q: q.q, cursor: q.cursor },
        signal: o?.signal,
      }),
  };
}

export interface ApiClient {
  /** Managed AI (`POST /v1/ai/{feature}`): AASP events. Errors before the stream throw `WireError`. */
  streamFeature(
    feature: StreamedAiFeature,
    body: AiFeatureRequest,
    opts?: { signal?: AbortSignal },
  ): AsyncGenerator<AaspEvent>;
  /** Managed dictation. Not zero-data-retention (D-128): the UI must say audio goes to OpenAI. */
  transcribe(req: TranscribeRequest, opts?: { signal?: AbortSignal }): Promise<TranscribeResponse>;
  quota(): Promise<QuotaResponse>;
  checkout(): Promise<UrlResponse>;
  portal(): Promise<UrlResponse>;
  cancel(body: CancelRequest): Promise<void>;
}

export function createApiClient(opts: HttpClientOptions): ApiClient {
  const http = createHttpClient(opts);
  return {
    async *streamFeature(feature, body, o) {
      const res = await http.request("POST", `/v1/ai/${seg(feature)}`, {
        json: body,
        headers: { accept: "text/event-stream" },
        signal: o?.signal,
      });
      if (!res.body) return;
      yield* parseAaspStream(res.body);
    },
    transcribe(req, o) {
      const form = new FormData();
      form.set("audio", req.audio);
      if (req.language !== undefined) form.set("language", req.language);
      if (req.prompt !== undefined) form.set("prompt", req.prompt);
      return http.json("POST", "/v1/ai/transcribe", { body: form, signal: o?.signal });
    },
    quota: () => http.json("GET", "/v1/ai/quota"),
    checkout: () => http.json("POST", "/v1/billing/checkout", { json: {} }),
    portal: () => http.json("POST", "/v1/billing/portal", { json: {} }),
    async cancel(body) {
      await (await http.request("POST", "/v1/billing/cancel", { json: body })).body?.cancel();
    },
  };
}
