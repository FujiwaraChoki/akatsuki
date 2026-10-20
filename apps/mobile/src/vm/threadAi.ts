// Thread AI on mobile (ARCHITECTURE §12, review round 2 D-408): Summarize and Suggest replies under an open
// conversation, the two features web and desktop have there. Pure parts: whether a feature can run with the
// user's settings on this device (the same states and copy as web's `aiStatus`), and the transcript input built
// from the bodies the thread screen already holds. Nothing here talks to a network.
import {
  type AiFeature,
  type AiSettings,
  KEYLESS_KINDS,
  resolveFeature,
  type ThreadForTranscript,
} from "@akatsuki/ai";
import { splitQuoted } from "@akatsuki/wire";

export type ThreadAiFeature = "summarize" | "instant_reply";

export type ThreadAiStatus =
  | "ready"
  | "consent"
  | "off"
  | "no_model"
  | "no_key"
  | "not_entitled"
  | "no_api"
  | "no_dictation";

/** Provider kinds that can transcribe audio (packages/ai `transcriptionModelFor`, plus the managed provider). */
const TRANSCRIBING_KINDS: ReadonlySet<string> = new Set(["akatsuki", "openai", "openai_compatible"]);

/** The same sentences as the web client (apps/web/src/ai/useAi.ts STATUS_COPY). */
export const THREAD_AI_COPY: Record<Exclude<ThreadAiStatus, "ready">, string> = {
  consent: "AI is off. Turn it on in Settings, AI.",
  off: "This AI feature is turned off in Settings, AI.",
  no_model: "Choose a model for this feature in Settings, AI.",
  no_key: "Key not set on this device. Add it in Settings, AI.",
  not_entitled: "Akatsuki AI is part of Pro. Use your own key or a local model instead.",
  no_api: "Akatsuki AI is not configured in this build.",
  no_dictation:
    "This provider cannot transcribe audio. Choose Akatsuki AI, OpenAI or a Whisper server for Dictation in Settings, AI.",
};

export interface ThreadAiEnv {
  settings: AiSettings;
  /** The user has managed AI (Pro or trial). */
  managedAi: boolean;
  /** apps/api is configured in this build. */
  apiConfigured: boolean;
  /** A BYO key for the profile is stored on this device. */
  hasKey: (profileId: string) => boolean;
}

/** Whether `feature` can run, and through which profile. */
export function threadAiStatus(
  feature: ThreadAiFeature,
  env: ThreadAiEnv,
): { status: ThreadAiStatus; profileId?: string } {
  return aiFeatureStatus(feature, env);
}

/** The same check for any feature (compose, rewrite, nl_search, dictation …). */
export function aiFeatureStatus(
  feature: AiFeature,
  env: ThreadAiEnv,
): { status: ThreadAiStatus; profileId?: string } {
  if (!env.settings.consent.grantedAt) return { status: "consent" };
  const r = resolveFeature(feature, env.settings);
  if (r.status === "off") return { status: "off" };
  if (feature === "dictation") {
    const profile =
      r.status === "ok" ? r.profile : env.settings.profiles.find((p) => p.profileId === r.profileId);
    if (!profile || !TRANSCRIBING_KINDS.has(profile.kind)) return { status: "no_dictation" };
  }
  if (r.status === "error") return { status: "no_model" };
  if (r.managed) {
    if (!env.managedAi) return { status: "not_entitled" };
    return { status: env.apiConfigured ? "ready" : "no_api" };
  }
  const profileId = r.profile.profileId;
  if (!KEYLESS_KINDS.has(r.profile.kind) && !env.hasKey(profileId)) return { status: "no_key", profileId };
  return { status: "ready", profileId };
}

export interface AiMessageInput {
  from?: { name?: string; email: string } | null;
  to: { name?: string; email: string }[];
  cc: { name?: string; email: string }[];
  date: number;
  snippet: string;
}

export interface AiBodyInput {
  html: string | null;
  text: string | null;
  quoted: { htmlStart: number | null; textStart: number | null };
}

/** Visible text of a sanitized HTML body (tags dropped, entities for the common characters decoded). */
export function htmlToText(html: string): string {
  return html
    .replace(/<(style|script)\b[\s\S]*?<\/\1>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'")
    .replace(/&amp;/g, "&")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

/** The part of a body the reader sees (quoted history removed), else the snippet. */
export function visibleText(body: AiBodyInput | null | undefined, snippet: string): string {
  if (!body) return snippet;
  if (body.text !== null) return splitQuoted(body.text, body.quoted.textStart).visible.trim() || snippet;
  if (body.html !== null) return htmlToText(splitQuoted(body.html, body.quoted.htmlStart).visible) || snippet;
  return snippet;
}

/** The transcript input for packages/ai (`summarizeVars` / `instantReplyVars`). */
export function threadForAi(
  messages: readonly AiMessageInput[],
  bodyOf: (index: number) => AiBodyInput | null | undefined,
  timezone: string,
): ThreadForTranscript {
  return {
    timezone,
    messages: messages.map((m, i) => ({
      from: m.from ?? null,
      to: m.to,
      cc: m.cc,
      date: m.date,
      text: visibleText(bodyOf(i), m.snippet),
    })),
  } as ThreadForTranscript;
}
