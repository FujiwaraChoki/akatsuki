// Runs Summarize and Suggest replies for the open conversation (§12, D-408) through packages/ai: BYO and local
// profiles go from this device straight to the chosen provider (key from SecureStore), the managed profile goes
// to apps/api with the user's token. Results stay in memory on this screen (§12.7 rule 5: device only).

import {
  AiFeatureError,
  type AiSettings,
  instantReply,
  summarize,
  type ThreadForTranscript,
} from "@akatsuki/ai";
import { api as convexApi } from "@akatsuki/convex";
import { getAccessToken } from "@akatsuki/wire";
import { useQuery } from "convex/react";
import { fetch as expoFetch } from "expo/fetch";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ENV } from "../env";
import { useMail } from "../state/mail";
import { keyStorageName, settingsOrEmpty } from "../vm/aiSettings";
import { THREAD_AI_COPY, type ThreadAiFeature, type ThreadAiStatus, threadAiStatus } from "../vm/threadAi";
import { errorText } from "./errors";
import { secureGet } from "./secure";

export interface SummaryResult {
  tldr: string;
  bullets: string[];
  asks: string[];
  deadlines: { text: string; due: string | null }[];
}
export interface ReplySuggestion {
  label: string;
  body: string;
}

type Run<T> =
  | { state: "idle" }
  | { state: "running" }
  | { state: "done"; value: T }
  | { state: "error"; message: string };

const streamingFetch = expoFetch as unknown as typeof fetch;

export function useThreadAi(thread: () => ThreadForTranscript | null) {
  const mail = useMail();
  const view = useQuery(convexApi.ai.getSettings, {});
  const settings: AiSettings = useMemo(() => settingsOrEmpty(view ?? null), [view]);
  const [keys, setKeys] = useState<Record<string, string>>({});
  const [summary, setSummary] = useState<Run<SummaryResult>>({ state: "idle" });
  const [replies, setReplies] = useState<Run<ReplySuggestion[]>>({ state: "idle" });
  const abort = useRef<AbortController | null>(null);

  // BYO keys live only in SecureStore; read the ones the routes use once settings are known.
  useEffect(() => {
    let live = true;
    void (async () => {
      const out: Record<string, string> = {};
      for (const p of settings.profiles) {
        const k = await secureGet(keyStorageName(p.profileId)).catch(() => null);
        if (k) out[p.profileId] = k;
      }
      if (live) setKeys(out);
    })();
    return () => {
      live = false;
    };
  }, [settings.profiles]);
  useEffect(() => () => abort.current?.abort(), []);

  const status = useCallback(
    (feature: ThreadAiFeature): ThreadAiStatus =>
      view === undefined
        ? "ready"
        : threadAiStatus(feature, {
            settings,
            managedAi: mail.me?.entitlement.managedAi === true,
            apiConfigured: !!ENV.env?.apiUrl,
            hasKey: (id) => id in keys,
          }).status,
    [view, settings, mail.me?.entitlement.managedAi, keys],
  );

  const run = useCallback(
    async (feature: ThreadAiFeature): Promise<string | null> => {
      const s = status(feature);
      if (s !== "ready") return THREAD_AI_COPY[s];
      const input = thread();
      if (!input) return "The conversation is still loading.";
      abort.current?.abort();
      const controller = new AbortController();
      abort.current = controller;
      const ctx = {
        settings,
        keyFor: (id: string) => keys[id],
        abortSignal: controller.signal,
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
      };
      const userName = mail.me?.name ?? mail.me?.email ?? "";
      const set = feature === "summarize" ? setSummary : setReplies;
      set({ state: "running" });
      try {
        if (feature === "summarize") {
          const value = (await summarize({ thread: input, userName }, ctx as never)) as SummaryResult;
          setSummary({ state: "done", value });
        } else {
          const value = await instantReply(
            { thread: input, userName, voice: settings.voiceProfile ?? null },
            ctx as never,
          );
          setReplies({ state: "done", value });
        }
        return null;
      } catch (e) {
        if (controller.signal.aborted) {
          set({ state: "idle" });
          return null;
        }
        const message = e instanceof AiFeatureError ? e.message : errorText(e);
        set({ state: "error", message });
        return null;
      }
    },
    [status, thread, settings, keys, mail.me],
  );

  return { status, run, summary, replies };
}
