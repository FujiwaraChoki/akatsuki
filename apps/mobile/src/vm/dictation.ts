// Dictation in the composer (ARCHITECTURE §12.1 `dictation`, tier `transcription`; §12.7 rules 1, 4 and 6; D-128).
// Record on the phone, then transcribe with the user's dictation route: Akatsuki AI through apps/api
// `POST /v1/ai/transcribe` (OpenAI behind it, may keep audio up to 30 days, and the label says so), the user's own
// OpenAI key straight from the phone, or a Whisper server with an OpenAI-compatible endpoint on their network
// (audio stays on that network). Nothing is recorded until the user has seen where the audio goes and tapped
// Record; the recording file is deleted once it is transcribed or discarded. Pure parts live here.
import { AUDIO_MAX_BYTES, AUDIO_MAX_SECONDS, type ProviderKind } from "@akatsuki/ai";

/** §8: at most 10 minutes per recording (apps/api refuses longer audio). */
export const MAX_RECORDING_MS = AUDIO_MAX_SECONDS * 1000;
export const MAX_AUDIO_BYTES = AUDIO_MAX_BYTES;

/** Provider kinds that can transcribe (`transcriptionModelFor` in packages/ai, plus the managed provider). */
export const DICTATION_KINDS: ReadonlySet<ProviderKind> = new Set([
  "akatsuki",
  "openai",
  "openai_compatible",
]);

export type DictationPhase =
  | { kind: "idle" }
  /** The sheet is open: where the audio goes is on screen, nothing is recording. */
  | { kind: "ready" }
  | { kind: "starting" }
  | { kind: "recording"; startedAt: number }
  | { kind: "transcribing" }
  | { kind: "error"; message: string };

export type DictationEvent =
  | { type: "open" }
  | { type: "start" }
  | { type: "recording"; at: number }
  | { type: "stop" }
  | { type: "text" }
  | { type: "fail"; message: string }
  | { type: "close" };

/** Dictation state machine; events that do not apply in a phase are ignored. */
export function dictationReducer(p: DictationPhase, e: DictationEvent): DictationPhase {
  switch (e.type) {
    case "open":
      return p.kind === "idle" || p.kind === "error" ? { kind: "ready" } : p;
    case "start":
      return p.kind === "ready" || p.kind === "error" ? { kind: "starting" } : p;
    case "recording":
      return p.kind === "starting" ? { kind: "recording", startedAt: e.at } : p;
    case "stop":
      return p.kind === "recording" ? { kind: "transcribing" } : p;
    case "text":
      return p.kind === "transcribing" ? { kind: "idle" } : p;
    case "fail":
      return { kind: "error", message: e.message };
    case "close":
      return { kind: "idle" };
  }
}

/** "0:42" for the recording timer. */
export function elapsedLabel(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** Inserts a transcript at the cursor with sensible spaces around it; returns the text and the new cursor. */
export function insertTranscript(
  text: string,
  selection: { start: number; end: number } | null,
  transcript: string,
): { text: string; cursor: number } {
  const t = transcript.trim();
  const start = Math.max(0, Math.min(selection?.start ?? text.length, text.length));
  const end = Math.max(start, Math.min(selection?.end ?? text.length, text.length));
  if (!t) return { text, cursor: end };
  const before = text.slice(0, start);
  const after = text.slice(end);
  const lead = before && !/\s$/.test(before) ? " " : "";
  const trail = after && !/^\s/.test(after) ? " " : after ? "" : " ";
  const insert = `${lead}${t}${trail}`;
  return { text: before + insert + after, cursor: before.length + insert.length };
}

/** The media type of a recording file, from its extension (expo-audio writes .m4a natively, .webm on web). */
export function audioMimeType(uri: string): string {
  const ext = /\.([a-z0-9]+)(?:[?#].*)?$/i.exec(uri)?.[1]?.toLowerCase() ?? "";
  switch (ext) {
    case "m4a":
    case "mp4":
    case "aac":
      return "audio/mp4";
    case "wav":
      return "audio/wav";
    case "mp3":
      return "audio/mpeg";
    case "ogg":
      return "audio/ogg";
    case "webm":
      return "audio/webm";
    default:
      return "audio/mp4";
  }
}

/** A filename the transcription APIs accept (they sniff the format from it). */
export function audioFilename(mimeType: string): string {
  if (mimeType === "audio/webm") return "dictation.webm";
  if (mimeType === "audio/ogg") return "dictation.ogg";
  if (mimeType === "audio/wav") return "dictation.wav";
  if (mimeType === "audio/mpeg") return "dictation.mp3";
  return "dictation.m4a";
}

/**
 * A `multipart/form-data` body as bytes: text fields first, then one file part. React Native cannot build a Blob
 * from bytes (its BlobManager refuses ArrayBuffer views), so neither FormData-with-bytes nor the AI SDK's
 * transcription call (which wraps the audio in a Blob) works on a phone; this builder does. Used for apps/api
 * `POST /v1/ai/transcribe` (file part `audio`) and OpenAI-style `POST …/audio/transcriptions` (part `file`).
 * `boundary` is passed in so the output is testable.
 */
export function multipartBody(
  fields: Readonly<Record<string, string | undefined>>,
  file: { name: string; filename: string; mimeType: string; bytes: Uint8Array },
  boundary: string,
): { body: Uint8Array; contentType: string } {
  if (!/^[A-Za-z0-9'()+_,\-./:=?]{1,70}$/.test(boundary)) throw new Error("bad boundary");
  const safe = (v: string) => v.replace(/["\r\n\\]/g, "_");
  const enc = new TextEncoder();
  const crlf = "\r\n";
  const parts: Uint8Array[] = [];
  for (const [name, value] of Object.entries(fields)) {
    if (value === undefined) continue;
    parts.push(
      enc.encode(
        `--${boundary}${crlf}content-disposition: form-data; name="${safe(name)}"${crlf}${crlf}${value.replace(/[\r\n]/g, " ")}${crlf}`,
      ),
    );
  }
  parts.push(
    enc.encode(
      `--${boundary}${crlf}content-disposition: form-data; name="${safe(file.name)}"; filename="${safe(file.filename)}"${crlf}content-type: ${safe(file.mimeType)}${crlf}${crlf}`,
    ),
    file.bytes,
    enc.encode(`${crlf}--${boundary}--${crlf}`),
  );
  const body = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let off = 0;
  for (const p of parts) {
    body.set(p, off);
    off += p.length;
  }
  return { body, contentType: `multipart/form-data; boundary=${boundary}` };
}

/** OpenAI's API origin (the `openai` provider without a baseUrl). */
export const OPENAI_BASE_URL = "https://api.openai.com/v1";

/** `…/audio/transcriptions` for an OpenAI or OpenAI-compatible profile (baseUrl includes `/v1`, as in the AI SDK). */
export function transcriptionsUrl(profile: { kind: string; baseUrl?: string }): string {
  const base = (profile.baseUrl ?? (profile.kind === "openai" ? OPENAI_BASE_URL : "")).replace(/\/+$/, "");
  if (!base) throw new Error("Set the server address.");
  return `${base}/audio/transcriptions`;
}

/** The language hint from the device locale ("de-CH" → "de"), or undefined. */
export function languageHint(locale: string | null | undefined): string | undefined {
  const l = locale?.split(/[-_]/)[0]?.toLowerCase();
  return l && /^[a-z]{2,3}$/.test(l) ? l : undefined;
}
