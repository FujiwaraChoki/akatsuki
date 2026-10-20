// Worker and apps/api clients against a stubbed fetch (no network).
import { describe, expect, it } from "vitest";
import {
  buildUrl,
  createApiClient,
  createWorkerClient,
  encodeAaspEvent,
  errorBody,
  type Id,
  readNdjson,
  WireError,
} from "../src/index.ts";

type Call = { url: string; method: string; headers: Record<string, string>; body: unknown };

function stub(responses: (Response | Error)[]) {
  const calls: Call[] = [];
  const fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({
      url: String(input),
      method: init?.method ?? "GET",
      headers: (init?.headers ?? {}) as Record<string, string>,
      body: init?.body,
    });
    const next = responses.shift();
    if (!next) throw new Error("no response queued");
    if (next instanceof Error) throw next;
    return next;
  }) as typeof globalThis.fetch;
  return { fetch, calls };
}

const json = (status: number, body: unknown, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });

const tokens = () => {
  const seen: (boolean | undefined)[] = [];
  return {
    seen,
    getAccessToken: async (o?: { forceRefresh?: boolean }) => {
      seen.push(o?.forceRefresh);
      return o?.forceRefresh ? "fresh" : "cached";
    },
  };
};

const mid = "k17abc2def3ghi4jkl5mno6pqr7stu8v" as Id<"messages">;

describe("buildUrl", () => {
  it("joins paths and drops undefined query values", () => {
    expect(buildUrl("https://sync.akatsuki.vc", "/v1/search", { q: "from:ada", cursor: undefined })).toBe(
      "https://sync.akatsuki.vc/v1/search?q=from%3Aada",
    );
    expect(buildUrl("http://127.0.0.1:8787/", "v1/x")).toBe("http://127.0.0.1:8787/v1/x");
  });
});

describe("createWorkerClient", () => {
  it("sends the bearer token and parses bodies", async () => {
    const s = stub([json(200, { messageId: mid, html: null })]);
    const t = tokens();
    const w = createWorkerClient({
      baseUrl: "https://sync.akatsuki.vc",
      fetch: s.fetch,
      getAccessToken: t.getAccessToken,
    });
    const body = await w.getMessage(mid);
    expect(body.messageId).toBe(mid);
    expect(s.calls[0]).toMatchObject({
      url: `https://sync.akatsuki.vc/v1/messages/${mid}?format=sanitized`,
      method: "GET",
      headers: { authorization: "Bearer cached" },
    });
  });

  it("retries once with a fresh token after a 401", async () => {
    const s = stub([json(401, errorBody("unauthenticated")), json(202, {})]);
    const t = tokens();
    const w = createWorkerClient({
      baseUrl: "https://sync.akatsuki.vc",
      fetch: s.fetch,
      getAccessToken: t.getAccessToken,
    });
    await w.syncAccount("acct" as Id<"mailAccounts">);
    expect(t.seen).toEqual([false, true]);
    expect(s.calls[1]?.headers.authorization).toBe("Bearer fresh");
  });

  it("turns §15 bodies into WireError, with version and Retry-After", async () => {
    const s = stub([
      json(409, errorBody("draft_version_conflict", { version: 4 })),
      json(
        429,
        { error: { code: "rate_limited", message: "Slow down.", retryable: true } },
        { "retry-after": "3" },
      ),
      new Response("<html>bad gateway</html>", { status: 502 }),
      new TypeError("fetch failed"),
    ]);
    const w = createWorkerClient({ baseUrl: "https://s", fetch: s.fetch, getAccessToken: async () => "t" });
    const draft = { baseVersion: 3, deviceId: "d", draft: {} as never };
    const conflict = await w.putDraft("draft-1", draft).catch((e: unknown) => e);
    expect(conflict).toBeInstanceOf(WireError);
    expect(conflict).toMatchObject({ code: "draft_version_conflict", version: 4, status: 409 });
    await expect(w.send({} as never)).rejects.toMatchObject({ code: "rate_limited", retryAfterMs: 3000 });
    await expect(w.search({ accountId: "a" as never, q: "x" })).rejects.toMatchObject({
      code: "internal",
      retryable: true,
      status: 502,
    });
    await expect(w.connectImap({} as never)).rejects.toMatchObject({ code: "unreachable", retryable: true });
  });

  it("streams NDJSON bodies and makes link URLs absolute", async () => {
    const lines = `${JSON.stringify({ messageId: "m2" })}\n\n${JSON.stringify({ messageId: "m1" })}`;
    const s = stub([new Response(lines), json(200, { url: "/v1/l/tok", expiresAt: 1 })]);
    const w = createWorkerClient({
      baseUrl: "https://sync.akatsuki.vc",
      fetch: s.fetch,
      getAccessToken: async () => "t",
    });
    const got: string[] = [];
    for await (const b of w.threadBodies("t1" as Id<"threads">)) got.push(b.messageId);
    expect(got).toEqual(["m2", "m1"]);
    expect((await w.createLink(mid, "1.2")).url).toBe("https://sync.akatsuki.vc/v1/l/tok");
    expect(s.calls[1]?.body).toBe(JSON.stringify({ messageId: mid, partId: "1.2" }));
  });

  it("uploads raw bytes with an encoded filename and caps prefetch", async () => {
    const s = stub([json(201, { uploadId: "u1", size: 3, sha256: "ab" })]);
    const w = createWorkerClient({ baseUrl: "https://s", fetch: s.fetch, getAccessToken: async () => "t" });
    await w.upload(new Blob(["abc"], { type: "text/plain" }), "Grüße.txt");
    expect(s.calls[0]?.headers).toMatchObject({
      "content-type": "text/plain",
      "x-filename": "Gr%C3%BC%C3%9Fe.txt",
    });
    expect(() => w.prefetch(Array.from({ length: 101 }, () => mid))).toThrow(WireError);
  });
});

describe("createApiClient", () => {
  it("streams AASP events from /v1/ai/{feature}", async () => {
    const sse =
      encodeAaspEvent({ event: "meta", data: { feature: "summarize", model: "m", promptVersion: 1 } }) +
      encodeAaspEvent({ event: "delta", data: { text: "Hi" } });
    const s = stub([new Response(sse, { headers: { "content-type": "text/event-stream" } })]);
    const api = createApiClient({
      baseUrl: "https://api.akatsuki.vc",
      fetch: s.fetch,
      getAccessToken: async () => "t",
    });
    const events = [];
    for await (const e of api.streamFeature("summarize", { promptVersion: 1, vars: { a: 1 } }))
      events.push(e.event);
    expect(events).toEqual(["meta", "delta"]);
    expect(s.calls[0]).toMatchObject({ url: "https://api.akatsuki.vc/v1/ai/summarize", method: "POST" });
  });

  it("reports errors before the stream starts", async () => {
    const s = stub([json(402, errorBody("not_entitled"))]);
    const api = createApiClient({ baseUrl: "https://api", fetch: s.fetch, getAccessToken: async () => "t" });
    const it = api.streamFeature("summarize", { promptVersion: 1, vars: {} });
    await expect(it.next()).rejects.toMatchObject({ code: "not_entitled", status: 402 });
  });

  it("sends multipart audio and billing bodies", async () => {
    const s = stub([
      json(200, { text: "hallo", durationSec: 1, language: "de" }),
      new Response(null, { status: 204 }),
    ]);
    const api = createApiClient({ baseUrl: "https://api", fetch: s.fetch, getAccessToken: async () => "t" });
    expect(
      (await api.transcribe({ audio: new Blob(["x"], { type: "audio/webm" }), language: "de" })).text,
    ).toBe("hallo");
    expect(s.calls[0]?.body).toBeInstanceOf(FormData);
    await api.cancel({ immediately: false });
    expect(s.calls[1]?.body).toBe('{"immediately":false}');
  });
});

describe("readNdjson", () => {
  it("handles lines split across chunks and a missing final newline", async () => {
    const enc = new TextEncoder();
    const parts = ['{"a":"Gr', 'üße"}\n{"a"', ":2}"];
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        for (const p of parts) c.enqueue(enc.encode(p));
        c.close();
      },
    });
    const out = [];
    for await (const x of readNdjson<{ a: unknown }>(body)) out.push(x.a);
    expect(out).toEqual(["Grüße", 2]);
  });
});
