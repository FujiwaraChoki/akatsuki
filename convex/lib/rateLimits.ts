// Rate limits (ARCHITECTURE §13.4) with @convex-dev/rate-limiter (D-006).
import { DAY, HOUR, MINUTE, RateLimiter, SECOND } from "@convex-dev/rate-limiter";
import { components } from "../_generated/api";
import type { MutationCtx } from "../_generated/server";
import { appError } from "./errors";

export const rateLimiter = new RateLimiter(components.rateLimiter, {
  mailApply: { kind: "token bucket", rate: 20, period: SECOND, capacity: 100 },
  searchThreads: { kind: "token bucket", rate: 5, period: SECOND, capacity: 5 },
  labels: { kind: "token bucket", rate: 2, period: SECOND, capacity: 2 },
  managedAiMinute: { kind: "token bucket", rate: 30, period: MINUTE, capacity: 30 },
  managedAiDay: { kind: "fixed window", rate: 1000, period: 24 * HOUR },
  // Sign-in emails per recipient (keyed by a hash of the address, D-394).
  signInEmailHour: { kind: "fixed window", rate: 5, period: HOUR },
  signInEmailDay: { kind: "fixed window", rate: 20, period: DAY },
  // New rows a client can create without a provider round trip (D-403).
  deviceRegister: { kind: "fixed window", rate: 20, period: HOUR },
  sendTombstone: { kind: "token bucket", rate: 30, period: MINUTE, capacity: 60 },
});

export type RateLimitName =
  | "mailApply"
  | "searchThreads"
  | "labels"
  | "managedAiMinute"
  | "managedAiDay"
  | "deviceRegister"
  | "sendTombstone";

/** Consumes one token for `key` or throws `rate_limited` with `retryAfterMs`. */
export async function consume(ctx: MutationCtx, name: RateLimitName, key: string): Promise<void> {
  const r = await rateLimiter.limit(ctx, name, { key });
  if (!r.ok) throw appError("rate_limited", undefined, { retryAfterMs: Math.ceil(r.retryAfter) });
}
