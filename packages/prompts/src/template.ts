// Pure prompt-pack types and the `{{var}}` renderer (README.md). No file or bundle access, so it is safe
// for the build script, browsers and React Native.
export const FEATURE_IDS = [
  "summarize",
  "instant_reply",
  "compose",
  "rewrite",
  "extract_actions",
  "triage",
  "nl_search",
  "voice_profile",
] as const;
/** Prompt-pack features: `vAiFeature` minus `dictation` (dictation is transcription, it has no prompt). */
export type FeatureId = (typeof FEATURE_IDS)[number];

export type Tier = "fast" | "default" | "heavy" | "transcription";
export type Effort = "low" | "medium" | "high";

export interface FeatureMeta {
  id: FeatureId;
  version: number;
  tier: Exclude<Tier, "transcription">;
  mode: "text" | "object";
  /** Stream deltas (text) or partial objects (object) to the UI. */
  stream: boolean;
  maxInputChars: number;
  maxOutputTokens: number;
  effort: Effort | null;
}

/** A JSON Schema document (input schemas use draft-07; output schemas use the strict subset). */
export type JsonSchema = { [key: string]: unknown };

export interface PromptFeature {
  meta: FeatureMeta;
  /** Final system prompt: system.md with shared/untrusted.md appended (README "System prompt"). */
  system: string;
  /** user.md, with `{{var}}` placeholders. */
  user: string;
  /** Placeholder names used by `user`, in order of first appearance. */
  variables: string[];
  inputSchema: JsonSchema;
  /** Present for `mode: "object"` features. */
  outputSchema: JsonSchema | null;
}

export interface PromptPack {
  packVersion: string;
  features: Record<FeatureId, PromptFeature>;
}

export function isFeatureId(value: unknown): value is FeatureId {
  return typeof value === "string" && (FEATURE_IDS as readonly string[]).includes(value);
}

export class PromptError extends Error {
  override name = "PromptError";
}

/** `{{name}}`: a letter, then letters, digits or underscores. Nothing else is template syntax. */
export const PLACEHOLDER = /\{\{([A-Za-z][A-Za-z0-9_]*)\}\}/g;

/** Placeholder names in order of first appearance. Throws on a stray `{{` or `}}`. */
export function templateVariables(template: string): string[] {
  const names: string[] = [];
  for (const m of template.matchAll(PLACEHOLDER)) {
    const name = m[1]!;
    if (!names.includes(name)) names.push(name);
  }
  const rest = template.replace(PLACEHOLDER, "");
  if (rest.includes("{{") || rest.includes("}}"))
    throw new PromptError("template contains '{{' or '}}' outside a valid placeholder");
  return names;
}

/** Escapes a value so email content cannot open or close a tag: `<` → `&lt;`, `>` → `&gt;`. */
export function escapeValue(value: string): string {
  return value.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

export type TemplateVars = Record<string, string | null | undefined>;

/**
 * Substitutes every `{{name}}` with the escaped value of `vars[name]`. `null` renders as the empty string.
 * A placeholder without a value (undefined) or with a non-string value throws: callers validate `vars`
 * against input.schema.json first. Substituted text is never re-scanned for placeholders.
 */
export function renderTemplate(template: string, vars: TemplateVars): string {
  templateVariables(template);
  return template.replace(PLACEHOLDER, (_m, name: string) => {
    const value = Object.hasOwn(vars, name) ? vars[name] : undefined;
    if (value === undefined) throw new PromptError(`missing template variable "${name}"`);
    if (value === null) return "";
    if (typeof value !== "string")
      throw new PromptError(`template variable "${name}" must be a string or null`);
    return escapeValue(value);
  });
}

/** Composes the final system prompt. Same rule in Rust (`crates/ai`): trimEnd both, join with one blank line. */
export function composeSystem(systemMd: string, untrustedMd: string): string {
  return `${systemMd.trimEnd()}\n\n${untrustedMd.trimEnd()}\n`;
}
