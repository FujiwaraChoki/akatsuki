// Render goldens: every eval fixture rendered through the TypeScript renderer. crates/ai renders the same
// vars and must produce byte-identical `system` and `user` strings (§12.4 "CI renders every fixture").
import type { EvalFixture } from "./fs.ts";
import { type FeatureId, type PromptPack, renderTemplate, type TemplateVars } from "./template.ts";

export interface RenderGolden {
  feature: FeatureId;
  name: string;
  version: number;
  vars: Record<string, unknown>;
  system: string;
  user: string;
}

export function renderGoldens(pack: PromptPack, fixtures: EvalFixture[]): RenderGolden[] {
  return fixtures.map((fx) => {
    const f = pack.features[fx.feature];
    return {
      feature: fx.feature,
      name: fx.name,
      version: f.meta.version,
      vars: fx.vars,
      system: f.system,
      user: renderTemplate(f.user, fx.vars as TemplateVars),
    };
  });
}
