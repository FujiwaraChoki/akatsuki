// The AI layer (packages/ai: AI SDK 7, providers, prompt pack) is loaded on demand in its own chunk, so the
// mail client's first paint never pays for it (D-282). `loadAi()` starts the import; `useAiModule()` renders
// with it once it is there (it is requested on idle after sign-in, so it is normally ready before any AI
// action is shown).
import { useEffect, useSyncExternalStore } from "react";

export type AiModule = typeof import("@akatsuki/ai") & { promptVersion: (feature: string) => number };

let mod: AiModule | null = null;
let loading: Promise<AiModule> | null = null;
const listeners = new Set<() => void>();

export function loadAi(): Promise<AiModule> {
  if (mod) return Promise.resolve(mod);
  if (!loading)
    loading = Promise.all([import("@akatsuki/ai"), import("@akatsuki/prompts")])
      .then(([ai, prompts]) => {
        mod = { ...ai, promptVersion: (f: string) => prompts.promptVersion(f as never) };
        for (const l of [...listeners]) l();
        return mod;
      })
      .catch((e) => {
        loading = null;
        throw e;
      });
  return loading;
}

export function aiModule(): AiModule | null {
  return mod;
}

const subscribe = (cb: () => void) => {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
};

/** The AI module, or null while it loads (the hook starts the load). */
export function useAiModule(): AiModule | null {
  const m = useSyncExternalStore(subscribe, aiModule, aiModule);
  useEffect(() => {
    if (!m) void loadAi().catch(() => undefined);
  }, [m]);
  return m;
}

/** Warms the chunk when the browser is idle (called once after sign-in). */
export function preloadAi(): void {
  const run = () => void loadAi().catch(() => undefined);
  const w = globalThis as { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => void };
  if (w.requestIdleCallback) w.requestIdleCallback(run, { timeout: 4000 });
  else setTimeout(run, 1500);
}
