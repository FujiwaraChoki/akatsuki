// Typed access to the prompt pack (ARCHITECTURE §12.4). The plain files under features/ and shared/ are the
// source; pack.json is the same content bundled by `pnpm build` (system prompts already composed), so this
// module works in browsers, React Native and Node without file-system access. Format: README.md.
import pack from "../pack.json" with { type: "json" };
import {
  type FeatureId,
  PromptError,
  type PromptFeature,
  type PromptPack,
  renderTemplate,
  type TemplateVars,
} from "./template.ts";

export * from "./template.ts";

export const PROMPT_PACK: PromptPack = pack as unknown as PromptPack;
export const PACK_VERSION: string = PROMPT_PACK.packVersion;

export function getFeature(id: FeatureId, source: PromptPack = PROMPT_PACK): PromptFeature {
  const feature = source.features[id];
  if (!feature) throw new PromptError(`unknown feature "${id}"`);
  return feature;
}

/** `manifest.features[id].version`; apps/api answers 409 when a client sends another value. */
export function promptVersion(id: FeatureId, source: PromptPack = PROMPT_PACK): number {
  return getFeature(id, source).meta.version;
}

export interface RenderedPrompt {
  feature: FeatureId;
  version: number;
  system: string;
  user: string;
}

/** Renders one feature's prompt from already-validated vars. */
export function renderPrompt(
  id: FeatureId,
  vars: TemplateVars,
  source: PromptPack = PROMPT_PACK,
): RenderedPrompt {
  const f = getFeature(id, source);
  const picked: TemplateVars = {};
  for (const name of f.variables) picked[name] = vars[name];
  return { feature: id, version: f.meta.version, system: f.system, user: renderTemplate(f.user, picked) };
}
