// Runs Summarize and Suggest replies for the open conversation (§12, D-408) through packages/ai: BYO and local
// profiles go from this device straight to the chosen provider (key from SecureStore, read when a feature runs),
// the managed profile goes to apps/api with the user's token. Results stay in memory on this screen (§12.7 rule 5:
// device only).

import { AiFeatureError, instantReply, summarize, type ThreadForTranscript } from "@akatsuki/ai";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ThreadAiFeature } from "../vm/threadAi";
import { aiStatusText, useAiEnv } from "./ai";
import { errorText } from "./errors";

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

export function useThreadAi(thread: () => ThreadForTranscript | null) {
  const ai = useAiEnv();
  const [summary, setSummary] = useState<Run<SummaryResult>>({ state: "idle" });
  const [replies, setReplies] = useState<Run<ReplySuggestion[]>>({ state: "idle" });
  const abort = useRef<AbortController | null>(null);
  useEffect(() => () => abort.current?.abort(), []);

  /** Runs a feature; resolves to a sentence for a toast when it cannot run, else null. */
  const run = useCallback(
    async (feature: ThreadAiFeature): Promise<string | null> => {
      let status: Awaited<ReturnType<typeof ai.check>>;
      try {
        status = await ai.check(feature);
      } catch (e) {
        return errorText(e);
      }
      if (status !== "ready") return aiStatusText(status);
      const input = thread();
      if (!input) return "The conversation is still loading.";
      abort.current?.abort();
      const controller = new AbortController();
      abort.current = controller;
      const set = feature === "summarize" ? setSummary : setReplies;
      set({ state: "running" });
      try {
        const ctx = await ai.context(controller.signal);
        if (feature === "summarize") {
          const value = (await summarize({ thread: input, userName: ai.userName }, ctx)) as SummaryResult;
          setSummary({ state: "done", value });
        } else {
          const value = await instantReply({ thread: input, userName: ai.userName, voice: ai.voice }, ctx);
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
    [ai, thread],
  );

  return { status: ai.quickStatus, run, summary, replies, egress: ai.egress };
}
