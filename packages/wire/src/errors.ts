// The error-code registry (ARCHITECTURE §15). Rust twin: crates/core::wire::ErrorCode. User-facing copy
// lives in packages/ui/src/errors.ts; the messages here are the safe, generic English defaults servers send.
//
// - Convex functions throw `ConvexError<{ code, message, retryable?, retryAfterMs? }>`.
// - HTTP services return `{ "error": { "code", "message", "retryable", "retryAfterMs"? } }`.
// - AASP sends `event: error` with the same object.
// `message` never contains provider response text or user data.

export interface ErrorCodeInfo {
  /** HTTP status, or `null` for codes that never travel over HTTP on their own ("—" in §15). */
  http: number | null;
  retryable: boolean;
  /** Raised on the client before any request (AI resolution and consent states, D-121). */
  clientOnly: boolean;
  /** What the code means (developer-facing). */
  meaning: string;
  /** Safe default English message (developer-facing default; UIs use packages/ui copy). */
  message: string;
}

const e = (
  http: number | null,
  retryable: boolean,
  meaning: string,
  message: string,
  clientOnly = false,
): ErrorCodeInfo => ({ http, retryable, clientOnly, meaning, message });

export const ERROR_CODES = {
  unauthenticated: e(401, false, "missing, invalid or expired JWT", "Sign in again."),
  not_found: e(404, false, "missing or not owned", "Not found."),
  not_entitled: e(402, false, "no hostedSync / managedAi", "This needs an active Akatsuki Pro plan."),
  invalid_argument: e(400, false, "validation failed", "The request was not valid."),
  invalid_action: e(
    400,
    false,
    "action not allowed for the account's caps or label keys",
    "That action is not available for this account.",
  ),
  invalid_vars: e(422, false, "AI vars fail input.schema.json", "The AI request was not valid."),
  payload_too_large: e(413, false, "request too large", "The request is too large."),
  rate_limited: e(
    429,
    true,
    "too many requests; carries retryAfterMs",
    "Too many requests. Try again shortly.",
  ),
  ai_budget_exhausted: e(
    429,
    false,
    "monthly AI budget used up; resetsAt in message",
    "The monthly AI budget is used up.",
  ),
  ai_model_required: e(null, false, "client-side resolution failure: no model", "Choose a model.", true),
  consent_required: e(
    null,
    false,
    "client-side: AI used before consent.grantedAt (§12.7 rule 1)",
    "Turn on AI features in Settings first.",
    true,
  ),
  ai_off: e(null, false, "client-side: the AI feature is turned off", "This AI feature is turned off.", true),
  key_missing: e(
    null,
    false,
    "client-side: the BYO key is not stored on this device",
    "Key not set on this device.",
    true,
  ),
  prompt_version_mismatch: e(
    409,
    false,
    "client must update its prompt pack",
    "Update the app to use this feature.",
  ),
  already_connected: e(
    409,
    false,
    "mailbox linked to another Akatsuki user",
    "This mailbox is connected to another Akatsuki account.",
  ),
  already_subscribed: e(409, false, "already has a subscription", "You already have a subscription."),
  draft_version_conflict: e(
    409,
    false,
    "draft changed elsewhere; carries version",
    "This draft was changed on another device.",
  ),
  lease_lost: e(409, false, "worker-internal", "Another worker owns this account."),
  version_conflict: e(409, false, "secret compare-and-set failed", "The stored credentials changed."),
  imap_auth_failed: e(
    422,
    false,
    "IMAP LOGIN rejected",
    "The IMAP server rejected the username or password.",
  ),
  smtp_auth_failed: e(422, false, "SMTP AUTH rejected", "The SMTP server rejected the username or password."),
  tls_failed: e(422, false, "TLS handshake failed", "A secure connection could not be established."),
  unreachable: e(422, false, "host unreachable", "The server could not be reached."),
  reauth_required: e(409, false, "account credentials rejected", "Reconnect this account."),
  reauth_timeout: e(
    null,
    false,
    "op failed after 7 days in reauth",
    "The account was not reconnected in time.",
  ),
  provider_transient: e(
    502,
    true,
    "mail provider 5xx, timeout or reset",
    "The mail provider had a temporary problem.",
  ),
  provider_rate_limited: e(503, true, "mail provider rate limit", "The mail provider is limiting requests."),
  provider_invalid: e(422, false, "provider rejected the request", "The mail provider rejected the request."),
  provider_conflict: e(422, false, "provider reported a conflict", "The mail provider reported a conflict."),
  provider_quota: e(
    507,
    false,
    "mailbox full, send limit",
    "The mailbox is full or a sending limit was reached.",
  ),
  provider_unavailable: e(
    503,
    true,
    "an upstream service is unavailable: the AI provider, or a worker-side provider or OAuth client",
    "The service is unavailable.",
  ),
  unsupported: e(
    400,
    false,
    "not supported, e.g. delete_forever on Gmail",
    "This is not supported for this account.",
  ),
  draft_deleted: e(null, false, "send outcome: the draft was deleted", "The draft was deleted."),
  stale_schedule: e(
    null,
    false,
    "send outcome: send-later too far overdue",
    "The scheduled send was too far overdue.",
  ),
  send_unconfirmed: e(null, false, "send outcome: could not confirm", "We could not confirm this was sent."),
  internal: e(500, true, "bug; logged with a request id", "Something went wrong."),
  not_implemented: e(
    501,
    false,
    "route exists in the contract but is not built yet (D-091)",
    "This is not available yet.",
  ),
} as const satisfies Record<string, ErrorCodeInfo>;

export type ErrorCode = keyof typeof ERROR_CODES;

/** Every code, in registry order. */
export const ALL_ERROR_CODES = Object.keys(ERROR_CODES) as ErrorCode[];

export function isErrorCode(code: unknown): code is ErrorCode {
  return typeof code === "string" && Object.hasOwn(ERROR_CODES, code);
}

/**
 * The HTTP status for a code. Codes without one ("—") map to 400, like the Rust registry, so a misuse is
 * still a well-formed response.
 */
export function httpStatusFor(code: ErrorCode): number {
  return ERROR_CODES[code].http ?? 400;
}

/** The error object of the §15 envelope, the AASP `error` event and Convex error data. */
export interface ErrorPayload {
  /** A registry code; unknown codes from newer servers are kept as strings. */
  code: ErrorCode | (string & {});
  message: string;
  retryable: boolean;
  retryAfterMs?: number;
  /** `draft_version_conflict` carries the current version. */
  version?: number;
}

/** `{ "error": ErrorPayload }`. */
export interface ErrorBody {
  error: ErrorPayload;
}

/** Builds an envelope with the registry defaults (servers in TypeScript use this). */
export function errorBody(
  code: ErrorCode,
  extra: { message?: string; retryAfterMs?: number; version?: number } = {},
): ErrorBody {
  const info = ERROR_CODES[code];
  const error: ErrorPayload = { code, message: extra.message ?? info.message, retryable: info.retryable };
  if (extra.retryAfterMs !== undefined) error.retryAfterMs = Math.max(0, Math.round(extra.retryAfterMs));
  if (extra.version !== undefined) error.version = extra.version;
  return { error };
}

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;

/** Reads an `ErrorPayload` from an envelope, a bare payload or Convex error data; `null` if it is none. */
export function parseErrorPayload(value: unknown): ErrorPayload | null {
  const inner = isRecord(value) && isRecord(value.error) ? value.error : value;
  if (!isRecord(inner) || typeof inner.code !== "string") return null;
  const code = inner.code;
  const known = isErrorCode(code) ? ERROR_CODES[code] : undefined;
  const payload: ErrorPayload = {
    code,
    message: typeof inner.message === "string" ? inner.message : (known?.message ?? "Something went wrong."),
    retryable: typeof inner.retryable === "boolean" ? inner.retryable : (known?.retryable ?? false),
  };
  if (typeof inner.retryAfterMs === "number") payload.retryAfterMs = inner.retryAfterMs;
  if (typeof inner.version === "number") payload.version = inner.version;
  return payload;
}

/** An error from the worker, apps/api, Convex or a client-side check, with a §15 code. */
export class WireError extends Error {
  override name = "WireError";
  readonly code: ErrorCode | (string & {});
  readonly retryable: boolean;
  readonly retryAfterMs: number | undefined;
  readonly version: number | undefined;
  /** HTTP status when the error came from an HTTP response. */
  readonly status: number | undefined;

  constructor(payload: ErrorPayload, status?: number) {
    super(payload.message);
    this.code = payload.code;
    this.retryable = payload.retryable;
    this.retryAfterMs = payload.retryAfterMs;
    this.version = payload.version;
    this.status = status;
  }

  /** A client-side error with the registry defaults. */
  static of(code: ErrorCode, message?: string): WireError {
    return new WireError(errorBody(code, { message }).error);
  }

  toPayload(): ErrorPayload {
    const p: ErrorPayload = { code: this.code, message: this.message, retryable: this.retryable };
    if (this.retryAfterMs !== undefined) p.retryAfterMs = this.retryAfterMs;
    if (this.version !== undefined) p.version = this.version;
    return p;
  }
}

/**
 * Normalises anything thrown into an ErrorPayload: WireError, Convex `ConvexError` (its `data`) and objects
 * with a string `code` (e.g. `AiFeatureError`). Anything else becomes `internal`.
 */
export function toErrorPayload(err: unknown): ErrorPayload {
  if (err instanceof WireError) return err.toPayload();
  if (isRecord(err)) {
    const fromData = "data" in err ? parseErrorPayload(err.data) : null;
    if (fromData) return fromData;
    const direct = parseErrorPayload(err);
    if (direct) return direct;
  }
  return errorBody("internal").error;
}
