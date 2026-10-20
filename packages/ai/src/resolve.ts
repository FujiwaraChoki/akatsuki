// Model resolution (ARCHITECTURE §12.3). Identical in crates/ai::resolve; both run fixtures/resolve/*.json.
import { type AiFeature, ROUTING, type RoutingTable, type Tier, tableModel, tierFor } from "./routing.ts";
import { type AiProfile, type AiRoute, type AiSettings, type Egress, egressFor } from "./settings.ts";

export type Resolution =
  | { status: "off"; feature: AiFeature }
  | { status: "error"; feature: AiFeature; code: "ai_model_required"; profileId: string }
  | {
      status: "ok";
      feature: AiFeature;
      tier: Tier;
      route: AiRoute;
      profile: AiProfile;
      /** For `akatsuki` this is informational: apps/api picks the model server-side. */
      model: string;
      managed: boolean;
      egress: Egress;
    };

export interface ResolveOptions {
  /** Overrides the table tier (used by escalation). */
  tier?: Tier;
  table?: RoutingTable;
}

/**
 * 1. route = enabled route for the feature, else the enabled "default" route; none → off.
 * 2. profile = profiles[route.profileId]; missing → off.
 * 3. model = route.model ?? profile.models[tier] ?? routing.tiers[tier][profile.kind]; none → ai_model_required.
 * 4. `akatsuki` profiles are managed: clients send only {feature, promptVersion, vars} to apps/api.
 */
export function resolveFeature(
  feature: AiFeature,
  settings: Pick<AiSettings, "profiles" | "routes">,
  options: ResolveOptions = {},
): Resolution {
  const table = options.table ?? ROUTING;
  const route =
    settings.routes.find((r) => r.feature === feature && r.enabled) ??
    settings.routes.find((r) => r.feature === "default" && r.enabled);
  if (!route) return { status: "off", feature };
  const profile = settings.profiles.find((p) => p.profileId === route.profileId);
  if (!profile) return { status: "off", feature };
  const tier = options.tier ?? tierFor(feature, table);
  const model = route.model ?? profile.models[tier] ?? tableModel(tier, profile.kind, table);
  if (!model) return { status: "error", feature, code: "ai_model_required", profileId: profile.profileId };
  return {
    status: "ok",
    feature,
    tier,
    route,
    profile,
    model,
    managed: profile.kind === "akatsuki",
    egress: egressFor(profile, feature),
  };
}
