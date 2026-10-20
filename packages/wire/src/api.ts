// apps/api HTTP wire types (ARCHITECTURE §8). Serde twins: crates/core::wire::api.
import type { Id, UnixMs } from "./ids.ts";
import type { Addr } from "./worker.ts";

/** AI features (`vAiFeature`). `dictation` goes through `/v1/ai/transcribe`, never `/v1/ai/{feature}`. */
export type AiFeature =
  | "summarize"
  | "instant_reply"
  | "compose"
  | "rewrite"
  | "extract_actions"
  | "triage"
  | "nl_search"
  | "voice_profile"
  | "dictation";
export type StreamedAiFeature = Exclude<AiFeature, "dictation">;

/**
 * `POST /v1/ai/{feature}` body (at most 256 KB). Raw prompts are never accepted. Response: `text/event-stream`
 * (AASP). Errors before the stream: 401, `402 not_entitled`, `409 prompt_version_mismatch`,
 * `413 payload_too_large`, `422 invalid_vars`, `429 rate_limited | ai_budget_exhausted`.
 */
export type AiFeatureRequest = { promptVersion: number; vars: Record<string, unknown> };

/** `POST /v1/ai/transcribe` (multipart: `audio`, `language?`, `prompt?`) 200 body. */
export type TranscribeResponse = { text: string; durationSec: number; language: string | null };
export type TranscribeRequest = {
  /** webm/opus, m4a, wav or mp3; at most 25 MB and 10 minutes. */
  audio: Blob;
  /** BCP-47. */
  language?: string;
  /** Vocabulary hint, at most 500 characters. */
  prompt?: string;
};

/** `GET /v1/ai/quota`. `period` is `YYYY-MM` (UTC); money is micro-dollars. */
export type QuotaResponse = { period: string; usedMicros: number; limitMicros: number; resetsAt: UnixMs };

/** `POST /v1/billing/checkout` and `/portal` (body `{}`) response. Checkout: 409 `already_subscribed`. */
export type UrlResponse = { url: string };
/** `POST /v1/billing/cancel` body; 204. `immediately: true` only during account deletion. */
export type CancelRequest = { immediately: boolean };

/** Header-and-snippet input for triage (`vTriageInput`); never body text. */
export type TriageInput = {
  threadId: Id<"threads">;
  from?: Addr;
  to: Addr[];
  subject: string;
  snippet: string;
  listId?: string;
  isBulk: boolean;
  hasAttachments: boolean;
};
export type TriageCategory =
  | "important"
  | "personal"
  | "updates"
  | "newsletters"
  | "promotions"
  | "receipts"
  | "calendar"
  | "notifications"
  | "other";

/** `POST /internal/v1/triage` (worker service JWT only). At most 25 items. */
export type InternalTriageRequest = { userId: string; items: TriageInput[] };
export type InternalTriageResponse = {
  results: {
    threadId: Id<"threads">;
    category: TriageCategory;
    needsReply: boolean;
    model: string;
    promptVersion: number;
  }[];
};
