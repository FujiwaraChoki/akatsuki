// Dictation (§12.1 `dictation`, tier `transcription`): OpenAI through the AI SDK, or a local Whisper server
// with an OpenAI-compatible `/v1/audio/transcriptions` endpoint (whisper.cpp server, faster-whisper-server,
// LocalAI, ...) so audio can stay on the device. Managed users go through apps/api `POST /v1/ai/transcribe`.
import { createOpenAI } from "@ai-sdk/openai";
import { type TranscriptionModel, transcribe } from "ai";
import { resolveFeature } from "./resolve.ts";
import { AiFeatureError } from "./run.ts";
import type { AiProfile, AiSettings } from "./settings.ts";

/** OpenAI transcription models verified in @ai-sdk/openai 4.0.97 (`OpenAITranscriptionModelId`). */
export const OPENAI_TRANSCRIPTION_MODELS = [
  "gpt-4o-transcribe",
  "gpt-4o-mini-transcribe",
  "whisper-1",
] as const;

export const AUDIO_MAX_BYTES = 25 * 1024 * 1024;
export const AUDIO_MAX_SECONDS = 600;
export const PROMPT_HINT_MAX = 500;
export const AUDIO_MEDIA_TYPES = [
  "audio/webm",
  "audio/ogg",
  "audio/mp4",
  "audio/m4a",
  "audio/x-m4a",
  "audio/wav",
  "audio/x-wav",
  "audio/wave",
  "audio/mpeg",
  "audio/mp3",
] as const;

export interface TranscriptionResult {
  text: string;
  durationSec: number;
  language: string | null;
}

export interface TranscribeOptions {
  /** BCP-47 hint, e.g. "en" or "de-CH". */
  language?: string;
  /** Vocabulary hint (names, jargon), at most 500 characters. */
  prompt?: string;
  abortSignal?: AbortSignal;
}

/**
 * The transcription model for a profile: `openai` (api.openai.com, or its baseUrl) or `openai_compatible`
 * (a local Whisper server; key optional). Other kinds have no transcription API.
 */
export function transcriptionModelFor(
  profile: Pick<AiProfile, "profileId" | "kind" | "baseUrl">,
  model: string,
  apiKey: string | undefined,
  fetch?: typeof globalThis.fetch,
): TranscriptionModel {
  if (profile.kind === "openai") {
    if (!apiKey) throw new AiFeatureError("key_missing", "Key not set on this device.");
    return createOpenAI({ apiKey, baseURL: profile.baseUrl, fetch }).transcription(model);
  }
  if (profile.kind === "openai_compatible") {
    if (!profile.baseUrl) throw new AiFeatureError("ai_model_required", "Set the server URL.");
    // Local servers usually ignore the key; the OpenAI client still sends a bearer header.
    return createOpenAI({ apiKey: apiKey ?? "local", baseURL: profile.baseUrl, fetch }).transcription(model);
  }
  throw new AiFeatureError(
    "unsupported",
    "This provider cannot transcribe audio. Choose OpenAI or a local Whisper server.",
  );
}

/** Runs one transcription with an explicit model (apps/api and the BYO path both end here). */
export async function transcribeAudio(
  model: TranscriptionModel,
  audio: Uint8Array,
  options: TranscribeOptions = {},
): Promise<TranscriptionResult> {
  if (audio.byteLength > AUDIO_MAX_BYTES)
    throw new AiFeatureError("payload_too_large", "The recording is too large.");
  const providerOptions: Record<string, Record<string, string>> = {};
  const openai: Record<string, string> = {};
  if (options.language) openai.language = options.language.split("-")[0]!.toLowerCase();
  if (options.prompt) openai.prompt = Array.from(options.prompt).slice(0, PROMPT_HINT_MAX).join("");
  if (Object.keys(openai).length > 0) providerOptions.openai = openai;
  const result = await transcribe({
    model,
    audio,
    providerOptions,
    abortSignal: options.abortSignal,
  });
  return {
    text: result.text,
    durationSec: result.durationInSeconds ?? 0,
    language: result.language ?? null,
  };
}

/** BYO dictation: resolves the `dictation` route and transcribes on the chosen provider. */
export async function dictate(
  audio: Uint8Array,
  ctx: {
    settings: AiSettings;
    keyFor: (profileId: string) => string | undefined;
    fetch?: typeof globalThis.fetch;
  },
  options: TranscribeOptions = {},
): Promise<TranscriptionResult> {
  if (!ctx.settings.consent.grantedAt)
    throw new AiFeatureError("consent_required", "Turn on AI features in Settings first.");
  const r = resolveFeature("dictation", ctx.settings);
  if (r.status === "off") throw new AiFeatureError("ai_off", "Dictation is turned off.");
  if (r.status === "error") throw new AiFeatureError("ai_model_required", "Choose a model.");
  // Akatsuki AI dictation is a server call (apps/api `POST /v1/ai/transcribe`), not a local provider.
  if (r.managed)
    throw new AiFeatureError(
      "unsupported",
      "Akatsuki AI dictation goes through apps/api POST /v1/ai/transcribe.",
    );
  const model = transcriptionModelFor(r.profile, r.model, ctx.keyFor(r.profile.profileId), ctx.fetch);
  return transcribeAudio(model, audio, options);
}
