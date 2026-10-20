// AI provider settings (ARCHITECTURE §12.2): the shape synced through Convex `aiSettings`. Keys are never
// part of it: a profile names where its key lives (`keyOnDevices` lists device ids whose keychain holds it,
// looked up by `profileId`), so this object is safe to sync, log-free and server-visible.
// schema/ai-settings.schema.json is generated from these schemas (`pnpm build`) for crates/ai::settings.
import { z } from "zod";
import { AI_FEATURES, type AiFeature, PROVIDER_KINDS } from "./routing.ts";

const LOCAL_HOSTS = /^(localhost|127(?:\.\d{1,3}){3}|\[::1\]|::1)$/i;
const PRIVATE_V4 = /^(10(?:\.\d{1,3}){3}|192\.168(?:\.\d{1,3}){2}|172\.(1[6-9]|2\d|3[01])(?:\.\d{1,3}){2})$/;
const MDNS = /\.local$/i;

/** Loopback only: `localhost`, a full 127.0.0.0/8 literal or `::1` ("Stays on this device"). */
export function isLoopbackHost(hostname: string): boolean {
  return LOCAL_HOSTS.test(hostname) || LOCAL_HOSTS.test(hostname.replace(/^\[|\]$/g, ""));
}

/** An RFC 1918 IPv4 literal or an mDNS `.local` name ("Stays on your local network"). */
export function isPrivateNetworkHost(hostname: string): boolean {
  const h = hostname.replace(/^\[|\]$/g, "");
  return PRIVATE_V4.test(h) || MDNS.test(h);
}

/**
 * True for loopback, RFC 1918 and `.local` hosts (where plain http is allowed). The same rule as
 * `isLocalHost` in convex/ai.ts and `is_local_host` in crates/ai; all three run fixtures/base-url/cases.json (D-129).
 */
export function isLocalHost(hostname: string): boolean {
  return isLoopbackHost(hostname) || isPrivateNetworkHost(hostname);
}

/**
 * Checks a profile `baseUrl`: https anywhere, http only for local hosts; no userinfo, query or fragment
 * (same rule as `ai.saveSettings`). Returns a problem string, or null when valid.
 */
export function baseUrlProblem(value: string): string | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return "must be an absolute URL";
  }
  if (url.username || url.password) return "must not contain a user name or password";
  if (url.search || value.includes("?")) return "must not contain a query string";
  if (url.hash || value.includes("#")) return "must not contain a fragment";
  if (url.protocol === "https:") return null;
  if (url.protocol === "http:" && isLocalHost(url.hostname)) return null;
  return "must use https (http is allowed only for localhost and private network addresses)";
}

export const zProviderKind = z.enum(PROVIDER_KINDS);
export const zAiFeature = z.enum(AI_FEATURES);
const zModelId = z.string().min(1).max(200);

export const zTierModels = z.strictObject({
  fast: zModelId.optional(),
  default: zModelId.optional(),
  heavy: zModelId.optional(),
  transcription: zModelId.optional(),
});

export const zAiProfile = z
  .strictObject({
    profileId: z
      .string()
      .min(1)
      .max(64)
      .regex(/^[A-Za-z0-9-]+$/, "letters, digits and hyphens only"),
    label: z.string().min(1).max(80),
    kind: zProviderKind,
    baseUrl: z
      .string()
      .max(500)
      .superRefine((v, ctx) => {
        const problem = baseUrlProblem(v);
        if (problem) ctx.addIssue({ code: "custom", message: `baseUrl ${problem}` });
      })
      .optional(),
    models: zTierModels,
    /** deviceIds whose keychain / secure store holds this profile's key. Never the key itself. */
    keyOnDevices: z.array(z.string().min(1).max(64)).max(50),
  })
  .superRefine((p, ctx) => {
    if (p.kind === "openai_compatible" && !p.baseUrl)
      ctx.addIssue({
        code: "custom",
        path: ["baseUrl"],
        message: "openai_compatible profiles need a baseUrl",
      });
    if (p.kind === "akatsuki" && p.baseUrl)
      ctx.addIssue({ code: "custom", path: ["baseUrl"], message: "the managed provider has no baseUrl" });
  });

export const zAiRoute = z.strictObject({
  feature: z.union([zAiFeature, z.literal("default")]),
  profileId: z.string().min(1).max(64),
  model: zModelId.optional(),
  enabled: z.boolean(),
});

export const zAiConsent = z.strictObject({
  grantedAt: z.number().int().nonnegative().optional(),
  version: z.number().int().nonnegative().optional(),
  autoTriage: z.boolean(),
  syncVoiceProfile: z.boolean(),
});

export const zAiSettings = z
  .strictObject({
    profiles: z.array(zAiProfile).max(10),
    routes: z.array(zAiRoute).max(20),
    consent: zAiConsent,
    voiceProfile: z.string().max(2048).optional(),
    updatedAt: z.number().optional(),
    // Convex document fields are tolerated so `ai.getSettings` results parse directly.
    _id: z.string().optional(),
    _creationTime: z.number().optional(),
  })
  .superRefine((s, ctx) => {
    const ids = new Set<string>();
    s.profiles.forEach((p, i) => {
      if (ids.has(p.profileId))
        ctx.addIssue({ code: "custom", path: ["profiles", i, "profileId"], message: "duplicate profileId" });
      ids.add(p.profileId);
    });
    s.routes.forEach((r, i) => {
      if (!ids.has(r.profileId))
        ctx.addIssue({ code: "custom", path: ["routes", i, "profileId"], message: "unknown profileId" });
    });
    if (s.voiceProfile !== undefined && !s.consent.syncVoiceProfile)
      ctx.addIssue({
        code: "custom",
        path: ["voiceProfile"],
        message: "voiceProfile is only synced with consent.syncVoiceProfile",
      });
  });

export type AiProfile = z.infer<typeof zAiProfile>;
export type AiRoute = z.infer<typeof zAiRoute>;
export type AiConsent = z.infer<typeof zAiConsent>;
export type AiSettings = z.infer<typeof zAiSettings>;

export function parseAiSettings(value: unknown): AiSettings {
  return zAiSettings.parse(value);
}

/** Settings for a new user: AI off until consent; no profiles. */
export const EMPTY_AI_SETTINGS: AiSettings = {
  profiles: [],
  routes: [],
  consent: { autoTriage: false, syncVoiceProfile: false },
};

/**
 * Where an AI action's data goes (§12.2):
 * - `device`: a loopback endpoint (Ollama by default, or `openai_compatible` / `ollama` on localhost);
 * - `network`: an RFC 1918 or `.local` endpoint, so data leaves the device but not the local network;
 * - `provider`: any BYO cloud provider;
 * - `akatsuki`: the managed text features (OpenRouter with zero data retention, §12.3);
 * - `akatsuki_openai`: managed dictation, which apps/api forwards to OpenAI's transcription API under
 *   OpenAI's standard data policy (no zero data retention, D-128).
 */
export type Egress = "device" | "network" | "provider" | "akatsuki" | "akatsuki_openai";

/** The feature an egress label is shown for; `"default"` is the settings row of the default route. */
export type EgressFeature = AiFeature | "default";

type EgressProfile = Pick<AiProfile, "kind" | "baseUrl">;

function endpointHost(baseUrl: string | undefined): URL | null {
  if (!baseUrl) return null;
  try {
    return new URL(baseUrl);
  } catch {
    return null;
  }
}

/** The egress label shown under every AI action and in settings (§12.2). Same rule as `egress` in crates/ai. */
export function egressFor(profile: EgressProfile, feature: EgressFeature): Egress {
  if (profile.kind === "akatsuki") return feature === "dictation" ? "akatsuki_openai" : "akatsuki";
  if (profile.kind !== "ollama" && profile.kind !== "openai_compatible") return "provider";
  const url = endpointHost(profile.baseUrl);
  // Ollama without a baseUrl talks to its default, http://127.0.0.1:11434.
  if (!url) return profile.kind === "ollama" && !profile.baseUrl ? "device" : "provider";
  if (isLoopbackHost(url.hostname)) return "device";
  if (isPrivateNetworkHost(url.hostname)) return "network";
  return "provider";
}

const DEFAULT_HOSTS: Record<string, string> = {
  openrouter: "openrouter.ai",
  anthropic: "api.anthropic.com",
  openai: "api.openai.com",
  google: "generativelanguage.googleapis.com",
  gateway: "ai-gateway.vercel.sh",
};

/** What a feature sends, for the "Leaves this device" label. */
const EGRESS_WHAT: Record<EgressFeature, string> = {
  default: "the mail you run AI on",
  summarize: "the thread you run this on",
  instant_reply: "the thread you run this on",
  extract_actions: "the thread you run this on",
  compose: "your instruction, your draft and the thread you reply to",
  rewrite: "the text you rewrite",
  triage: "the headers and snippets of new mail",
  nl_search: "your search and your label and split names",
  voice_profile: "samples of your sent mail",
  dictation: "your recording",
};

/** Managed dictation copy (D-128): names OpenAI and its abuse-monitoring retention. */
export const MANAGED_DICTATION_EGRESS_TEXT =
  "Sent to OpenAI through Akatsuki; may be kept up to 30 days for abuse monitoring";

/** UI copy for the egress label (§12.2). Byte-identical to `egress_text` in crates/ai (fixtures/egress). */
export function egressText(profile: EgressProfile, feature: EgressFeature): string {
  const egress = egressFor(profile, feature);
  if (egress === "device") return "Stays on this device";
  if (egress === "akatsuki") return "Sent to Akatsuki for processing, not stored";
  if (egress === "akatsuki_openai") return MANAGED_DICTATION_EGRESS_TEXT;
  const url = endpointHost(profile.baseUrl);
  if (egress === "network") return `Stays on your local network (${url!.host})`;
  const host = url ? url.host : (DEFAULT_HOSTS[profile.kind] ?? profile.kind);
  return `Leaves this device: ${EGRESS_WHAT[feature]}, sent to ${host}`;
}
