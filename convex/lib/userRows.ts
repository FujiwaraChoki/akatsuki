// Rows every user has (ARCHITECTURE §4.2 me.bootstrap, §6.1 user.onCreate trigger).
import type { MutationCtx } from "../_generated/server";
import { billingDisabled, NO_ENTITLEMENT, selfhostEntitlement } from "../ee/entitlement";
import type { Prefs, Rule } from "./validators";

export const DEFAULT_PREFS: Prefs = {
  undoSendSeconds: 10,
  remoteImages: "block",
  readReceipts: { enabled: false },
  cacheBodiesOnDevice: true,
  density: "comfortable",
  theme: "system",
  keymap: "superhuman",
  unifiedInbox: true,
  notifications: "private",
};

export const DEFAULT_SPLITS: { name: string; rule: Rule; showCount: boolean }[] = [
  { name: "VIP", rule: { match: "all", conds: [{ f: "vip", eq: true }] }, showCount: true },
  { name: "News", rule: { match: "all", conds: [{ f: "is_bulk", eq: true }] }, showCount: false },
];

export const DEFAULT_TIMEZONE = "UTC";

export function isValidTimezone(tz: string): boolean {
  try {
    new Intl.DateTimeFormat("en-US", { timeZone: tz });
    return tz.length > 0 && tz.length <= 64;
  } catch {
    return false;
  }
}

/** Idempotent: creates profiles, billing (status none), aiSettings and default splits when missing. */
export async function createUserRows(
  ctx: MutationCtx,
  args: { userId: string; email: string; name?: string | null; timezone?: string },
): Promise<void> {
  const now = Date.now();
  const { userId } = args;
  const profile = await ctx.db
    .query("profiles")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
  if (!profile) {
    await ctx.db.insert("profiles", {
      userId,
      email: args.email.trim().toLowerCase(),
      name: args.name ?? undefined,
      timezone: args.timezone && isValidTimezone(args.timezone) ? args.timezone : DEFAULT_TIMEZONE,
      onboarding: billingDisabled() ? "connect" : "checkout",
      prefs: DEFAULT_PREFS,
      createdAt: now,
      updatedAt: now,
    });
    // Default splits only for a brand-new user, so deleted defaults do not come back.
    const existingSplit = await ctx.db
      .query("splits")
      .withIndex("by_user_order", (q) => q.eq("userId", userId))
      .first();
    if (!existingSplit) {
      let order = 0;
      for (const s of DEFAULT_SPLITS) await ctx.db.insert("splits", { userId, order: order++, ...s });
    }
  }
  const billing = await ctx.db
    .query("billing")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
  if (!billing) {
    await ctx.db.insert("billing", {
      userId,
      status: "none",
      cancelAtPeriodEnd: false,
      entitlement: billingDisabled() ? selfhostEntitlement() : NO_ENTITLEMENT,
      updatedAt: now,
    });
  }
  const ai = await ctx.db
    .query("aiSettings")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
  if (!ai) {
    await ctx.db.insert("aiSettings", {
      userId,
      profiles: [],
      routes: [],
      consent: { autoTriage: false, syncVoiceProfile: false },
      updatedAt: now,
    });
  }
}
