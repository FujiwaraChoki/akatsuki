// AI provider settings on mobile (ARCHITECTURE §12.2): profiles and routes sync through Convex `aiSettings`;
// BYO keys live only in this device's SecureStore under `ai.<profileId>`, and `keyOnDevices` records which
// devices hold one. Egress labels come from packages/ai (`egressText`), the one implementation of §12.2.
import {
  type AiProfile,
  type AiRoute,
  type AiSettings,
  baseUrlProblem,
  type Egress,
  EMPTY_AI_SETTINGS,
  egressFor,
  egressText,
  isLoopbackHost,
  type ProviderKind,
  ROUTING,
  type Tier,
} from "@akatsuki/ai";

export interface ProviderChoice {
  kind: ProviderKind;
  title: string;
  description: string;
  needsKey: boolean;
  /** "required" | "optional" | "none" */
  baseUrl: "required" | "optional" | "none";
  keyPlaceholder?: string;
  baseUrlPlaceholder?: string;
}

export const PROVIDER_CHOICES: readonly ProviderChoice[] = [
  {
    kind: "akatsuki",
    title: "Akatsuki AI",
    description: "Included with Akatsuki Pro. Sent to Akatsuki for processing, not stored.",
    needsKey: false,
    baseUrl: "none",
  },
  {
    kind: "openrouter",
    title: "OpenRouter",
    description: "Your OpenRouter key. Requests ask for zero data retention.",
    needsKey: true,
    baseUrl: "none",
    keyPlaceholder: "sk-or-…",
  },
  {
    kind: "anthropic",
    title: "Anthropic",
    description: "Your Anthropic API key.",
    needsKey: true,
    baseUrl: "none",
    keyPlaceholder: "sk-ant-…",
  },
  {
    kind: "openai",
    title: "OpenAI",
    description: "Your OpenAI API key. Choose a model.",
    needsKey: true,
    baseUrl: "none",
    keyPlaceholder: "sk-…",
  },
  {
    kind: "google",
    title: "Google Gemini",
    description: "Your Google AI Studio key. Choose a model.",
    needsKey: true,
    baseUrl: "none",
  },
  {
    kind: "gateway",
    title: "Vercel AI Gateway",
    description: "Your AI Gateway key.",
    needsKey: true,
    baseUrl: "none",
  },
  {
    kind: "ollama",
    title: "Ollama on your network",
    description: "A computer on your Wi-Fi running Ollama. Mail stays on your local network.",
    needsKey: false,
    baseUrl: "required",
    baseUrlPlaceholder: "http://192.168.1.20:11434",
  },
  {
    kind: "openai_compatible",
    title: "OpenAI-compatible server",
    description: "LM Studio, llama.cpp, vLLM or any OpenAI-style endpoint.",
    needsKey: false,
    baseUrl: "required",
    baseUrlPlaceholder: "http://192.168.1.20:1234/v1",
  },
];

export function providerChoice(kind: ProviderKind): ProviderChoice {
  return PROVIDER_CHOICES.find((c) => c.kind === kind)!;
}

/** Version of the consent text below; stored in `consent.version`. */
export const CONSENT_VERSION = 1;

/** The consent sentence for a profile (§12.7 rule 1: names the provider and what leaves the device). */
export function consentText(profile: Pick<AiProfile, "kind" | "baseUrl">): string {
  const where = egressText(profile, "default");
  return `AI features run only when you ask for them, plus auto-triage if you turn it on. ${where}.`;
}

/** SecureStore key of a profile's API key (§12.2: `ai.<profileId>`). */
export function keyStorageName(profileId: string): string {
  if (!/^[A-Za-z0-9-]+$/.test(profileId)) throw new Error("bad profile id");
  return `ai.${profileId}`;
}

/** The model a tier resolves to for a provider kind when the user picks none (§12.3 step 3). */
export function defaultModel(kind: ProviderKind, tier: Tier): string | undefined {
  return ROUTING.tiers[tier]?.[kind];
}

/** Kinds where the user must choose the model (no routing default). */
export function modelRequired(kind: ProviderKind): boolean {
  return kind !== "akatsuki" && defaultModel(kind, "default") === undefined;
}

export interface ProfileDraft {
  profileId: string;
  kind: ProviderKind;
  label: string;
  baseUrl: string;
  /** Model for the default tier ("" = routing default). */
  model: string;
  /** Model for the fast tier ("" = routing default or the default-tier model). */
  fastModel: string;
}

export function draftFromProfile(p: AiProfile): ProfileDraft {
  return {
    profileId: p.profileId,
    kind: p.kind,
    label: p.label,
    baseUrl: p.baseUrl ?? "",
    model: p.models.default ?? "",
    fastModel: p.models.fast ?? "",
  };
}

export function newDraft(profileId: string, kind: ProviderKind): ProfileDraft {
  return { profileId, kind, label: providerChoice(kind).title, baseUrl: "", model: "", fastModel: "" };
}

/** Validation errors for a draft, in display order. */
export function draftProblems(d: ProfileDraft, opts: { managedAiAllowed: boolean }): string[] {
  const problems: string[] = [];
  const choice = providerChoice(d.kind);
  if (d.kind === "akatsuki" && !opts.managedAiAllowed)
    problems.push("Akatsuki AI needs an active Akatsuki Pro plan on this account.");
  if (!d.label.trim()) problems.push("Give this provider a name.");
  if (d.label.length > 80) problems.push("The name is too long.");
  const base = d.baseUrl.trim();
  if (choice.baseUrl === "required" && !base) {
    problems.push(
      d.kind === "ollama"
        ? "Enter the address of the computer running Ollama, for example http://192.168.1.20:11434. A phone does not run Ollama itself."
        : "Enter the server address.",
    );
  }
  if (base) {
    const p = baseUrlProblem(base);
    if (p) problems.push(`The address ${p}.`);
    else if (d.kind === "ollama" || d.kind === "openai_compatible") {
      try {
        if (isLoopbackHost(new URL(base).hostname))
          problems.push("localhost is this phone. Use the address of the computer on your network instead.");
      } catch {
        // reported by baseUrlProblem
      }
    }
  }
  if (modelRequired(d.kind) && !d.model.trim()) problems.push("Choose a model.");
  return problems;
}

/** Builds the synced profile (never the key). `keyOnDevices` is maintained by `withKeyOnDevice`. */
export function profileFromDraft(d: ProfileDraft, existing?: AiProfile): AiProfile {
  const models: AiProfile["models"] = {};
  if (d.model.trim()) models.default = d.model.trim();
  if (d.fastModel.trim()) models.fast = d.fastModel.trim();
  else if (d.model.trim() && defaultModel(d.kind, "fast") === undefined) models.fast = d.model.trim();
  const profile: AiProfile = {
    profileId: d.profileId,
    label: d.label.trim(),
    kind: d.kind,
    models,
    keyOnDevices: existing?.keyOnDevices ?? [],
  };
  const base = d.baseUrl.trim().replace(/\/+$/, "");
  if (base && d.kind !== "akatsuki") profile.baseUrl = base;
  return profile;
}

/** Adds or removes this device in `keyOnDevices`. */
export function withKeyOnDevice(profile: AiProfile, deviceId: string, stored: boolean): AiProfile {
  const set = new Set(profile.keyOnDevices.filter((d) => d !== deviceId));
  if (stored) set.add(deviceId);
  return { ...profile, keyOnDevices: [...set].slice(0, 50) };
}

/** Inserts or replaces a profile; with `makeDefault` it also becomes the enabled `default` route. */
export function upsertProfile(
  settings: AiSettings,
  profile: AiProfile,
  makeDefault: boolean,
): Pick<AiSettings, "profiles" | "routes"> {
  const profiles = settings.profiles.filter((p) => p.profileId !== profile.profileId);
  profiles.push(profile);
  let routes = settings.routes;
  if (makeDefault) {
    routes = routes.filter((r) => r.feature !== "default");
    routes = [{ feature: "default", profileId: profile.profileId, enabled: true }, ...routes];
  }
  return { profiles, routes };
}

/** Removes a profile and every route that used it. */
export function removeProfile(
  settings: AiSettings,
  profileId: string,
): Pick<AiSettings, "profiles" | "routes"> {
  return {
    profiles: settings.profiles.filter((p) => p.profileId !== profileId),
    routes: settings.routes.filter((r) => r.profileId !== profileId),
  };
}

/** The profile the default route points at, if it is enabled. */
export function defaultProfile(settings: AiSettings): AiProfile | undefined {
  const route = settings.routes.find((r: AiRoute) => r.feature === "default" && r.enabled);
  return route ? settings.profiles.find((p) => p.profileId === route.profileId) : undefined;
}

export type KeyStatus = "not_needed" | "on_this_device" | "on_other_devices" | "missing";

export function keyStatus(profile: AiProfile, deviceId: string, hasLocalKey: boolean): KeyStatus {
  if (!providerChoice(profile.kind).needsKey) return "not_needed";
  if (hasLocalKey) return "on_this_device";
  if (profile.keyOnDevices.some((d) => d !== deviceId)) return "on_other_devices";
  return "missing";
}

export function keyStatusText(status: KeyStatus): string {
  switch (status) {
    case "not_needed":
      return "No key needed";
    case "on_this_device":
      return "Key stored on this device";
    case "on_other_devices":
      return "Key not set on this device (it is stored on another device)";
    case "missing":
      return "Key not set on this device";
  }
}

export interface EgressView {
  egress: Egress;
  text: string;
}

/** The egress label for settings (the default route row) or one feature. */
export function egressView(
  profile: Pick<AiProfile, "kind" | "baseUrl">,
  feature: Parameters<typeof egressFor>[1] = "default",
): EgressView {
  return { egress: egressFor(profile, feature), text: egressText(profile, feature) };
}

/** Normalises `ai.getSettings` (null until the row exists, D-353) to settings. */
export function settingsOrEmpty(view: Partial<AiSettings> | null | undefined): AiSettings {
  if (!view) return EMPTY_AI_SETTINGS;
  return {
    profiles: view.profiles ?? [],
    routes: view.routes ?? [],
    consent: view.consent ?? EMPTY_AI_SETTINGS.consent,
    ...(view.voiceProfile !== undefined ? { voiceProfile: view.voiceProfile } : {}),
  };
}

/** How "Test connection" reaches a profile (§12.2: from the device; Ollama via `GET /api/tags`). */
export type ConnectionTest =
  | { kind: "http_get"; url: string }
  | { kind: "generate"; model: string }
  | { kind: "managed" };

export function connectionTest(profile: AiProfile): ConnectionTest {
  if (profile.kind === "akatsuki") return { kind: "managed" };
  if (profile.kind === "ollama") {
    const base = (profile.baseUrl ?? "http://127.0.0.1:11434").replace(/\/+$/, "").replace(/\/api$/, "");
    return { kind: "http_get", url: `${base}/api/tags` };
  }
  const model = profile.models.fast ?? profile.models.default ?? defaultModel(profile.kind, "fast") ?? "";
  return { kind: "generate", model };
}
