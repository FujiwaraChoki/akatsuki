// HTTP error bodies (ARCHITECTURE §15): { error: { code, message, retryable, retryAfterMs? } }.
// Messages are fixed English strings: never provider text or user data.
import type { Context } from "hono";
import type { ContentfulStatusCode } from "hono/utils/http-status";

export const ERRORS = {
  unauthenticated: [401, "Sign in to continue.", false],
  not_found: [404, "Not found.", false],
  not_entitled: [402, "This needs an active Akatsuki Pro subscription or trial.", false],
  invalid_argument: [400, "Invalid request.", false],
  invalid_vars: [422, "The input for this AI feature is not valid.", false],
  payload_too_large: [413, "The request is too large.", false],
  rate_limited: [429, "Too many requests. Try again shortly.", true],
  ai_budget_exhausted: [429, "This month's Akatsuki AI budget is used up.", false],
  prompt_version_mismatch: [409, "Update the app to use this AI feature.", false],
  already_subscribed: [409, "You already have an Akatsuki Pro subscription.", false],
  provider_unavailable: [503, "The AI provider is not available right now.", true],
  provider_invalid: [422, "The AI provider rejected the request.", false],
  internal: [500, "Something went wrong.", true],
} as const satisfies Record<string, readonly [number, string, boolean]>;

export type ApiErrorCode = keyof typeof ERRORS;

export function errorBody(code: ApiErrorCode, retryAfterMs?: number, detail?: string) {
  const [, message, retryable] = ERRORS[code];
  return {
    error: {
      code,
      message: detail ? `${message} ${detail}` : message,
      retryable,
      ...(retryAfterMs !== undefined ? { retryAfterMs: Math.max(0, Math.round(retryAfterMs)) } : {}),
    },
  };
}

export function sendError(c: Context, code: ApiErrorCode, retryAfterMs?: number, detail?: string) {
  c.set("errorCode", code);
  const res = c.json(errorBody(code, retryAfterMs, detail), ERRORS[code][0] as ContentfulStatusCode);
  if (retryAfterMs !== undefined && code === "rate_limited")
    res.headers.set("retry-after", String(Math.max(1, Math.ceil(retryAfterMs / 1000))));
  return res;
}
