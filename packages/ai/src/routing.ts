// The task → tier routing table (ARCHITECTURE §12.3). routing.json is the only place model ids live;
// crates/ai embeds the same file with include_str!.
import routingJson from "../routing.json" with { type: "json" };

export const AI_FEATURES = [
  "summarize",
  "instant_reply",
  "compose",
  "rewrite",
  "extract_actions",
  "triage",
  "nl_search",
  "voice_profile",
  "dictation",
] as const;
/** `vAiFeature` (convex/lib/validators.ts). */
export type AiFeature = (typeof AI_FEATURES)[number];

export const TIERS = ["fast", "default", "heavy", "transcription"] as const;
export type Tier = (typeof TIERS)[number];

export const PROVIDER_KINDS = [
  "akatsuki",
  "openrouter",
  "anthropic",
  "openai",
  "google",
  "gateway",
  "ollama",
  "openai_compatible",
] as const;
/** `vAiProviderKind`. */
export type ProviderKind = (typeof PROVIDER_KINDS)[number];

export interface ModelPrice {
  inputPerMTokMicros?: number;
  outputPerMTokMicros?: number;
  perAudioMinuteMicros?: number;
}

export interface RoutingTable {
  version: number;
  features: Record<AiFeature, Tier>;
  tiers: Record<Tier, Partial<Record<ProviderKind, string>>>;
  /** Escalation candidates per tier and provider kind, cheapest first (DECISIONS D-120). */
  escalation: Partial<Record<Tier, Partial<Record<ProviderKind, string[]>>>>;
  /** List prices in micro-dollars, keyed by the managed (OpenRouter-style) model id. */
  pricing: Record<string, ModelPrice>;
}

export const ROUTING: RoutingTable = routingJson as RoutingTable;

export function isAiFeature(value: unknown): value is AiFeature {
  return typeof value === "string" && (AI_FEATURES as readonly string[]).includes(value);
}

export function tierFor(feature: AiFeature, table: RoutingTable = ROUTING): Tier {
  return table.features[feature];
}

/** The routing-table model for a tier and provider kind, if the table has one. */
export function tableModel(
  tier: Tier,
  kind: ProviderKind,
  table: RoutingTable = ROUTING,
): string | undefined {
  return table.tiers[tier]?.[kind];
}

/**
 * Escalation hook (D-120). A feature runs on its table tier; a hook may lift one request to a heavier
 * model, for example after a schema-invalid answer or for an unusually long thread. The default policy
 * never escalates; a feature moves tiers permanently only through routing.json, backed by an eval.
 */
export interface EscalationContext {
  feature: AiFeature;
  tier: Tier;
  kind: ProviderKind;
  /** 0 for the first call, 1 for the retry after a failed attempt. */
  attempt: number;
  reason: "initial" | "invalid_output";
  inputChars: number;
}
export type EscalationHook = (ctx: EscalationContext) => { tier: Tier; model?: string } | null;

export const NO_ESCALATION: EscalationHook = () => null;

/** Escalate an invalid structured answer once, to the first heavy candidate for the provider kind. */
export const escalateOnInvalidOutput: EscalationHook = (ctx) => {
  if (ctx.reason !== "invalid_output" || ctx.attempt > 1 || ctx.tier === "transcription") return null;
  const model = ROUTING.escalation.heavy?.[ctx.kind]?.[0];
  return model ? { tier: "heavy", model } : null;
};

/** Candidates for a heavier model (Opus 5.5, then Fable 5.1) for a BYO provider kind. */
export function escalationCandidates(
  tier: Tier,
  kind: ProviderKind,
  table: RoutingTable = ROUTING,
): string[] {
  return table.escalation[tier]?.[kind] ?? [];
}
