// The AI environment on this phone (ARCHITECTURE §12.2, §12.3, §12.7): the synced settings, BYO keys from
// SecureStore (read only when a feature runs, never on mount), the per-feature status with the same states and
// copy as web, the egress label for any feature, and the packages/ai `FeatureContext` (managed via apps/api with
// the user's token, BYO straight from the device). Thread AI, write with AI, rewrite, Search with AI and
// dictation all go through this one hook.
import {
  type AaspEvent,
  type AiFeature,
  AiFeatureError,
  type AiSettings,
  egressText,
  type FeatureContext,
  resolveFeature,
  runFeature,
} from "@akatsuki/ai";
import { api as convexApi } from "@akatsuki/convex";
import { getAccessToken } from "@akatsuki/wire";
import { useQuery } from "convex/react";
import { fetch as expoFetch } from "expo/fetch";
import { useCallback, useMemo, useRef } from "react";
import { ENV } from "../env";
import { useMail } from "../state/mail";
import { keyStorageName, settingsOrEmpty } from "../vm/aiSettings";
import { aiFeatureStatus, THREAD_AI_COPY, type ThreadAiStatus } from "../vm/threadAi";
import { secureGet } from "./secure";

export const streamingFetch = expoFetch as unknown as typeof fetch;

export type AiStatus = ThreadAiStatus | "loading";

/** Copy for a status that is not `ready` (web's STATUS_COPY), with "Open AI settings" offered next to it. */
export function aiStatusText(status: Exclude<AiStatus, "ready">): string {
  return status === "loading" ? "AI settings are still loading." : THREAD_AI_COPY[status];
}

/** Profiles whose key may live in SecureStore (Ollama needs none, the managed provider uses the user's token). */
function mayHaveKey(kind: string): boolean {
  return kind !== "akatsuki" && kind !== "ollama";
}

export function useAiEnv() {
  const mail = useMail();
  const view = useQuery(convexApi.ai.getSettings, {});
  const settings: AiSettings = useMemo(() => settingsOrEmpty(view ?? null), [view]);
  const keyCache = useRef<{ sig: string; keys: Record<string, string> } | null>(null);
  const sig = settings.profiles.map((p) => `${p.profileId}:${p.kind}`).join(",");
  const managedAi = mail.me?.entitlement.managedAi === true;
  const apiConfigured = !!ENV.env?.apiUrl;

  /** BYO keys for the profiles in use; throws the secure-storage error when the store cannot be read. */
  const loadKeys = useCallback(async (): Promise<Record<string, string>> => {
    if (keyCache.current?.sig === sig) return keyCache.current.keys;
    const keys: Record<string, string> = {};
    for (const p of settings.profiles) {
      if (!mayHaveKey(p.kind)) continue;
      const k = await secureGet(keyStorageName(p.profileId));
      if (k) keys[p.profileId] = k;
    }
    keyCache.current = { sig, keys };
    return keys;
  }, [sig, settings.profiles]);

  /** Forgets cached keys (after Settings stored or removed one). */
  const forgetKeys = useCallback(() => {
    keyCache.current = null;
  }, []);

  const statusWith = useCallback(
    (feature: AiFeature, keys: Record<string, string>): ThreadAiStatus =>
      aiFeatureStatus(feature, { settings, managedAi, apiConfigured, hasKey: (id) => id in keys }).status,
    [settings, managedAi, apiConfigured],
  );

  /** The status without reading keys (a missing key shows up when the feature runs). */
  const quickStatus = useCallback(
    (feature: AiFeature): AiStatus => {
      if (view === undefined) return "loading";
      const s = aiFeatureStatus(feature, { settings, managedAi, apiConfigured, hasKey: () => true }).status;
      return s;
    },
    [view, settings, managedAi, apiConfigured],
  );

  /** The full status, keys included. */
  const check = useCallback(
    async (feature: AiFeature): Promise<AiStatus> => {
      if (view === undefined) return "loading";
      const quick = quickStatus(feature);
      if (quick !== "ready") return quick;
      return statusWith(feature, await loadKeys());
    },
    [view, quickStatus, statusWith, loadKeys],
  );

  /** "Stays on this device" / "Sent to Akatsuki …" / "Leaves this device: …", or null when the feature is off. */
  const egress = useCallback(
    (feature: AiFeature): string | null => {
      const r = resolveFeature(feature, settings);
      return r.status === "ok" ? egressText(r.profile, feature) : null;
    },
    [settings],
  );

  /** The packages/ai context for one run. */
  const context = useCallback(
    async (abortSignal?: AbortSignal): Promise<FeatureContext> => {
      const keys = await loadKeys();
      return {
        settings,
        keyFor: (id: string) => keys[id],
        ...(abortSignal ? { abortSignal } : {}),
        providerOptions: { fetch: streamingFetch, browser: false },
        ...(ENV.env?.apiUrl
          ? {
              managed: {
                apiUrl: ENV.env.apiUrl,
                getAccessToken: () => getAccessToken(),
                fetch: streamingFetch,
              },
            }
          : {}),
      } as FeatureContext;
    },
    [settings, loadKeys],
  );

  const userName = mail.me?.name ?? mail.me?.email ?? "";

  return {
    settings,
    loaded: view !== undefined,
    managedAi,
    userName,
    voice: settings.voiceProfile ?? null,
    quickStatus,
    check,
    egress,
    context,
    loadKeys,
    forgetKeys,
  };
}

export type AiEnv = ReturnType<typeof useAiEnv>;

/**
 * Runs a text feature (compose, rewrite) and calls `onText` with the text so far on every delta. Resolves to the
 * final text; an `error` event or a provider failure rejects with `AiFeatureError`.
 */
export async function streamTextFeature(
  feature: "compose" | "rewrite",
  vars: Record<string, unknown>,
  ctx: FeatureContext,
  onText: (text: string) => void,
): Promise<string> {
  let text = "";
  const events: AsyncIterable<AaspEvent> = runFeature(feature, vars, ctx);
  for await (const e of events) {
    if (e.event === "delta") {
      text += e.data.text;
      onText(text);
    } else if (e.event === "error") throw new AiFeatureError(e.data.code, e.data.message, e.data.retryable);
    else if (e.event === "done") {
      const final = e.data.text ?? text;
      onText(final);
      return final;
    }
  }
  throw new AiFeatureError("provider_unavailable", "The AI stream ended early.", true);
}
