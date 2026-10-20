// Feature entry points for clients (web, mobile): resolve the user's route (§12.3), then either call the
// provider directly through the registry (BYO, local) or apps/api (managed). Both paths yield AASP events.
import { type FeatureId, getFeature, promptVersion } from "@akatsuki/prompts";
import type { LanguageModel } from "ai";
import type { AaspEvent } from "./aasp.ts";
import { createAkatsukiRegistry, MissingKeyError, type ProviderOptions } from "./providers.ts";
import { resolveFeature } from "./resolve.ts";
import { type EscalationHook, NO_ESCALATION, tableModel } from "./routing.ts";
import { AiFeatureError, collectFeature, type ModelChoice, streamFeature } from "./run.ts";
import type { AiSettings } from "./settings.ts";
import { parseAaspStream } from "./sse.ts";
import {
  charLength,
  renderTranscript,
  renderTriageItems,
  type ThreadForTranscript,
  type TriageInput,
} from "./transcript.ts";

export interface ManagedClient {
  /** apps/api base URL, e.g. https://api.akatsuki.vc */
  apiUrl: string;
  /** User JWT (`getAccessToken()` from packages/wire). */
  getAccessToken: () => Promise<string>;
  fetch?: typeof globalThis.fetch;
}

export interface FeatureContext {
  settings: AiSettings;
  /** The key for a BYO profile on this device (keychain, secure store, IndexedDB). */
  keyFor: (profileId: string) => string | undefined;
  managed?: ManagedClient;
  providerOptions?: ProviderOptions;
  escalation?: EscalationHook;
  abortSignal?: AbortSignal;
  /** Test seam: replaces the registry lookup. */
  languageModelFor?: (profileId: string, model: string) => LanguageModel;
}

/** Throws AiFeatureError for the client-side states: no consent, feature off, no model, no key. */
export function runFeature(feature: FeatureId, vars: unknown, ctx: FeatureContext): AsyncIterable<AaspEvent> {
  if (!ctx.settings.consent.grantedAt)
    throw new AiFeatureError("consent_required", "Turn on AI features in Settings first.");
  const r = resolveFeature(feature, ctx.settings);
  if (r.status === "off") throw new AiFeatureError("ai_off", "This AI feature is turned off.");
  if (r.status === "error") throw new AiFeatureError("ai_model_required", "Choose a model.");
  if (r.managed) {
    if (!ctx.managed) throw new AiFeatureError("ai_off", "Akatsuki AI is not configured on this client.");
    return managedStream(feature, vars, ctx.managed, ctx.abortSignal);
  }
  const { profile } = r;
  const lookup =
    ctx.languageModelFor ??
    (() => {
      const reg = createAkatsukiRegistry({
        profiles: [profile],
        keyFor: ctx.keyFor,
        options: ctx.providerOptions,
      });
      return (profileId: string, model: string) => reg.languageModel(profileId, model);
    })();
  const choose = (tier: ModelChoice["tier"], model: string): ModelChoice => ({
    model: lookup(profile.profileId, model),
    modelId: model,
    kind: profile.kind,
    tier,
  });
  let first: ModelChoice;
  try {
    first = choose(r.tier, r.model);
  } catch (e) {
    if (e instanceof MissingKeyError) throw new AiFeatureError("key_missing", "Key not set on this device.");
    throw e;
  }
  return streamFeature({
    feature,
    vars,
    choice: first,
    abortSignal: ctx.abortSignal,
    escalation: {
      hook: ctx.escalation ?? NO_ESCALATION,
      modelFor: (tier, model) => {
        const id = model ?? profile.models[tier] ?? tableModel(tier, profile.kind);
        return id ? choose(tier, id) : null;
      },
    },
  });
}

async function* managedStream(
  feature: FeatureId,
  vars: unknown,
  managed: ManagedClient,
  abortSignal?: AbortSignal,
): AsyncGenerator<AaspEvent> {
  const doFetch = managed.fetch ?? globalThis.fetch;
  const res = await doFetch(`${managed.apiUrl.replace(/\/+$/, "")}/v1/ai/${feature}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${await managed.getAccessToken()}`,
      "content-type": "application/json",
      accept: "text/event-stream",
    },
    body: JSON.stringify({ promptVersion: promptVersion(feature), vars }),
    signal: abortSignal,
  });
  if (!res.ok || !res.body) {
    let error = {
      code: "provider_unavailable",
      message: "Akatsuki AI is not available right now.",
      retryable: true,
    };
    try {
      const body = (await res.json()) as { error?: typeof error };
      if (body.error?.code) error = body.error;
    } catch {
      // keep the generic error
    }
    yield { event: "error", data: error };
    return;
  }
  yield* parseAaspStream(res.body);
}

// ───────────── named feature functions (collect the final result) ─────────────

const today = (now: Date, timeZone: string) =>
  new Intl.DateTimeFormat("en-CA", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" }).format(
    now,
  );

export interface ThreadInput {
  thread: ThreadForTranscript;
  userName: string;
  now?: Date;
}

export type Summary = {
  tldr: string;
  bullets: string[];
  asks: string[];
  deadlines: { text: string; due: string | null }[];
};

/**
 * Characters left for `{{transcript}}` once the other vars are counted: `meta.maxInputChars` minus the
 * length of every other string var minus TRANSCRIPT_BUDGET_MARGIN, never below 0. The cap is on the sum of
 * all string vars (packages/prompts README), so a transcript rendered at the full cap could never be sent.
 * Twin: `crates/ai::transcript::transcript_budget` / `PromptPack::transcript_budget`.
 */
export function transcriptBudget(feature: FeatureId, otherVars: Record<string, unknown>): number {
  let used = 0;
  for (const [k, v] of Object.entries(otherVars))
    if (k !== "transcript" && typeof v === "string") used += charLength(v);
  return Math.max(0, getFeature(feature).meta.maxInputChars - used - TRANSCRIPT_BUDGET_MARGIN);
}

/** Head-room kept below the cap (covers the 4-character cut marker on a tiny budget). */
export const TRANSCRIPT_BUDGET_MARGIN = 200;

/** Renders the thread into `transcript` with whatever budget the other vars leave. */
function withTranscript<V extends Record<string, unknown>>(
  feature: FeatureId,
  thread: ThreadForTranscript,
  others: V,
): V & { transcript: string } {
  const transcript = renderTranscript({ ...thread, maxInputChars: transcriptBudget(feature, others) });
  return { transcript, ...others };
}

export function summarizeVars(input: ThreadInput) {
  return withTranscript("summarize", input.thread, {
    userName: input.userName,
    today: today(input.now ?? new Date(), input.thread.timezone),
  });
}

export async function summarize(input: ThreadInput, ctx: FeatureContext): Promise<Summary> {
  const vars = summarizeVars(input);
  const out = (await collectFeature(runFeature("summarize", vars, ctx))).object as Summary;
  return { ...out, bullets: out.bullets.slice(0, 5) };
}

export function instantReplyVars(input: ThreadInput & { voice?: string | null }) {
  return withTranscript("instant_reply", input.thread, {
    userName: input.userName,
    voice: input.voice ?? null,
  });
}

export async function instantReply(
  input: ThreadInput & { voice?: string | null },
  ctx: FeatureContext,
): Promise<{ label: string; body: string }[]> {
  const vars = instantReplyVars(input);
  const out = (await collectFeature(runFeature("instant_reply", vars, ctx))).object as {
    replies: { label: string; body: string }[];
  };
  return out.replies.slice(0, 3);
}

export interface WriteInVoiceInput {
  instruction: string;
  userName: string;
  thread?: ThreadForTranscript | null;
  draft?: string | null;
  voice?: string | null;
}

/** Vars for `compose`; the transcript gets whatever input budget the instruction and draft leave. */
export function composeVars(input: WriteInVoiceInput) {
  const others = {
    instruction: input.instruction,
    draft: input.draft ?? null,
    voice: input.voice ?? null,
    userName: input.userName,
  };
  const transcript = input.thread
    ? renderTranscript({ ...input.thread, maxInputChars: transcriptBudget("compose", others) })
    : null;
  return {
    instruction: others.instruction,
    transcript,
    draft: others.draft,
    voice: others.voice,
    userName: others.userName,
  };
}

/** `compose` ("write in my voice"): streams text; use runFeature directly to render deltas. */
export async function writeInVoice(input: WriteInVoiceInput, ctx: FeatureContext): Promise<string> {
  return (await collectFeature(runFeature("compose", composeVars(input), ctx))).text ?? "";
}

export async function rewrite(
  input: { text: string; style: "shorter" | "friendlier" | "formal" | "fix"; voice?: string | null },
  ctx: FeatureContext,
): Promise<string> {
  const vars = { text: input.text, style: input.style, voice: input.voice ?? null };
  return (await collectFeature(runFeature("rewrite", vars, ctx))).text ?? "";
}

export type ActionItem = {
  kind: "todo" | "deadline" | "meeting" | "payment" | "reply";
  title: string;
  due: string | null;
  quote: string;
};

export function extractActionsVars(input: { thread: ThreadForTranscript; now?: Date }) {
  return withTranscript("extract_actions", input.thread, {
    today: today(input.now ?? new Date(), input.thread.timezone),
  });
}

export async function extractActions(
  input: { thread: ThreadForTranscript; now?: Date },
  ctx: FeatureContext,
): Promise<ActionItem[]> {
  const vars = extractActionsVars(input);
  return (
    (await collectFeature(runFeature("extract_actions", vars, ctx))).object as { actions: ActionItem[] }
  ).actions;
}

export type TriageCategory =
  | "important"
  | "personal"
  | "updates"
  | "newsletters"
  | "promotions"
  | "receipts"
  | "calendar"
  | "notifications"
  | "other";

export interface TriageResult {
  threadId: string;
  category: TriageCategory;
  needsReply: boolean;
  model: string;
  promptVersion: number;
}

/** Maps the model's keyed answer back to thread ids; unknown or duplicate keys are dropped. */
export function triageResults(
  keys: Map<string, string>,
  output: { items: { key: string; category: TriageCategory; needsReply: boolean }[] },
  model: string,
): TriageResult[] {
  const seen = new Set<string>();
  const out: TriageResult[] = [];
  for (const item of output.items) {
    const threadId = keys.get(item.key);
    if (!threadId || seen.has(threadId)) continue;
    seen.add(threadId);
    out.push({
      threadId,
      category: item.category,
      needsReply: item.needsReply,
      model,
      promptVersion: promptVersion("triage"),
    });
  }
  return out;
}

/** Triage up to 25 items (headers and snippet only). Results go to `ai.setTriage`. */
export async function triage(items: TriageInput[], ctx: FeatureContext): Promise<TriageResult[]> {
  if (items.length === 0) return [];
  const { text, keys } = renderTriageItems(items);
  const res = await collectFeature(runFeature("triage", { items: text }, ctx));
  return triageResults(keys, res.object as Parameters<typeof triageResults>[1], res.model ?? "unknown");
}

export async function nlSearchToQuery(
  input: { query: string; timezone: string; labels: string[]; splits: string[]; now?: Date },
  ctx: FeatureContext,
): Promise<{ query: string; explanation: string }> {
  const vars = {
    query: input.query,
    today: today(input.now ?? new Date(), input.timezone),
    timezone: input.timezone,
    labels: input.labels.join("\n"),
    splits: input.splits.join("\n"),
  };
  return (await collectFeature(runFeature("nl_search", vars, ctx))).object as {
    query: string;
    explanation: string;
  };
}

/** Builds a style-only voice profile from up to 10 of the user's own sent bodies (desktop or web only). */
export async function voiceProfile(samples: string[], ctx: FeatureContext): Promise<string> {
  const text = samples
    .slice(0, 10)
    .map((s, i) => `Sample ${i + 1}:\n${s.replace(/\r\n?/g, "\n").trim()}`)
    .join("\n\n");
  const out = (await collectFeature(runFeature("voice_profile", { samples: text }, ctx))).object as {
    profile: string;
  };
  return Array.from(out.profile).slice(0, 2048).join("");
}
