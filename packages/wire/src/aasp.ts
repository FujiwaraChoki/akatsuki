// Akatsuki AI Stream Protocol (ARCHITECTURE §12.6): types, encoder and the TypeScript parser. Rust twin:
// crates/core::wire::aasp. Both run fixtures/aasp/*.txt (each with a *.json of the valid events and the number
// of rejected frames).
//
//   event: meta     data: {"feature":"summarize","model":"anthropic/claude-sonnet-5.5","promptVersion":1}
//   event: delta    data: {"text":"…"}                       (text features)
//   event: partial  data: {"object":{…deep partial…}}        (object features)
//   event: done     data: {"text":null,"object":{…},"usage":{"inputTokens":0,"outputTokens":0},"finishReason":"stop"}
//   event: error    data: {"code":"provider_unavailable","message":"…","retryable":true}
//
// Parser rules, as in Rust: `\n`, `\r\n` and `\r` line endings (also when a `\r\n` is split across chunks), `:`
// comment lines, multi-line `data` joined with `\n`, a frame without `data` is ignored, a frame with an unknown
// event name (including the SSE default `message`) or data that is not JSON is rejected and skipped, and an
// unterminated last frame is flushed.
import type { ErrorPayload } from "./errors.ts";

export interface AaspUsage {
  inputTokens: number;
  outputTokens: number;
}

/** The `error` event payload: the §15 error object. */
export type AaspError = ErrorPayload;

export type AaspEvent =
  | { event: "meta"; data: { feature: string; model: string; promptVersion: number } }
  | { event: "delta"; data: { text: string } }
  | { event: "partial"; data: { object: unknown } }
  | {
      event: "done";
      data: { text: string | null; object: unknown; usage: AaspUsage; finishReason: string };
    }
  | { event: "error"; data: AaspError };

export type AaspEventName = AaspEvent["event"];
export const AASP_EVENTS: readonly AaspEventName[] = ["meta", "delta", "partial", "done", "error"];
export const AASP_CONTENT_TYPE = "text/event-stream";

/** One SSE frame: `event: <name>\ndata: <json>\n\n`. JSON never contains a raw newline. */
export function encodeAaspEvent(e: AaspEvent): string {
  return `event: ${e.event}\ndata: ${JSON.stringify(e.data)}\n\n`;
}

/** Turns events into a `text/event-stream` body. */
export function aaspReadableStream(events: AsyncIterable<AaspEvent>): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  const iterator = events[Symbol.asyncIterator]();
  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      const next = await iterator.next();
      if (next.done) controller.close();
      else controller.enqueue(encoder.encode(encodeAaspEvent(next.value)));
    },
    async cancel() {
      await iterator.return?.();
    },
  });
}

const KNOWN: ReadonlySet<string> = new Set(AASP_EVENTS);

export type AaspFrameResult =
  | { ok: true; event: AaspEvent }
  | { ok: false; reason: "unknown_event" | "bad_data" };

/** Incremental line parser; `feed` and `finish` return the frames they complete. */
export class AaspLineParser {
  private pending = "";
  private event: string | undefined;
  private data: string[] = [];

  feed(text: string): AaspFrameResult[] {
    this.pending += text;
    const out: AaspFrameResult[] = [];
    for (;;) {
      const m = /[\r\n]/.exec(this.pending);
      if (!m) break;
      const pos = m.index;
      // A lone "\r" at the end may be the first half of "\r\n": wait for more.
      if (this.pending[pos] === "\r" && pos + 1 === this.pending.length) break;
      const line = this.pending.slice(0, pos);
      const skip = this.pending[pos] === "\r" && this.pending[pos + 1] === "\n" ? 2 : 1;
      this.pending = this.pending.slice(pos + skip);
      const r = this.line(line);
      if (r) out.push(r);
    }
    return out;
  }

  finish(): AaspFrameResult[] {
    const out: AaspFrameResult[] = [];
    if (this.pending !== "") {
      const rest = this.pending;
      this.pending = "";
      const r = this.line(rest);
      if (r) return [r];
    }
    const r = this.dispatch();
    if (r) out.push(r);
    return out;
  }

  private line(line: string): AaspFrameResult | null {
    if (line === "") return this.dispatch();
    if (line.startsWith(":")) return null;
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    const raw = colon === -1 ? "" : line.slice(colon + 1);
    const value = raw.startsWith(" ") ? raw.slice(1) : raw;
    if (field === "event") this.event = value;
    else if (field === "data") this.data.push(value);
    return null;
  }

  private dispatch(): AaspFrameResult | null {
    if (this.data.length === 0) {
      this.event = undefined;
      return null;
    }
    const name = this.event ?? "message";
    const data = this.data.join("\n");
    this.event = undefined;
    this.data = [];
    if (!KNOWN.has(name)) return { ok: false, reason: "unknown_event" };
    try {
      return { ok: true, event: { event: name, data: JSON.parse(data) } as AaspEvent };
    } catch {
      return { ok: false, reason: "bad_data" };
    }
  }
}

/** Parses an AASP body into its valid events (rejected frames are skipped). */
export async function* parseAaspStream(body: ReadableStream<Uint8Array>): AsyncGenerator<AaspEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const parser = new AaspLineParser();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      const text = decoder.decode(value ?? new Uint8Array(), { stream: !done });
      for (const r of done ? [...parser.feed(text), ...parser.finish()] : parser.feed(text))
        if (r.ok) yield r.event;
      if (done) break;
    }
  } finally {
    reader.releaseLock();
  }
}
