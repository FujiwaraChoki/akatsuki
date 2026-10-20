// internal.notify.newMail (ARCHITECTURE §4.5, §14.2): Expo push notifications for new inbox mail.
// prefs.notifications: "off" sends nothing; "private" (the default, also when no profile exists yet)
// sends a count only ("New mail", "3 new messages"); "full" sends the sender's name (or address) and the
// subject through Expo's push service (exp.host) to Apple/Google, opt-in only (D-351). Nothing else about
// the message ever leaves Convex.
import { v } from "convex/values";
import { internal } from "./_generated/api";
import { internalAction, internalMutation, internalQuery } from "./_generated/server";
import { releasePushToken } from "./devices";
import { DEFAULT_PREFS } from "./lib/userRows";

const EXPO_PUSH_URL = "https://exp.host/--/api/v2/push/send";

export const pushTargets = internalQuery({
  args: { userId: v.string(), threadIds: v.array(v.id("threads")) },
  returns: v.object({
    mode: v.union(v.literal("off"), v.literal("private"), v.literal("full")),
    tokens: v.array(v.string()),
    items: v.array(v.object({ threadId: v.id("threads"), from: v.string(), subject: v.string() })),
  }),
  handler: async (ctx, { userId, threadIds }) => {
    const profile = await ctx.db
      .query("profiles")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .unique();
    const mode = profile?.prefs.notifications ?? DEFAULT_PREFS.notifications;
    const devices = await ctx.db
      .query("devices")
      .withIndex("by_user", (q) => q.eq("userId", userId))
      .take(50);
    const tokens = devices.flatMap((d) => (d.expoPushToken ? [d.expoPushToken] : []));
    const items = [];
    if (mode === "full") {
      for (const id of threadIds.slice(0, 20)) {
        const t = await ctx.db.get(id);
        if (t && t.userId === userId) {
          items.push({ threadId: id, from: t.lastFrom?.name ?? t.lastFrom?.email ?? "", subject: t.subject });
        }
      }
    }
    return { mode, tokens, items };
  },
});

export const newMail = internalAction({
  args: { userId: v.string(), threadIds: v.array(v.id("threads")) },
  returns: v.null(),
  handler: async (ctx, { userId, threadIds }) => {
    const { mode, tokens, items } = await ctx.runQuery(internal.notify.pushTargets, { userId, threadIds });
    if (mode === "off" || tokens.length === 0 || threadIds.length === 0) return null;
    const messages =
      mode === "full" && items.length > 0
        ? items.flatMap((it) =>
            tokens.map((to) => ({
              to,
              title: it.from || "New mail",
              body: it.subject,
              data: { threadId: it.threadId },
            })),
          )
        : tokens.map((to) => ({
            to,
            title: "Akatsuki",
            body: threadIds.length === 1 ? "New mail" : `${threadIds.length} new messages`,
          }));
    const gone = new Set<string>();
    for (let i = 0; i < messages.length; i += 100) {
      const batch = messages.slice(i, i + 100);
      const res = await fetch(EXPO_PUSH_URL, {
        method: "POST",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify(batch),
      });
      if (!res.ok) {
        console.warn(`expo push failed with HTTP ${res.status}`);
        continue;
      }
      for (const to of unregisteredTokens(batch, await res.json().catch(() => null))) gone.add(to);
    }
    if (gone.size > 0) await ctx.runMutation(internal.notify.dropPushTokens, { tokens: [...gone] });
    return null;
  },
});

/**
 * Tokens Expo reports as `DeviceNotRegistered` (the app was uninstalled or the token rotated). Tickets come back
 * in the order of the messages sent (docs.expo.dev push "push tickets").
 */
export function unregisteredTokens(batch: { to: string }[], body: unknown): string[] {
  const data = (body as { data?: unknown } | null)?.data;
  if (!Array.isArray(data)) return [];
  const out: string[] = [];
  data.forEach((ticket, i) => {
    const t = ticket as { status?: string; details?: { error?: string } } | null;
    const to = batch[i]?.to;
    if (to && t?.status === "error" && t.details?.error === "DeviceNotRegistered") out.push(to);
  });
  return out;
}

export const dropPushTokens = internalMutation({
  args: { tokens: v.array(v.string()) },
  returns: v.null(),
  handler: async (ctx, { tokens }) => {
    for (const token of tokens.slice(0, 500)) await releasePushToken(ctx, token);
    return null;
  },
});
