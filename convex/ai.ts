// ai.* (ARCHITECTURE §4.2, §12). No keys and no message text are stored here (D-028).
import { v } from "convex/values";
import { internal } from "./_generated/api";
import type { Doc } from "./_generated/dataModel";
import { internalMutation, type QueryCtx } from "./_generated/server";
import { entitledMutation, getOwned, userMutation, userQuery } from "./lib/auth";
import { appError } from "./lib/errors";
import { project, vAiSettingsView, vTriageInput } from "./lib/rows";
import { recomputeThread } from "./lib/threads";
import { vAiFeature, vAiProviderKind, vTriageCategory } from "./lib/validators";

const vProfileIn = v.object({
  profileId: v.string(),
  label: v.string(),
  kind: vAiProviderKind,
  baseUrl: v.optional(v.string()),
  models: v.object({
    fast: v.optional(v.string()),
    default: v.optional(v.string()),
    heavy: v.optional(v.string()),
    transcription: v.optional(v.string()),
  }),
  keyOnDevices: v.array(v.string()),
});
const vRouteIn = v.object({
  feature: v.union(vAiFeature, v.literal("default")),
  profileId: v.string(),
  model: v.optional(v.string()),
  enabled: v.boolean(),
});
const vConsentIn = v.object({
  grantedAt: v.optional(v.number()),
  version: v.optional(v.number()),
  autoTriage: v.boolean(),
  syncVoiceProfile: v.boolean(),
});

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// The same rule as `isLocalHost`/`baseUrlProblem` in packages/ai/src/settings.ts and `is_local_host`/
// `base_url_problem` in crates/ai/src/settings.rs (D-129). All three run packages/ai/fixtures/base-url/cases.json.
// Hostnames come from WHATWG `URL`, so IPv4 literals are already normalised (`127.1` -> `127.0.0.1`).
const LOOPBACK_HOST = /^(localhost|127(?:\.\d{1,3}){3}|::1)$/i;
const PRIVATE_V4 = /^(10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(1[6-9]|2\d|3[01])(?:\.\d{1,3}){2})$/;
const MDNS = /\.local$/i;

/** Loopback (`localhost`, 127.0.0.0/8, `::1`), RFC 1918 IPv4 literals and mDNS `.local` names; nothing else. */
export function isLocalHost(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, "");
  return LOOPBACK_HOST.test(h) || PRIVATE_V4.test(h) || MDNS.test(h);
}

/** `https://…` or `http://` on localhost/LAN; never userinfo, a query string or a fragment (§3.2, §4.2). */
export function checkBaseUrl(raw: string): void {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw appError("invalid_argument", "The base URL is not a valid URL.");
  }
  if (url.username || url.password)
    throw appError("invalid_argument", "The base URL must not contain credentials.");
  if (url.search || raw.includes("?"))
    throw appError("invalid_argument", "The base URL must not contain a query string.");
  if (url.hash || raw.includes("#"))
    throw appError("invalid_argument", "The base URL must not contain a fragment.");
  if (url.protocol === "https:") return;
  if (url.protocol === "http:" && isLocalHost(url.hostname)) return;
  throw appError("invalid_argument", "Use https, or http only for localhost and LAN addresses.");
}

async function settingsOf(ctx: Pick<QueryCtx, "db">, userId: string) {
  return await ctx.db
    .query("aiSettings")
    .withIndex("by_user", (q) => q.eq("userId", userId))
    .unique();
}

export const getSettings = userQuery({
  args: {},
  returns: v.union(vAiSettingsView, v.null()),
  handler: async (ctx) => {
    const s = await settingsOf(ctx, ctx.userId);
    return s ? project(vAiSettingsView, s) : null;
  },
});

export const saveSettings = userMutation({
  args: {
    profiles: v.optional(v.array(vProfileIn)),
    routes: v.optional(v.array(vRouteIn)),
    consent: v.optional(vConsentIn),
    voiceProfile: v.optional(v.union(v.string(), v.null())),
  },
  returns: v.null(),
  handler: async (ctx, args) => {
    const existing = await settingsOf(ctx, ctx.userId);
    const profiles = args.profiles ?? existing?.profiles ?? [];
    const routes = args.routes ?? existing?.routes ?? [];
    const consent = args.consent ?? existing?.consent ?? { autoTriage: false, syncVoiceProfile: false };
    if (profiles.length > 10) throw appError("invalid_argument", "At most 10 AI profiles.");
    const ids = new Set<string>();
    for (const p of profiles) {
      if (!UUID_RE.test(p.profileId)) throw appError("invalid_argument", "profileId must be a UUID.");
      if (ids.has(p.profileId)) throw appError("invalid_argument", "Duplicate profileId.");
      ids.add(p.profileId);
      if (p.label.length === 0 || p.label.length > 60)
        throw appError("invalid_argument", "Profile labels are 1 to 60 characters.");
      // The managed provider receives the user's session JWT: it goes only to the client's configured apps/api,
      // never to a URL stored here (the same rule as zAiProfile and crates/ai validate_settings, D-415).
      if (p.kind === "akatsuki" && p.baseUrl !== undefined)
        throw appError("invalid_argument", "The managed provider has no base URL.");
      if (p.baseUrl !== undefined) checkBaseUrl(p.baseUrl);
      if (p.keyOnDevices.length > 50) throw appError("invalid_argument", "Too many devices.");
    }
    if (routes.length > 20) throw appError("invalid_argument", "Too many routes.");
    for (const r of routes) {
      if (!ids.has(r.profileId)) throw appError("invalid_argument", "A route points at an unknown profile.");
    }
    let voiceProfile = existing?.voiceProfile;
    if (args.voiceProfile !== undefined) {
      if (args.voiceProfile === null) voiceProfile = undefined;
      else {
        if (!consent.syncVoiceProfile) {
          throw appError("invalid_argument", "Turn on voice profile sync before saving a voice profile.");
        }
        if (args.voiceProfile.length > 2048)
          throw appError("invalid_argument", "The voice profile is limited to 2,048 characters.");
        voiceProfile = args.voiceProfile;
      }
    }
    if (!consent.syncVoiceProfile) voiceProfile = undefined;
    const doc = { userId: ctx.userId, profiles, routes, consent, voiceProfile, updatedAt: Date.now() };
    const triageWasOn = triageRouteKind(existing) !== null;
    if (existing) await ctx.db.replace(existing._id, doc);
    else await ctx.db.insert("aiSettings", doc);
    // Turning triage on makes the recent conversations that synced while it was off claimable (D-413).
    if (!triageWasOn && triageRouteKind(doc) !== null)
      await ctx.scheduler.runAfter(0, internal.ai.enableRecentTriage, { userId: ctx.userId });
    return null;
  },
});

/** Mail older than this is not triaged (§12.5, the same window as `worker.claimTriage`). */
export const TRIAGE_WINDOW_MS = 14 * 24 * 60 * 60 * 1000;
const ENABLE_BATCH = 200;

/**
 * Moves the user's threads with mail from the last 14 days from `triageState: "off"` to `"none"` once triage is
 * on (D-413), in batches of 200 (each batch leaves the `off` range, so the next one continues), at most 25 batches.
 * The claim paths (`ai.claimTriage`, `worker.claimTriage`) keep their own inbox and 14-day checks.
 */
export const enableRecentTriage = internalMutation({
  args: { userId: v.string(), round: v.optional(v.number()) },
  returns: v.null(),
  handler: async (ctx, { userId, round = 0 }) => {
    if (triageRouteKind(await settingsOf(ctx, userId)) === null) return null;
    const since = Date.now() - TRIAGE_WINDOW_MS;
    const batch = await ctx.db
      .query("threads")
      .withIndex("by_user_triage", (q) =>
        q.eq("userId", userId).eq("triageState", "off").gte("lastMessageAt", since),
      )
      .order("desc")
      .take(ENABLE_BATCH);
    for (const t of batch) await ctx.db.patch(t._id, { triageState: "none" });
    if (batch.length === ENABLE_BATCH && round < 24)
      await ctx.scheduler.runAfter(0, internal.ai.enableRecentTriage, { userId, round: round + 1 });
    return null;
  },
});

/** The profile kind that serves triage for this user, or null when triage is off (§12.3 step 1–2). */
export function triageRouteKind(
  s: Pick<Doc<"aiSettings">, "consent" | "routes" | "profiles"> | null,
): string | null {
  if (!s?.consent.grantedAt || !s.consent.autoTriage) return null;
  const route =
    s.routes.find((r) => r.feature === "triage" && r.enabled) ??
    s.routes.find((r) => r.feature === "default" && r.enabled);
  if (!route) return null;
  return s.profiles.find((p) => p.profileId === route.profileId)?.kind ?? null;
}

/** Triage lease (§12.5): a claimed batch that is not answered within it can be claimed again. */
export const TRIAGE_LEASE_MS = 2 * 60 * 1000;

/** The `vTriageInput` projection of a thread: headers and snippet only, never body text (§12.7 rule 2). */
export function triageInputOf(t: Doc<"threads">) {
  const fromEmail = t.lastFrom?.email;
  return {
    threadId: t._id,
    from: t.lastFrom,
    to: t.participants.filter((p) => p.email !== fromEmail).slice(0, 10),
    subject: t.subject,
    snippet: t.snippet,
    listId: t.listId,
    isBulk: t.isBulk,
    hasAttachments: t.hasAttachments,
  };
}

/** Whether a thread may be claimed for triage now: in the inbox, not being deleted, and not leased. */
export function triageClaimable(t: Doc<"threads">, now: number): boolean {
  if (t.deletedPending || !t.labelKeys.includes("sys:inbox")) return false;
  if (t.triageState === "none") return true;
  return t.triageState === "pending" && (t.triageLeaseUntil ?? 0) < now;
}

export const claimTriage = entitledMutation({
  args: { deviceId: v.string(), limit: v.number() },
  returns: v.array(vTriageInput),
  handler: async (ctx, { limit }) => {
    if (limit < 1 || limit > 25) throw appError("invalid_argument", "limit is 1 to 25.");
    const kind = triageRouteKind(await settingsOf(ctx, ctx.userId));
    if (kind === null || kind === "akatsuki") return []; // managed triage runs on the worker (§12.5)
    const now = Date.now();
    const candidates: Doc<"threads">[] = [];
    const fresh = await ctx.db
      .query("threads")
      .withIndex("by_user_triage", (q) => q.eq("userId", ctx.userId).eq("triageState", "none"))
      .order("desc")
      .take(limit * 4);
    const expired = await ctx.db
      .query("threads")
      .withIndex("by_user_triage", (q) => q.eq("userId", ctx.userId).eq("triageState", "pending"))
      .order("desc")
      .filter((q) => q.lt(q.field("triageLeaseUntil"), now))
      .take(limit);
    for (const t of [...fresh, ...expired]) {
      if (candidates.length >= limit) break;
      if (triageClaimable(t, now)) candidates.push(t);
    }
    const out = [];
    for (const t of candidates) {
      await ctx.db.patch(t._id, { triageState: "pending", triageLeaseUntil: now + TRIAGE_LEASE_MS });
      out.push(triageInputOf(t));
    }
    return out;
  },
});

export const vTriageResult = v.object({
  threadId: v.id("threads"),
  category: vTriageCategory,
  needsReply: v.boolean(),
  model: v.string(),
  promptVersion: v.number(),
});

export const setTriage = entitledMutation({
  args: { results: v.array(vTriageResult) },
  returns: v.null(),
  handler: async (ctx, { results }) => {
    if (results.length > 50) throw appError("invalid_argument", "At most 50 results.");
    const now = Date.now();
    for (const r of results) {
      await getOwned(ctx, r.threadId, ctx.userId);
      await ctx.db.patch(r.threadId, {
        triage: {
          category: r.category,
          needsReply: r.needsReply,
          source: "client",
          model: r.model.slice(0, 200),
          promptVersion: r.promptVersion,
          at: now,
        },
        triageState: "done",
        triageLeaseUntil: undefined,
      });
      await recomputeThread(ctx, r.threadId);
    }
    return null;
  },
});
