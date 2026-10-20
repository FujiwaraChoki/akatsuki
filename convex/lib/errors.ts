// Error helpers (ARCHITECTURE §15). Convex functions throw ConvexError<AppErrorData>.
import { ConvexError } from "convex/values";

export type ErrorCode =
  | "unauthenticated"
  | "not_found"
  | "not_entitled"
  | "invalid_argument"
  | "invalid_action"
  | "payload_too_large"
  | "rate_limited"
  | "ai_budget_exhausted"
  | "already_connected"
  | "already_subscribed"
  | "draft_version_conflict"
  | "lease_lost"
  | "version_conflict"
  | "reauth_required"
  | "reauth_timeout"
  | "unsupported"
  | "draft_deleted"
  | "stale_schedule"
  | "send_unconfirmed"
  | "internal";

export type AppErrorData = {
  code: ErrorCode;
  message: string;
  retryable?: boolean;
  retryAfterMs?: number;
};

const DEFAULT_MESSAGES: Record<ErrorCode, string> = {
  unauthenticated: "Sign in to continue.",
  not_found: "Not found.",
  not_entitled: "This needs an active Akatsuki Pro subscription.",
  invalid_argument: "Invalid argument.",
  invalid_action: "That action is not available for this account.",
  payload_too_large: "Too large.",
  rate_limited: "Too many requests. Try again shortly.",
  ai_budget_exhausted: "The monthly AI budget is used up.",
  already_connected: "This mailbox is already connected to another Akatsuki account.",
  already_subscribed: "Already subscribed.",
  draft_version_conflict: "This draft changed on another device.",
  lease_lost: "Lease lost.",
  version_conflict: "Version conflict.",
  reauth_required: "Reconnect this account.",
  reauth_timeout: "The account was not reconnected in time.",
  unsupported: "Not supported for this account.",
  draft_deleted: "The draft was deleted.",
  stale_schedule: "The scheduled send was too far overdue.",
  send_unconfirmed: "We could not confirm this was sent.",
  internal: "Something went wrong.",
};

const RETRYABLE: ReadonlySet<ErrorCode> = new Set(["rate_limited", "internal"]);

export function appError(
  code: ErrorCode,
  message?: string,
  extra?: { retryAfterMs?: number },
): ConvexError<AppErrorData> {
  const data: AppErrorData = {
    code,
    message: message ?? DEFAULT_MESSAGES[code],
    retryable: RETRYABLE.has(code),
  };
  if (extra?.retryAfterMs !== undefined) data.retryAfterMs = extra.retryAfterMs;
  return new ConvexError(data);
}

/** Plain AppError value (vAppError shape) for storing on rows. */
export function appErrorValue(code: ErrorCode, message?: string) {
  return { code, message: message ?? DEFAULT_MESSAGES[code], retryable: RETRYABLE.has(code) };
}

/**
 * Typed stub marker for logic that depends on code owned elsewhere. Throws a clear error naming the
 * directory that must provide the implementation.
 */
export class NotImplementedError extends Error {
  constructor(what: string, ownerDir: string) {
    super(`NotImplemented: ${what} (owner: ${ownerDir})`);
    this.name = "NotImplementedError";
  }
}

export function notImplemented(what: string, ownerDir: string): never {
  throw new NotImplementedError(what, ownerDir);
}
