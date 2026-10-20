// Entitlement storage and lapse handling (ARCHITECTURE §13.3).
import { internal } from "../_generated/api";
import type { Doc } from "../_generated/dataModel";
import type { MutationCtx } from "../_generated/server";
import { computeEntitlement } from "../ee/entitlement";
import { appErrorValue } from "./errors";
import { recomputeThread } from "./threads";

const DAY = 24 * 60 * 60 * 1000;
export const PURGE_AFTER_MS = 30 * DAY;
export const PURGE_WARNING_AFTER_MS = 23 * DAY;

/** Fails queued ops of a user that lost hostedSync (§13.3 "Queued ops fail with not_entitled"). */
async function failQueuedOps(ctx: MutationCtx, userId: string, now: number) {
  const accounts = await ctx.db
    .query("mailAccounts")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .collect();
  for (const a of accounts) {
    const ops = await ctx.db
      .query("ops")
      .withIndex("by_account_state", (q) => q.eq("accountId", a._id).eq("state", "queued"))
      .take(500);
    const touched = new Set<string>();
    for (const op of ops) {
      await ctx.db.patch(op._id, {
        state: "failed",
        completedAt: now,
        lastError: appErrorValue("not_entitled"),
      });
      if (op.threadId) touched.add(op.threadId);
    }
    for (const t of touched) await recomputeThread(ctx, t as Doc<"threads">["_id"]);
  }
}

/** Recomputes and stores the entitlement, schedules the next re-evaluation and handles lapse/return. */
export async function storeEntitlement(
  ctx: MutationCtx,
  billing: Doc<"billing">,
  now = Date.now(),
): Promise<void> {
  const entitlement = computeEntitlement(billing, now);
  const patch: Partial<Doc<"billing">> = { entitlement, updatedAt: now };

  if (billing.reevaluateFnId) await ctx.scheduler.cancel(billing.reevaluateFnId);
  patch.reevaluateFnId =
    entitlement.until > now && entitlement.until < Number.MAX_SAFE_INTEGER
      ? await ctx.scheduler.runAt(entitlement.until, internal.billing.reevaluate, { userId: billing.userId })
      : undefined;

  if (entitlement.state === "lapsed" && billing.lapsedAt === undefined) {
    patch.lapsedAt = now;
    patch.purgeFnId = await ctx.scheduler.runAt(now + PURGE_AFTER_MS, internal.billing.purgeLapsed, {
      userId: billing.userId,
    });
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", billing.userId))
      .unique();
    if (profile?.email) {
      await ctx.scheduler.runAt(now + PURGE_WARNING_AFTER_MS, internal.billing.purgeWarning, {
        userId: billing.userId,
      });
    }
    await failQueuedOps(ctx, billing.userId, now);
  } else if (entitlement.hostedSync && billing.lapsedAt !== undefined) {
    if (billing.purgeFnId) await ctx.scheduler.cancel(billing.purgeFnId);
    patch.lapsedAt = undefined;
    patch.purgeFnId = undefined;
  }
  await ctx.db.patch(billing._id, patch);
}
