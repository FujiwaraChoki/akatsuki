// AI SDK provider registry (§12.3 step 5): one registered provider per user profile, built from the
// profile's kind and baseUrl plus a key the device supplies at call time. Keys are never read from the
// environment here and never stored in settings.
import { createAnthropic } from "@ai-sdk/anthropic";
import { createGateway } from "@ai-sdk/gateway";
import { createGoogleGenerativeAI } from "@ai-sdk/google";
import { createOpenAI } from "@ai-sdk/openai";
import { createOpenAICompatible } from "@ai-sdk/openai-compatible";
import { createOpenRouter } from "@openrouter/ai-sdk-provider";
import { createProviderRegistry, type LanguageModel } from "ai";
import { createOllama } from "ollama-ai-provider-v2";
import type { ProviderKind } from "./routing.ts";
import type { AiProfile } from "./settings.ts";

type AnyProvider = Parameters<typeof createProviderRegistry>[0][string];
type FetchFn = typeof globalThis.fetch;

/**
 * OpenRouter routing preferences used for every Akatsuki request (managed and BYO): only zero-data-retention
 * endpoints, and never providers that may store or train on data. If no ZDR endpoint serves the model the
 * request fails instead of falling back to one that retains data (§12.3, §12.7).
 */
export const OPENROUTER_PRIVACY = {
  zdr: true,
  data_collection: "deny",
  allow_fallbacks: true,
  require_parameters: true,
} as const;

export const OLLAMA_DEFAULT_URL = "http://127.0.0.1:11434";

export class MissingKeyError extends Error {
  override name = "MissingKeyError";
  readonly profileId: string;
  constructor(profileId: string) {
    super("Key not set on this device");
    this.profileId = profileId;
  }
}

export interface ProviderOptions {
  /** True when running in a browser (web app): Anthropic then needs the direct-browser-access header. */
  browser?: boolean;
  /** Injected fetch (tests, proxies). */
  fetch?: FetchFn;
}

const isBrowser = () => typeof (globalThis as { document?: unknown }).document !== "undefined";
const trimSlash = (u: string) => u.replace(/\/+$/, "");
/** Kinds that run without a key (local servers). */
export const KEYLESS_KINDS: ReadonlySet<ProviderKind> = new Set(["ollama", "openai_compatible"]);

/** Builds the AI SDK provider for one BYO profile. `akatsuki` profiles never get a provider: they go to apps/api. */
export function createProfileProvider(
  profile: Pick<AiProfile, "profileId" | "kind" | "baseUrl">,
  apiKey: string | undefined,
  options: ProviderOptions = {},
): AnyProvider {
  const { kind, baseUrl } = profile;
  if (kind === "akatsuki")
    throw new Error("the managed provider is called through apps/api, not the registry");
  if (!apiKey && !KEYLESS_KINDS.has(kind)) throw new MissingKeyError(profile.profileId);
  const fetch = options.fetch;
  const base = baseUrl ? trimSlash(baseUrl) : undefined;
  switch (kind) {
    case "openrouter":
      return createOpenRouter({
        apiKey,
        baseURL: base,
        fetch,
        compatibility: "strict",
        extraBody: { provider: OPENROUTER_PRIVACY },
      });
    case "anthropic":
      return createAnthropic({
        apiKey,
        baseURL: base,
        fetch,
        headers:
          (options.browser ?? isBrowser())
            ? { "anthropic-dangerous-direct-browser-access": "true" }
            : undefined,
      });
    case "openai":
      return createOpenAI({ apiKey, baseURL: base, fetch });
    case "google":
      return createGoogleGenerativeAI({ apiKey, baseURL: base, fetch });
    case "gateway":
      return createGateway({ apiKey, baseURL: base, fetch });
    case "ollama":
      return createOllama({
        baseURL: `${trimSlash(base ?? OLLAMA_DEFAULT_URL).replace(/\/api$/, "")}/api`,
        fetch,
      });
    case "openai_compatible":
      if (!base) throw new Error("openai_compatible profiles need a baseUrl");
      return createOpenAICompatible({
        name: "openai_compatible",
        baseURL: base,
        apiKey,
        fetch,
        supportsStructuredOutputs: true,
      });
  }
}

export interface RegistryInput {
  profiles: Pick<AiProfile, "profileId" | "kind" | "baseUrl">[];
  /** Returns the key held on this device for a profile, if any. */
  keyFor: (profileId: string) => string | undefined;
  options?: ProviderOptions;
}

/**
 * `createProviderRegistry` over every BYO profile that is usable on this device, keyed by profileId, so a
 * resolved model is `registry.languageModel("<profileId>:<modelId>")`. Profiles without a key on this device
 * are left out and reported in `missingKeys`.
 */
export function createAkatsukiRegistry(input: RegistryInput) {
  const providers: Record<string, AnyProvider> = {};
  const missingKeys: string[] = [];
  for (const p of input.profiles) {
    if (p.kind === "akatsuki") continue;
    try {
      providers[p.profileId] = createProfileProvider(p, input.keyFor(p.profileId), input.options);
    } catch (e) {
      if (e instanceof MissingKeyError) missingKeys.push(p.profileId);
      else throw e;
    }
  }
  const registry = createProviderRegistry(providers);
  return {
    registry,
    missingKeys,
    languageModel(profileId: string, model: string): LanguageModel {
      if (!(profileId in providers)) throw new MissingKeyError(profileId);
      return registry.languageModel(`${profileId}:${model}` as never);
    },
  };
}

/**
 * Provider options per kind for one call: Anthropic structured output through `output_config.format`
 * (never a forced tool, which 5.5 models reject) and the effort hint where the provider has one (S6).
 */
export function callProviderOptions(kind: ProviderKind, effort: "low" | "medium" | "high" | null) {
  switch (kind) {
    case "anthropic":
      return { anthropic: { structuredOutputMode: "outputFormat", ...(effort ? { effort } : {}) } };
    case "openrouter":
    case "akatsuki":
      return effort ? { openrouter: { reasoning: { effort, exclude: true } } } : undefined;
    default:
      return undefined;
  }
}

/** The managed model (apps/api only): OpenRouter with ZDR and data-collection deny, exactly as §12.3. */
export function createManagedModel(model: string, apiKey: string, fetch?: FetchFn): LanguageModel {
  return createOpenRouter({ apiKey, fetch, compatibility: "strict" }).chat(model, {
    provider: OPENROUTER_PRIVACY,
  });
}
