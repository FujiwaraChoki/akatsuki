// AI on the web client (ARCHITECTURE §12): the user's synced settings (`ai.getSettings`), BYO keys from this
// browser (keyStore), and one `run()` that streams AASP events from either path:
// - managed (`akatsuki` profile, Pro): apps/api `POST /v1/ai/{feature}` through the backend's ApiService;
// - BYO and local: `runFeature` from packages/ai, straight from this device to the chosen provider.
// The UI is identical either way (§12.6). Results are cached on the device only (§12.7 rule 5, `cache.ts`).
import type { AaspEvent } from "@akatsuki/wire";
import { WireError } from "@akatsuki/wire";
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useBackend } from "../backend/context.tsx";
import type { AiSettingsView, ApiService } from "../backend/types.ts";
import { aiCacheGet, aiCachePut } from "./cache.ts";
import { type KeyStore, keyStore } from "./keyStore.ts";
import { type AiModule, loadAi, useAiModule } from "./module.ts";

export type AiFeatureId =
  | "summarize"
  | "instant_reply"
  | "compose"
  | "rewrite"
  | "extract_actions"
  | "triage"
  | "nl_search"
  | "voice_profile";

/** Settings in the packages/ai shape (no Convex system fields). */
export interface AiSettingsLike {
  profiles: AiSettingsView["profiles"];
  routes: AiSettingsView["routes"];
  consent: AiSettingsView["consent"];
  voiceProfile?: string;
}

export const NO_AI_SETTINGS: AiSettingsLike = {
  profiles: [],
  routes: [],
  consent: { autoTriage: false, syncVoiceProfile: false },
};

export function settingsFromView(v: AiSettingsView | null | undefined): AiSettingsLike {
  if (!v) return NO_AI_SETTINGS;
  return {
    profiles: v.profiles,
    routes: v.routes,
    consent: v.consent,
    ...(v.voiceProfile !== undefined && { voiceProfile: v.voiceProfile }),
  };
}

/** Why a feature cannot run here, or "ready". */
export type AiStatus =
  | "loading"
  | "ready"
  | "consent"
  | "off"
  | "no_model"
  | "no_key"
  | "not_entitled"
  | "no_api";

export interface AiEnv {
  settings: AiSettingsLike;
  api: ApiService;
  keys: Pick<KeyStore, "get">;
  managedAi: boolean;
}

const fail = (code: string, message: string) => new WireError({ code, message, retryable: false });

/** Streams one feature through the resolved path. Client-side states throw a WireError with their §15 code. */
export async function* runAi(
  feature: AiFeatureId,
  vars: Record<string, unknown>,
  env: AiEnv,
  signal?: AbortSignal,
  ai?: AiModule,
): AsyncGenerator<AaspEvent> {
  const m = ai ?? (await loadAi());
  if (!env.settings.consent.grantedAt)
    throw fail("consent_required", "Turn on AI features in Settings first.");
  const r = m.resolveFeature(feature, env.settings as never);
  if (r.status === "off") throw fail("ai_off", "This AI feature is turned off.");
  if (r.status === "error") throw fail("ai_model_required", "Choose a model.");
  if (r.managed) {
    if (!env.managedAi) throw fail("not_entitled", "Akatsuki AI is part of Pro.");
    if (!env.api.configured) throw fail("unreachable", "Akatsuki AI is not configured in this build.");
    yield* env.api.streamFeature(feature, { promptVersion: m.promptVersion(feature), vars }, signal);
    return;
  }
  yield* m.runFeature(feature, vars, {
    settings: env.settings as never,
    keyFor: (id) => env.keys.get(id),
    abortSignal: signal,
    providerOptions: { browser: true },
  });
}

/** Status of a feature for the current settings (what the AI buttons show). */
export function aiStatus(
  ai: AiModule | null,
  feature: AiFeatureId | "dictation",
  env: Omit<AiEnv, "api"> & { apiConfigured: boolean },
): AiStatus {
  if (!ai) return "loading";
  if (!env.settings.consent.grantedAt) return "consent";
  const r = ai.resolveFeature(feature, env.settings as never);
  if (r.status === "off") return "off";
  if (r.status === "error") return "no_model";
  if (r.managed) return !env.managedAi ? "not_entitled" : env.apiConfigured ? "ready" : "no_api";
  if (!ai.KEYLESS_KINDS.has(r.profile.kind) && !env.keys.get(r.profile.profileId)) return "no_key";
  return "ready";
}

export const STATUS_COPY: Record<Exclude<AiStatus, "ready" | "loading">, string> = {
  consent: "AI is off. Turn it on in Settings, AI.",
  off: "This AI feature is turned off in Settings, AI.",
  no_model: "Choose a model for this feature in Settings, AI.",
  no_key: "Key not set on this device. Add it in Settings, AI.",
  not_entitled: "Akatsuki AI is part of Pro. Use your own key or a local model instead.",
  no_api: "Akatsuki AI is not configured in this build.",
};

/** The settings, keys and run function for the signed-in user. */
export function useAi() {
  const backend = useBackend();
  const view = backend.useAiSettings();
  const me = backend.useMe();
  const ai = useAiModule();
  const keys = keyStore();
  const keysVersion = useSyncExternalStore(keys.subscribe, keys.getVersion, keys.getVersion);
  useEffect(() => {
    void keys.load();
  }, [keys]);
  const settings = useMemo(() => settingsFromView(view), [view]);
  const managedAi = me?.entitlement.managedAi ?? false;
  // biome-ignore lint/correctness/useExhaustiveDependencies: keysVersion changes when a key is set
  const env: AiEnv = useMemo(
    () => ({ settings, api: backend.api, keys, managedAi }),
    [settings, backend.api, keys, managedAi, keysVersion],
  );
  const status = useCallback(
    (feature: AiFeatureId | "dictation") =>
      view === undefined
        ? "loading"
        : aiStatus(ai, feature, { ...env, apiConfigured: backend.api.configured }),
    [ai, env, view, backend.api.configured],
  );
  /** "Stays on this device" / "Sent to Akatsuki…" / "Leaves this device: …" for a feature, or null when off. */
  const egress = useCallback(
    (feature: AiFeatureId | "dictation" | "default"): string | null => {
      if (!ai) return null;
      if (feature === "default") {
        const route = settings.routes.find((r) => r.feature === "default" && r.enabled);
        const p = route && settings.profiles.find((x) => x.profileId === route.profileId);
        return p ? ai.egressText(p, "default") : null;
      }
      const r = ai.resolveFeature(feature, settings as never);
      return r.status === "ok" ? ai.egressText(r.profile, feature) : null;
    },
    [ai, settings],
  );
  const run = useCallback(
    (feature: AiFeatureId, vars: Record<string, unknown>, signal?: AbortSignal) =>
      runAi(feature, vars, env, signal, ai ?? undefined),
    [env, ai],
  );
  return { ai, settings, view, env, status, egress, run, managedAi, keys };
}

export type UseAi = ReturnType<typeof useAi>;

export interface StreamState {
  status: "idle" | "running" | "done" | "error";
  text: string;
  object: unknown;
  model: string | null;
  error: unknown;
  cached: boolean;
}

const IDLE: StreamState = { status: "idle", text: "", object: null, model: null, error: null, cached: false };

/** Runs a feature and keeps the streamed text or partial object in state. `cache` keeps the result on the device. */
export function useAiStream(ai: UseAi) {
  const [state, setState] = useState<StreamState>(IDLE);
  const ctrl = useRef<AbortController | null>(null);
  useEffect(() => () => ctrl.current?.abort(), []);

  const start = useCallback(
    async (feature: AiFeatureId, vars: Record<string, unknown>, opts: { cache?: boolean } = {}) => {
      ctrl.current?.abort();
      const c = new AbortController();
      ctrl.current = c;
      const mod = ai.ai ?? (await loadAi());
      const r = mod.resolveFeature(feature, ai.settings as never);
      const model = r.status === "ok" ? r.model : "";
      const version = mod.promptVersion(feature);
      if (opts.cache) {
        const hit = await aiCacheGet(feature, version, model, vars);
        if (hit && !c.signal.aborted) {
          setState({
            status: "done",
            text: hit.text ?? "",
            object: hit.object,
            model,
            error: null,
            cached: true,
          });
          return;
        }
      }
      setState({ ...IDLE, status: "running" });
      let text = "";
      let object: unknown = null;
      let usedModel: string | null = null;
      try {
        for await (const e of ai.run(feature, vars, c.signal)) {
          if (c.signal.aborted) return;
          if (e.event === "meta") usedModel = e.data.model;
          else if (e.event === "delta") {
            text += e.data.text;
            setState((s) => ({ ...s, text }));
          } else if (e.event === "partial") {
            object = e.data.object;
            setState((s) => ({ ...s, object }));
          } else if (e.event === "error") throw new WireError(e.data);
          else if (e.event === "done") {
            text = e.data.text ?? text;
            object = e.data.object ?? object;
          }
        }
        if (c.signal.aborted) return;
        setState({ status: "done", text, object, model: usedModel, error: null, cached: false });
        if (opts.cache) void aiCachePut(feature, version, model, vars, { text, object });
      } catch (error) {
        if (c.signal.aborted || (error as { name?: string })?.name === "AbortError") return;
        setState({ ...IDLE, status: "error", error });
      }
    },
    [ai],
  );
  const cancel = useCallback(() => {
    ctrl.current?.abort();
    setState(IDLE);
  }, []);
  return { state, start, cancel };
}
