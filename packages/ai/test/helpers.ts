// Test helpers: AI SDK mock models (no keys, no network) and a fetch stub.
import { simulateReadableStream } from "ai";
import { MockLanguageModelV4 } from "ai/test";
import type { AaspEvent } from "../src/aasp.ts";

const usage = (input: number, output: number) => ({
  inputTokens: { total: input, noCache: input, cacheRead: undefined, cacheWrite: undefined },
  outputTokens: { total: output, text: output, reasoning: undefined },
});

export interface Captured {
  calls: Parameters<MockLanguageModelV4["doStream"]>[0][];
}

/** A streaming mock that emits `pieces` as text deltas, then finishes with the given usage. */
export function mockModel(pieces: string[], opts: { input?: number; output?: number; fail?: Error } = {}) {
  const captured: Captured = { calls: [] };
  const model = new MockLanguageModelV4({
    doStream: async (options) => {
      captured.calls.push(options);
      if (opts.fail) throw opts.fail;
      return {
        stream: simulateReadableStream({
          chunks: [
            { type: "text-start", id: "t1" },
            ...pieces.map((delta) => ({ type: "text-delta" as const, id: "t1", delta })),
            { type: "text-end", id: "t1" },
            {
              type: "finish",
              finishReason: { unified: "stop", raw: undefined },
              logprobs: undefined,
              usage: usage(opts.input ?? 100, opts.output ?? 20),
            },
          ],
        }),
      };
    },
  });
  return { model, captured };
}

/** Splits a JSON string into small pieces so partial-object streaming is exercised. */
export function jsonPieces(value: unknown, size = 7): string[] {
  const s = JSON.stringify(value);
  const out: string[] = [];
  for (let i = 0; i < s.length; i += size) out.push(s.slice(i, i + size));
  return out;
}

export async function drain(events: AsyncIterable<AaspEvent>): Promise<AaspEvent[]> {
  const out: AaspEvent[] = [];
  for await (const e of events) out.push(e);
  return out;
}

/** A fetch stub that records requests and answers from `respond`. */
export function stubFetch(respond: (url: string, init: RequestInit) => Response | Promise<Response>) {
  const requests: { url: string; init: RequestInit; body: unknown }[] = [];
  const fetch = (async (input: string | URL | Request, init: RequestInit = {}) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    let body: unknown = init.body;
    if (typeof init.body === "string") {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = init.body;
      }
    }
    requests.push({ url, init, body });
    return respond(url, init);
  }) as typeof globalThis.fetch;
  return { fetch, requests };
}

export const CONSENTED = { grantedAt: 1, version: 1, autoTriage: true, syncVoiceProfile: false };
