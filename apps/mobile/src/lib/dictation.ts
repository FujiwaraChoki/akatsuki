// Dictation transport (ARCHITECTURE §12.1 `dictation`, §8 `POST /v1/ai/transcribe`, §12.7 rules 4 and 6, D-128).
// The route comes from packages/ai (`resolveFeature("dictation")`): the managed provider goes to apps/api with the
// user's token (OpenAI behind it); an OpenAI profile goes straight from the phone to api.openai.com with the key
// from SecureStore; an OpenAI-compatible profile goes to the user's Whisper server. The multipart body is built
// as bytes (vm/dictation.ts) because React Native cannot put bytes into a Blob.
import { AiFeatureError, resolveFeature } from "@akatsuki/ai";
import { createHttpClient, type TranscribeResponse } from "@akatsuki/wire";
import { getLocales } from "expo-localization";
import { ENV } from "../env";
import {
  audioFilename,
  audioMimeType,
  languageHint,
  MAX_AUDIO_BYTES,
  multipartBody,
  transcriptionsUrl,
} from "../vm/dictation";
import { type AiEnv, aiStatusText, streamingFetch } from "./ai";
import { newClientId } from "./ids";
import { readRecording } from "./recordingFile";

function boundary(): string {
  return `----akatsuki-${newClientId().replace(/-/g, "")}`;
}

function deviceLanguage(): string | undefined {
  try {
    return languageHint(getLocales()[0]?.languageTag);
  } catch {
    return undefined;
  }
}

/** Transcribes a finished recording with the user's dictation route. Rejects with a sentence for the user. */
export async function transcribeRecording(ai: AiEnv, uri: string, signal?: AbortSignal): Promise<string> {
  const status = await ai.check("dictation");
  if (status !== "ready") throw new AiFeatureError(status, aiStatusText(status));
  const r = resolveFeature("dictation", ai.settings);
  if (r.status !== "ok") throw new AiFeatureError("ai_off", aiStatusText("off"));
  const bytes = await readRecording(uri);
  if (bytes.byteLength === 0) throw new AiFeatureError("invalid_argument", "The recording is empty.");
  if (bytes.byteLength > MAX_AUDIO_BYTES)
    throw new AiFeatureError("payload_too_large", "The recording is too large.");
  const mimeType = audioMimeType(uri);
  const file = { filename: audioFilename(mimeType), mimeType, bytes };
  const language = deviceLanguage();

  if (r.managed) {
    if (!ENV.env?.apiUrl) throw new AiFeatureError("no_api", aiStatusText("no_api"));
    const mp = multipartBody({ language }, { name: "audio", ...file }, boundary());
    const http = createHttpClient({ baseUrl: ENV.env.apiUrl, fetch: streamingFetch });
    const res = await http.json<TranscribeResponse>("POST", "/v1/ai/transcribe", {
      body: mp.body as unknown as BodyInit,
      headers: { "content-type": mp.contentType },
      ...(signal ? { signal } : {}),
    });
    return res.text;
  }

  const key = (await ai.loadKeys())[r.profile.profileId];
  if (r.profile.kind === "openai" && !key) throw new AiFeatureError("key_missing", aiStatusText("no_key"));
  const mp = multipartBody(
    { model: r.model, language, response_format: "json" },
    { name: "file", ...file },
    boundary(),
  );
  const res = await streamingFetch(transcriptionsUrl(r.profile), {
    method: "POST",
    headers: { "content-type": mp.contentType, ...(key ? { authorization: `Bearer ${key}` } : {}) },
    body: mp.body as unknown as BodyInit,
    ...(signal ? { signal } : {}),
  }).catch((e: unknown) => {
    if ((e as { name?: string }).name === "AbortError") throw e;
    throw new AiFeatureError(
      "unreachable",
      `Could not reach ${new URL(transcriptionsUrl(r.profile)).host}.`,
      true,
    );
  });
  const body = (await res.json().catch(() => null)) as {
    text?: unknown;
    error?: { message?: unknown };
  } | null;
  if (!res.ok) {
    const msg = typeof body?.error?.message === "string" ? body.error.message.slice(0, 200) : null;
    throw new AiFeatureError(
      res.status === 401 ? "key_invalid" : "provider_unavailable",
      msg ?? `The transcription server answered ${res.status}.`,
      res.status >= 500,
    );
  }
  if (typeof body?.text !== "string")
    throw new AiFeatureError("provider_unavailable", "No transcript came back.");
  return body.text;
}
