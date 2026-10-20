// ee.service.* (ARCHITECTURE §4.3, §13): functions apps/api calls under its service identity.
// Stripe itself lives in apps/api (D-013); these functions only store and evaluate state.
import { calculateRateLimit, DAY, RateLimiter } from "@convex-dev/rate-limiter";
import { v } from "convex/values";
import { components, internal } from "../_generated/api";
import type { Doc } from "../_generated/dataModel";
import { internalMutation, type MutationCtx, type QueryCtx } from "../_generated/server";
import { apiMutation, apiQuery, loadEntitlement } from "../lib/auth";
import { storeEntitlement } from "../lib/billing";
import { appError } from "../lib/errors";
import { rateLimiter } from "../lib/rateLimits";
import { vAiFeature, vEntitlement } from "../lib/validators";
import schema from "../schema";
import { aiBudgetMicros, nextPeriodStart, periodOf } from "./entitlement";

const vTier = v.union(
  v.literal("fast"),
  v.literal("default"),
  v.literal("heavy"),
  v.literal("transcription"),
);

/** Managed dictation cap (§13.4): 120 minutes of audio per user per UTC day (D-343). */
export const TRANSCRIPTION_SECONDS_PER_DAY = 120 * 60;

/**
 * The daily transcription bucket: a fixed window of TRANSCRIPTION_SECONDS_PER_DAY tokens (one per audio
 * second) per user, aligned to UTC midnight (`start: 0`). Same component as lib/rateLimits.ts, own name.
 */
export const transcriptionLimiter = new RateLimiter(components.rateLimiter, {
  transcriptionDay: { kind: "fixed window", rate: TRANSCRIPTION_SECONDS_PER_DAY, period: DAY, start: 0 },
});

/** Hex sha256 of a lowercased, trimmed email: the trial ledger never holds an address (D-393). */
export async function emailHash(email: string): Promise<string> {
  const bytes = new TextEncoder().encode(email.trim().toLowerCase());
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(digest, (b) => b.toString(16).padStart(2, "0")).join("");
}

async function trialLedgerFor(ctx: Pick<QueryCtx, "db">, email: string) {
  const hash = await emailHash(email);
  return await ctx.db
    .query("trialLedger")
    .withIndex("by_email_hash", (q) => q.eq("emailHash", hash))
    .first();
}

/** Records that this person used the trial; kept when the account is deleted (D-393). */
export async function recordTrial(ctx: MutationCtx, email: string, at: number): Promise<void> {
  if (await trialLedgerFor(ctx, email)) return;
  await ctx.db.insert("trialLedger", { emailHash: await emailHash(email), trialUsedAt: at });
}

async function usageRow(ctx: MutationCtx, userId: string, period: string) {
  return await ctx.db
    .query("aiUsage")
    .withIndex("by_user_period", (q) => q.eq("userId", userId).eq("period", period))
    .unique();
}

export const aiPrecheck = apiMutation({
  args: {
    userId: v.string(),
    feature: vAiFeature,
    tier: vTier,
    estCostMicros: v.number(),
    /** Dictation only: the estimated length of the recording, checked against the daily cap. */
    audioSeconds: v.optional(v.number()),
  },
  returns: v.union(
    v.object({ ok: v.literal(true), remainingMicros: v.number() }),
    v.object({
      ok: v.literal(false),
      code: v.union(v.literal("not_entitled"), v.literal("rate_limited"), v.literal("ai_budget_exhausted")),
      retryAfterMs: v.optional(v.number()),
      /** Set when the daily transcription cap refused the request (`rate_limited`). */
      limit: v.optional(v.literal("transcription_day")),
    }),
  ),
  handler: async (ctx, { userId, feature, tier, estCostMicros, audioSeconds }) => {
    const ent = await loadEntitlement(ctx, userId);
    if (!ent.managedAi) return { ok: false as const, code: "not_entitled" as const };
    const now = Date.now();
    const used = (await usageRow(ctx, userId, periodOf(now)))?.costMicros ?? 0;
    const remaining = aiBudgetMicros(ent) - used;
    if (remaining <= 0 || remaining < Math.max(0, estCostMicros)) {
      return {
        ok: false as const,
        code: "ai_budget_exhausted" as const,
        retryAfterMs: nextPeriodStart(now) - now,
      };
    }
    if (feature === "dictation" || tier === "transcription") {
      // Checked, not consumed: aiRecordUsage takes the real length once the transcription is done.
      const count = Math.min(TRANSCRIPTION_SECONDS_PER_DAY, Math.max(1, Math.ceil(audioSeconds ?? 1)));
      const r = await transcriptionLimiter.check(ctx, "transcriptionDay", { key: userId, count });
      if (!r.ok)
        return {
          ok: false as const,
          code: "rate_limited" as const,
          retryAfterMs: Math.ceil(r.retryAfter),
          limit: "transcription_day" as const,
        };
    }
    for (const name of ["managedAiMinute", "managedAiDay"] as const) {
      const r = await rateLimiter.limit(ctx, name, { key: userId });
      if (!r.ok)
        return { ok: false as const, code: "rate_limited" as const, retryAfterMs: Math.ceil(r.retryAfter) };
    }
    return { ok: true as const, remainingMicros: remaining };
  },
});

export const aiRecordUsage = apiMutation({
  args: {
    userId: v.string(),
    feature: vAiFeature,
    model: v.string(),
    inputTokens: v.number(),
    outputTokens: v.number(),
    audioSeconds: v.number(),
    costMicros: v.number(),
  },
  returns: v.object({ remainingMicros: v.number() }),
  handler: async (ctx, args) => {
    if (
      [args.inputTokens, args.outputTokens, args.audioSeconds, args.costMicros].some(
        (n) => n < 0 || !Number.isFinite(n),
      )
    ) {
      throw appError("invalid_argument", "Usage values must be non-negative.");
    }
    const now = Date.now();
    const period = periodOf(now);
    const row = await usageRow(ctx, args.userId, period);
    const next = {
      costMicros: (row?.costMicros ?? 0) + Math.round(args.costMicros),
      requests: (row?.requests ?? 0) + 1,
      inputTokens: (row?.inputTokens ?? 0) + args.inputTokens,
      outputTokens: (row?.outputTokens ?? 0) + args.outputTokens,
      audioSeconds: (row?.audioSeconds ?? 0) + args.audioSeconds,
      updatedAt: now,
    };
    if (row) await ctx.db.patch(row._id, next);
    else await ctx.db.insert("aiUsage", { userId: args.userId, period, ...next });
    if (args.feature === "dictation" && args.audioSeconds > 0) await takeTranscriptionSeconds(ctx, args);
    const ent = await loadEntitlement(ctx, args.userId);
    return { remainingMicros: Math.max(0, aiBudgetMicros(ent) - next.costMicros) };
  },
});

/**
 * Takes the recording's real length from today's transcription bucket, at most what is left: one recording
 * that runs past the cap (the precheck only had an estimate) is not carried into the next day (D-343).
 */
async function takeTranscriptionSeconds(ctx: MutationCtx, args: { userId: string; audioSeconds: number }) {
  const state = await transcriptionLimiter.getValue(ctx, "transcriptionDay", { key: args.userId });
  const left = Math.floor(calculateRateLimit(state, state.config, Date.now(), 0).value);
  const count = Math.min(Math.ceil(args.audioSeconds), left);
  if (count > 0) await transcriptionLimiter.limit(ctx, "transcriptionDay", { key: args.userId, count });
}

export const billingIdentity = apiQuery({
  args: { userId: v.string() },
  returns: v.object({
    email: v.string(),
    stripeCustomerId: v.optional(v.string()),
    trialUsedAt: v.optional(v.number()),
    entitlement: vEntitlement,
  }),
  handler: async (ctx, { userId }) => {
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (!profile) throw appError("not_found");
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    // A person who deleted their account and signed up again keeps their used trial (D-393).
    const ledger = billing?.trialUsedAt === undefined ? await trialLedgerFor(ctx, profile.email) : null;
    return {
      email: profile.email,
      stripeCustomerId: billing?.stripeCustomerId,
      trialUsedAt: billing?.trialUsedAt ?? ledger?.trialUsedAt,
      entitlement: await loadEntitlement(ctx, userId),
    };
  },
});

export const linkStripeCustomer = apiMutation({
  args: { userId: v.string(), stripeCustomerId: v.string() },
  returns: v.null(),
  handler: async (ctx, { userId, stripeCustomerId }) => {
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    if (!billing) throw appError("not_found");
    const other = await ctx.db
      .query("billing")
      .withIndex("by_customer", (q) => q.eq("stripeCustomerId", stripeCustomerId))
      .first();
    if (other && other._id !== billing._id)
      throw appError("invalid_argument", "Customer belongs to another user.");
    await ctx.db.patch(billing._id, { stripeCustomerId, updatedAt: Date.now() });
    return null;
  },
});

export const recordStripeEvent = apiMutation({
  args: { eventId: v.string(), type: v.string() },
  returns: v.object({ duplicate: v.boolean() }),
  handler: async (ctx, { eventId, type }) => {
    const seen = await ctx.db
      .query("stripeEvents")
      .withIndex("by_event", (q) => q.eq("eventId", eventId))
      .first();
    if (seen) return { duplicate: true };
    await ctx.db.insert("stripeEvents", { eventId, type, processedAt: Date.now() });
    return { duplicate: false };
  },
});

const billingStatus = schema.tables.billing.validator.fields.status;
export const vStripeSubSnapshot = v.object({
  id: v.string(),
  status: billingStatus,
  priceId: v.string(),
  trialEnd: v.optional(v.number()),
  currentPeriodEnd: v.optional(v.number()),
  cancelAtPeriodEnd: v.boolean(),
});

export const applyStripeSubscription = apiMutation({
  args: {
    stripeCustomerId: v.string(),
    userId: v.optional(v.string()),
    snapshot: v.union(vStripeSubSnapshot, v.null()),
    fetchedAt: v.number(),
  },
  returns: v.null(),
  handler: async (ctx, { stripeCustomerId, userId, snapshot, fetchedAt }) => {
    let billing: Doc<"billing"> | null = await ctx.db
      .query("billing")
      .withIndex("by_customer", (q) => q.eq("stripeCustomerId", stripeCustomerId))
      .first();
    if (!billing && userId) {
      billing = await ctx.db
        .query("billing")
        .withIndex("by_user", (q) => q.eq("userId", userId))
        .unique();
      if (billing) await ctx.db.patch(billing._id, { stripeCustomerId });
    }
    if (!billing) throw appError("not_found");
    // Full snapshots replace stored state, so event order does not matter; older snapshots lose.
    if (billing.snapshotFetchedAt !== undefined && fetchedAt < billing.snapshotFetchedAt) return null;
    const now = Date.now();
    const status = snapshot ? snapshot.status : billing.status === "none" ? "none" : "canceled";
    const next: Doc<"billing"> = {
      ...billing,
      stripeCustomerId,
      stripeSubscriptionId: snapshot?.id ?? billing.stripeSubscriptionId,
      status,
      priceId: snapshot?.priceId ?? billing.priceId,
      trialEnd: snapshot?.trialEnd,
      currentPeriodEnd: snapshot?.currentPeriodEnd,
      cancelAtPeriodEnd: snapshot?.cancelAtPeriodEnd ?? false,
      pastDueSince: status === "past_due" ? (billing.pastDueSince ?? now) : undefined,
      trialUsedAt: billing.trialUsedAt ?? (status === "trialing" || status === "active" ? now : undefined),
      snapshotFetchedAt: fetchedAt,
      updatedAt: now,
    };
    const { _id, _creationTime, ...fields } = next;
    await ctx.db.replace(_id, fields);
    await storeEntitlement(ctx, next, now);
    if (next.trialUsedAt !== undefined && billing.trialUsedAt === undefined) {
      const profile = await ctx.db
        .query("profiles")
        .withIndex("by_user", (q) => q.eq("userId", next.userId))
        .unique();
      if (profile) await recordTrial(ctx, profile.email, next.trialUsedAt);
    }
    // Onboarding moves on once the user is entitled.
    if (next.status === "trialing" || next.status === "active") {
      const profile = await ctx.db
        .query("profiles")
        .withIndex("by_user", (q) => q.eq("userId", next.userId))
        .unique();
      if (profile?.onboarding === "checkout")
        await ctx.db.patch(profile._id, { onboarding: "connect", updatedAt: now });
    }
    return null;
  },
});

export const trialWillEnd = apiMutation({
  args: { stripeCustomerId: v.string() },
  returns: v.null(),
  handler: async (ctx, { stripeCustomerId }) => {
    const billing = await ctx.db
      .query("billing")
      .withIndex("by_customer", (q) => q.eq("stripeCustomerId", stripeCustomerId))
      .first();
    if (!billing) return null;
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", billing.userId))
      .unique();
    if (!profile?.email) return null;
    const endsOn = billing.trialEnd
      ? new Date(billing.trialEnd).toLocaleDateString("en-US", {
          timeZone: profile.timezone,
          dateStyle: "long",
        })
      : "soon";
    await ctx.scheduler.runAfter(0, internal.email.send, {
      template: "trial_will_end",
      to: profile.email,
      vars: { endsOn: `on ${endsOn}`, url: `${process.env.SITE_URL ?? "https://app.akatsuki.vc"}/billing` },
    });
    return null;
  },
});

/** Cron (daily): delete processed Stripe events older than 30 days. */
export const gcStripeEvents = internalMutation({
  args: {},
  returns: v.null(),
  handler: async (ctx) => {
    const rows = await ctx.db
      .query("stripeEvents")
      .withIndex("by_processed", (q) => q.lt("processedAt", Date.now() - 30 * 24 * 60 * 60 * 1000))
      .take(1000);
    for (const r of rows) await ctx.db.delete(r._id);
    return null;
  },
});
