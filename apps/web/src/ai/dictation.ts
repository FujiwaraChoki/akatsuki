// Dictation (§12.1 `dictation`): record with MediaRecorder (webm/opus where available), then transcribe with
// the user's route: Akatsuki managed through apps/api `POST /v1/ai/transcribe` (OpenAI behind it, not zero
// data retention, D-128, and the egress label says so), BYO OpenAI, or a local Whisper server with an
// OpenAI-compatible endpoint so audio stays on the device. Recordings stop at 10 minutes (§8 limit).

import { WireError } from "@akatsuki/wire";
import { useCallback, useEffect, useRef, useState } from "react";
import { loadAi } from "./module.ts";
import type { UseAi } from "./useAi.ts";

export const MAX_RECORDING_MS = 10 * 60 * 1000;

export type DictationState =
  | { status: "idle" }
  | { status: "recording"; startedAt: number }
  | { status: "transcribing" }
  | { status: "error"; error: unknown };

function pickMime(): string | undefined {
  const MR = (globalThis as { MediaRecorder?: { isTypeSupported?: (t: string) => boolean } }).MediaRecorder;
  for (const t of ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg"])
    if (MR?.isTypeSupported?.(t)) return t;
  return undefined;
}

export function dictationSupported(): boolean {
  return (
    typeof navigator !== "undefined" &&
    !!navigator.mediaDevices?.getUserMedia &&
    typeof (globalThis as { MediaRecorder?: unknown }).MediaRecorder !== "undefined"
  );
}

/** Transcribes one recording with the user's dictation route. */
export async function transcribe(ai: UseAi, audio: Blob, signal?: AbortSignal): Promise<string> {
  const mod = await loadAi();
  if (!ai.settings.consent.grantedAt)
    throw new WireError({
      code: "consent_required",
      message: "Turn on AI features in Settings first.",
      retryable: false,
    });
  const r = mod.resolveFeature("dictation", ai.settings as never);
  if (r.status === "off")
    throw new WireError({ code: "ai_off", message: "Dictation is turned off.", retryable: false });
  if (r.status === "error")
    throw new WireError({ code: "ai_model_required", message: "Choose a model.", retryable: false });
  if (r.managed) {
    if (!ai.managedAi)
      throw new WireError({ code: "not_entitled", message: "Akatsuki AI is part of Pro.", retryable: false });
    return (await ai.env.api.transcribe({ audio }, signal)).text;
  }
  const bytes = new Uint8Array(await audio.arrayBuffer());
  const out = await mod.dictate(
    bytes,
    { settings: ai.settings as never, keyFor: (id) => ai.keys.get(id) },
    { abortSignal: signal },
  );
  return out.text;
}

/** Record → stop → transcribe → `onText`. */
export function useDictation(ai: UseAi, onText: (text: string) => void) {
  const [state, setState] = useState<DictationState>({ status: "idle" });
  const rec = useRef<{
    recorder: MediaRecorder;
    stream: MediaStream;
    timer: ReturnType<typeof setTimeout>;
  } | null>(null);
  const cbRef = useRef(onText);
  cbRef.current = onText;

  const cleanup = useCallback(() => {
    const r = rec.current;
    if (!r) return;
    clearTimeout(r.timer);
    for (const t of r.stream.getTracks()) t.stop();
    rec.current = null;
  }, []);
  useEffect(() => cleanup, [cleanup]);

  const start = useCallback(async () => {
    if (rec.current) return;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const mimeType = pickMime();
      const recorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
      const chunks: Blob[] = [];
      recorder.ondataavailable = (e) => {
        if (e.data.size > 0) chunks.push(e.data);
      };
      recorder.onstop = async () => {
        cleanup();
        const blob = new Blob(chunks, { type: recorder.mimeType || mimeType || "audio/webm" });
        if (blob.size === 0) return setState({ status: "idle" });
        setState({ status: "transcribing" });
        try {
          const text = (await transcribe(ai, blob)).trim();
          if (text) cbRef.current(text);
          setState({ status: "idle" });
        } catch (error) {
          setState({ status: "error", error });
        }
      };
      recorder.start(1000);
      const timer = setTimeout(() => recorder.state !== "inactive" && recorder.stop(), MAX_RECORDING_MS);
      rec.current = { recorder, stream, timer };
      setState({ status: "recording", startedAt: Date.now() });
    } catch (error) {
      cleanup();
      setState({
        status: "error",
        error:
          (error as { name?: string })?.name === "NotAllowedError"
            ? new WireError({
                code: "invalid_argument",
                message: "Microphone access was denied.",
                retryable: false,
              })
            : error,
      });
    }
  }, [ai, cleanup]);

  const stop = useCallback(() => {
    const r = rec.current;
    if (r && r.recorder.state !== "inactive") r.recorder.stop();
  }, []);

  const toggle = useCallback(() => {
    if (rec.current) stop();
    else void start();
  }, [start, stop]);

  return { state, start, stop, toggle };
}
