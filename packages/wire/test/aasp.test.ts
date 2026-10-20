// fixtures/aasp: the same streams crates/core::wire::aasp parses (ARCHITECTURE §1.7, §12.6).
import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  type AaspEvent,
  AaspLineParser,
  aaspReadableStream,
  encodeAaspEvent,
  parseAaspStream,
} from "../src/aasp.ts";

const dir = new URL("../fixtures/aasp/", import.meta.url);
const cases = readdirSync(dir)
  .filter((f) => f.endsWith(".txt"))
  .sort()
  .map((f) => {
    const name = f.replace(/\.txt$/, "");
    const bytes = new Uint8Array(readFileSync(new URL(f, dir)));
    const expected = JSON.parse(readFileSync(new URL(`${name}.json`, dir), "utf8")) as {
      events: AaspEvent[];
      rejected: number;
    };
    return [name, bytes, expected] as const;
  });

const streamOf = (chunks: Uint8Array[]) =>
  new ReadableStream<Uint8Array>({
    start(c) {
      for (const chunk of chunks) c.enqueue(chunk);
      c.close();
    },
  });

async function drain<T>(it: AsyncIterable<T>): Promise<T[]> {
  const out: T[] = [];
  for await (const x of it) out.push(x);
  return out;
}

describe("AASP fixtures", () => {
  it("has the shared fixtures", () => expect(cases.length).toBeGreaterThanOrEqual(10));

  it.each(cases)("%s", async (_name, bytes, expected) => {
    expect(await drain(parseAaspStream(streamOf([bytes])))).toEqual(expected.events);
    const oneByte = Array.from(bytes, (b) => Uint8Array.of(b));
    expect(await drain(parseAaspStream(streamOf(oneByte)))).toEqual(expected.events);

    const p = new AaspLineParser();
    const text = new TextDecoder().decode(bytes);
    const results = [...Array.from(text).flatMap((c) => p.feed(c)), ...p.finish()];
    expect(results.filter((r) => !r.ok)).toHaveLength(expected.rejected);

    // Every valid event re-encodes to a frame that parses back to itself.
    for (const e of expected.events)
      expect(await drain(parseAaspStream(streamOf([new TextEncoder().encode(encodeAaspEvent(e))])))).toEqual([
        e,
      ]);
  });
});

describe("aaspReadableStream", () => {
  it("encodes events into a body the parser reads back", async () => {
    const events: AaspEvent[] = [
      {
        event: "meta",
        data: { feature: "summarize", model: "anthropic/claude-sonnet-5.5", promptVersion: 1 },
      },
      { event: "delta", data: { text: "Grüße\nzwei Zeilen" } },
      {
        event: "done",
        data: {
          text: "Grüße",
          object: null,
          usage: { inputTokens: 3, outputTokens: 2 },
          finishReason: "stop",
        },
      },
    ];
    async function* gen() {
      yield* events;
    }
    expect(await drain(parseAaspStream(aaspReadableStream(gen())))).toEqual(events);
  });
});
