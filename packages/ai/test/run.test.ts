import { APICallError } from "ai";
import { describe, expect, it } from "vitest";
import { aaspReadableStream, encodeAaspEvent } from "../src/aasp.ts";
import {
  extractActions,
  instantReply,
  nlSearchToQuery,
  rewrite,
  runFeature,
  summarize,
  triage,
  voiceProfile,
  writeInVoice,
} from "../src/features.ts";
import { audioCostMicros, estimateCostMicros, tokenCostMicros } from "../src/pricing.ts";
import { escalateOnInvalidOutput } from "../src/routing.ts";
import { type AiFeatureError, collectFeature, streamFeature, varsChars } from "../src/run.ts";
import type { AiSettings } from "../src/settings.ts";
import { parseAaspStream } from "../src/sse.ts";
import { CONSENTED, drain, jsonPieces, mockModel, stubFetch } from "./helpers.ts";

const thread = {
  timezone: "Europe/Zurich",
  messages: [
    {
      from: { name: "Mia Example", email: "mia@example.com" },
      to: [{ email: "sam@example.org" }],
      date: 1792483200000,
      text: "Can you confirm the head count by Friday?",
    },
  ],
};

const summary = {
  tldr: "Head count due Friday.",
  bullets: ["a", "b", "c", "d", "e", "f"],
  asks: [],
  deadlines: [],
};

describe("streamFeature (mock models, no network)", () => {
  it("streams a text feature as meta, deltas, done", async () => {
    const { model, captured } = mockModel(["Hi Lee,", " Thursday", " works."]);
    const events = await drain(
      streamFeature({
        feature: "rewrite",
        vars: { text: "hi lee thursday works", style: "fix", voice: null },
        choice: { model, modelId: "claude-sonnet-5-5", kind: "anthropic", tier: "default" },
      }),
    );
    expect(events.map((e) => e.event)).toEqual(["meta", "delta", "delta", "delta", "done"]);
    expect(events[0]).toEqual({
      event: "meta",
      data: { feature: "rewrite", model: "claude-sonnet-5-5", promptVersion: 1 },
    });
    expect(events.at(-1)).toEqual({
      event: "done",
      data: {
        text: "Hi Lee, Thursday works.",
        object: null,
        usage: { inputTokens: 100, outputTokens: 20 },
        finishReason: "stop",
      },
    });
    // System prompt from the pack, user prompt rendered, structured output via output_config (not a tool),
    // effort hint passed, no sampling parameters.
    const call = captured.calls[0]!;
    expect(call.prompt[0]).toMatchObject({ role: "system" });
    expect(JSON.stringify(call.prompt)).toContain("Style: fix");
    expect(call.providerOptions).toEqual({
      anthropic: { structuredOutputMode: "outputFormat", effort: "low" },
    });
    expect(call.temperature).toBeUndefined();
    expect(call.maxOutputTokens).toBe(2000);
  });

  it("streams partial objects for summarize and validates the final object", async () => {
    const { model, captured } = mockModel(jsonPieces(summary));
    const usage: unknown[] = [];
    const events = await drain(
      streamFeature({
        feature: "summarize",
        vars: { transcript: "From: a@example.com", userName: "Sam", today: "2026-10-20" },
        choice: { model, modelId: "anthropic/claude-sonnet-5.5", kind: "akatsuki", tier: "default" },
        onUsage: (u) => {
          usage.push(u);
        },
      }),
    );
    expect(events.filter((e) => e.event === "partial").length).toBeGreaterThan(1);
    const done = events.at(-1)!;
    expect(done.event).toBe("done");
    if (done.event === "done") expect(done.data.object).toEqual(summary);
    expect(usage).toEqual([
      { modelId: "anthropic/claude-sonnet-5.5", usage: { inputTokens: 100, outputTokens: 20 } },
    ]);
    expect(captured.calls[0]!.responseFormat).toMatchObject({ type: "json" });
    expect(captured.calls[0]!.providerOptions).toEqual({
      openrouter: { reasoning: { effort: "low", exclude: true } },
    });
  });

  it("rejects invalid vars before calling the model", async () => {
    const { model, captured } = mockModel(["x"]);
    const events = await drain(
      streamFeature({
        feature: "summarize",
        vars: { transcript: "x", userName: "Sam", today: "yesterday", prompt: "raw prompt" },
        choice: { model, modelId: "m", kind: "openrouter", tier: "default" },
      }),
    );
    expect(events).toEqual([
      {
        event: "error",
        data: { code: "invalid_vars", message: "Invalid input for summarize.", retryable: false },
      },
    ]);
    expect(captured.calls).toHaveLength(0);
  });

  it("returns provider_invalid when the object does not match output.schema.json", async () => {
    const { model } = mockModel(jsonPieces({ tldr: 1 }));
    const events = await drain(
      streamFeature({
        feature: "extract_actions",
        vars: { transcript: "x", today: "2026-10-20" },
        choice: { model, modelId: "m", kind: "openrouter", tier: "default" },
      }),
    );
    expect(events.at(-1)).toMatchObject({ event: "error", data: { code: "provider_invalid" } });
  });

  it("escalates once to a heavier model after an invalid object when a hook allows it", async () => {
    const bad = mockModel(["not json"]);
    const good = mockModel(jsonPieces({ actions: [] }));
    const events = await drain(
      streamFeature({
        feature: "extract_actions",
        vars: { transcript: "x", today: "2026-10-20" },
        choice: {
          model: bad.model,
          modelId: "anthropic/claude-sonnet-5.5",
          kind: "openrouter",
          tier: "default",
        },
        escalation: {
          hook: escalateOnInvalidOutput,
          modelFor: (tier, model) => ({ model: good.model, modelId: model ?? "?", kind: "openrouter", tier }),
        },
      }),
    );
    expect(events.filter((e) => e.event === "meta").map((e) => e.event === "meta" && e.data.model)).toEqual([
      "anthropic/claude-sonnet-5.5",
      "anthropic/claude-opus-5.5",
    ]);
    expect(events.at(-1)).toMatchObject({ event: "done", data: { object: { actions: [] } } });
  });

  it("maps provider failures to §15 codes without provider text", async () => {
    const fail = new APICallError({
      message: "upstream said: secret details",
      url: "https://example.com",
      requestBodyValues: {},
      statusCode: 429,
      isRetryable: true,
    });
    const { model } = mockModel([], { fail });
    const events = await drain(
      streamFeature({
        feature: "rewrite",
        vars: { text: "x", style: "fix", voice: null },
        choice: { model, modelId: "m", kind: "openrouter", tier: "default" },
        maxRetries: 0,
      }),
    );
    const last = events.at(-1)!;
    expect(last).toMatchObject({ event: "error", data: { code: "provider_rate_limited", retryable: true } });
    expect(JSON.stringify(last)).not.toContain("secret");
  });

  it("refuses input above maxInputChars", async () => {
    const { model } = mockModel(["x"]);
    const events = await drain(
      streamFeature({
        feature: "nl_search",
        vars: {
          query: "q",
          today: "2026-10-20",
          timezone: "UTC",
          labels: "x".repeat(4000),
          splits: "y".repeat(2000),
        },
        choice: { model, modelId: "m", kind: "openrouter", tier: "fast" },
      }),
    );
    expect(events.at(-1)).toMatchObject({ event: "error" });
    expect(varsChars({ a: "é🎉", b: null })).toBe(2);
  });
});

const byoSettings = (kind: "openrouter" | "anthropic" | "akatsuki" = "openrouter"): AiSettings => ({
  profiles: [{ profileId: "p1", label: "P", kind, models: {}, keyOnDevices: ["d1"] }],
  routes: [{ feature: "default", profileId: "p1", enabled: true }],
  consent: CONSENTED,
});

describe("runFeature and the named feature functions", () => {
  const ctxWith = (pieces: string[], settings = byoSettings()) => {
    const mock = mockModel(pieces);
    const models: string[] = [];
    return {
      mock,
      models,
      ctx: {
        settings,
        keyFor: () => "test-key",
        languageModelFor: (_p: string, m: string) => {
          models.push(m);
          return mock.model;
        },
      },
    };
  };

  it("enforces consent, off routes, model choice and device keys before any call", () => {
    const noConsent = { ...byoSettings(), consent: { autoTriage: false, syncVoiceProfile: false } };
    const code = (fn: () => unknown) => {
      try {
        fn();
      } catch (e) {
        return (e as AiFeatureError).code;
      }
      return "none";
    };
    const vars = { text: "x", style: "fix", voice: null };
    expect(code(() => runFeature("rewrite", vars, { settings: noConsent, keyFor: () => "k" }))).toBe(
      "consent_required",
    );
    expect(
      code(() =>
        runFeature("rewrite", vars, { settings: { ...byoSettings(), routes: [] }, keyFor: () => "k" }),
      ),
    ).toBe("ai_off");
    const google: AiSettings = {
      ...byoSettings(),
      profiles: [{ profileId: "p1", label: "G", kind: "google", models: {}, keyOnDevices: [] }],
    };
    expect(code(() => runFeature("rewrite", vars, { settings: google, keyFor: () => "k" }))).toBe(
      "ai_model_required",
    );
    expect(
      code(() => runFeature("rewrite", vars, { settings: byoSettings(), keyFor: () => undefined })),
    ).toBe("key_missing");
  });

  it("summarize renders the transcript, uses the default-tier model and caps bullets at 5", async () => {
    const { ctx, models, mock } = ctxWith(jsonPieces(summary));
    const out = await summarize({ thread, userName: "Sam", now: new Date(1792483200000) }, ctx);
    expect(out.bullets).toHaveLength(5);
    expect(models).toEqual(["anthropic/claude-sonnet-5.5"]);
    const user = JSON.stringify(mock.captured.calls[0]!.prompt);
    expect(user).toContain("Mia Example &lt;mia@example.com&gt;");
    expect(user).toContain("Today: 2026-10-20");
  });

  it("triage uses the fast tier and maps keys back to thread ids", async () => {
    const answer = {
      items: [
        { key: "2", category: "promotions", needsReply: false },
        { key: "1", category: "important", needsReply: true },
        { key: "9", category: "other", needsReply: false },
      ],
    };
    const { ctx, models } = ctxWith(jsonPieces(answer));
    const item = (threadId: string) => ({
      threadId,
      from: { email: "a@example.com" },
      to: [],
      subject: "s",
      snippet: "n",
      isBulk: false,
      hasAttachments: false,
    });
    const out = await triage([item("t1"), item("t2")], ctx);
    expect(models).toEqual(["anthropic/claude-haiku-5.5"]);
    expect(out).toEqual([
      {
        threadId: "t2",
        category: "promotions",
        needsReply: false,
        model: "anthropic/claude-haiku-5.5",
        promptVersion: 1,
      },
      {
        threadId: "t1",
        category: "important",
        needsReply: true,
        model: "anthropic/claude-haiku-5.5",
        promptVersion: 1,
      },
    ]);
  });

  it("instantReply, extractActions, nlSearchToQuery, voiceProfile, writeInVoice and rewrite return their outputs", async () => {
    const replies = { replies: [1, 2, 3, 4].map((i) => ({ label: `L${i}`, body: `B${i}` })) };
    expect(await instantReply({ thread, userName: "Sam" }, ctxWith(jsonPieces(replies)).ctx)).toHaveLength(3);
    const actions = {
      actions: [{ kind: "reply", title: "Confirm head count", due: "2026-10-23", quote: "Can you confirm" }],
    };
    expect(await extractActions({ thread }, ctxWith(jsonPieces(actions)).ctx)).toEqual(actions.actions);
    const q = { query: "from:mia is:unread", explanation: "Unread mail from Mia." };
    const nl = ctxWith(jsonPieces(q));
    expect(
      await nlSearchToQuery(
        { query: "unread from mia", timezone: "UTC", labels: ["Work"], splits: [] },
        nl.ctx,
      ),
    ).toEqual(q);
    expect(nl.models).toEqual(["anthropic/claude-haiku-5.5"]);
    expect(await voiceProfile(["Hi,\nok.\n– S"], ctxWith(jsonPieces({ profile: "Short." })).ctx)).toBe(
      "Short.",
    );
    expect(
      await writeInVoice(
        { instruction: "Say yes", userName: "Sam", thread },
        ctxWith(["Yes, ", "works."]).ctx,
      ),
    ).toBe("Yes, works.");
    expect(await rewrite({ text: "teh", style: "fix" }, ctxWith(["the"]).ctx)).toBe("the");
  });

  it("managed profiles call apps/api with only feature, promptVersion and vars", async () => {
    const sse = [
      encodeAaspEvent({
        event: "meta",
        data: { feature: "rewrite", model: "anthropic/claude-sonnet-5.5", promptVersion: 1 },
      }),
      encodeAaspEvent({ event: "delta", data: { text: "the" } }),
      encodeAaspEvent({
        event: "done",
        data: { text: "the", object: null, usage: { inputTokens: 5, outputTokens: 1 }, finishReason: "stop" },
      }),
    ].join("");
    const { fetch, requests } = stubFetch(
      () => new Response(sse, { headers: { "content-type": "text/event-stream" } }),
    );
    const out = await rewrite(
      { text: "teh", style: "fix" },
      {
        settings: byoSettings("akatsuki"),
        keyFor: () => undefined,
        managed: { apiUrl: "https://api.akatsuki.vc/", getAccessToken: async () => "jwt", fetch },
      },
    );
    expect(out).toBe("the");
    expect(requests[0]!.url).toBe("https://api.akatsuki.vc/v1/ai/rewrite");
    expect(requests[0]!.body).toEqual({ promptVersion: 1, vars: { text: "teh", style: "fix", voice: null } });
    expect((requests[0]!.init.headers as Record<string, string>).authorization).toBe("Bearer jwt");
  });

  it("surfaces apps/api JSON errors as AASP errors", async () => {
    const { fetch } = stubFetch(
      () =>
        new Response(
          JSON.stringify({ error: { code: "ai_budget_exhausted", message: "Used up.", retryable: false } }),
          {
            status: 429,
          },
        ),
    );
    await expect(
      collectFeature(
        runFeature(
          "rewrite",
          { text: "x", style: "fix", voice: null },
          {
            settings: byoSettings("akatsuki"),
            keyFor: () => undefined,
            managed: { apiUrl: "https://api.akatsuki.vc", getAccessToken: async () => "jwt", fetch },
          },
        ),
      ),
    ).rejects.toMatchObject({ code: "ai_budget_exhausted" });
  });
});

describe("AASP encoding", () => {
  it("round-trips through the SSE reader, split at arbitrary byte boundaries", async () => {
    const events = [
      { event: "meta", data: { feature: "summarize", model: "m", promptVersion: 1 } },
      { event: "partial", data: { object: { tldr: "multi\nline <é>" } } },
      { event: "error", data: { code: "provider_unavailable", message: "x", retryable: true } },
    ] as const;
    async function* gen() {
      yield* events;
    }
    const bytes = new Uint8Array(await new Response(aaspReadableStream(gen())).arrayBuffer());
    const chunked = new ReadableStream<Uint8Array>({
      start(c) {
        for (let i = 0; i < bytes.length; i += 3) c.enqueue(bytes.slice(i, i + 3));
        c.close();
      },
    });
    expect(await drain(parseAaspStream(chunked))).toEqual(events);
    expect(encodeAaspEvent(events[0])).toBe(
      'event: meta\ndata: {"feature":"summarize","model":"m","promptVersion":1}\n\n',
    );
  });
});

describe("pricing", () => {
  it("computes micro-dollar costs from routing.json list prices", () => {
    // Sonnet 5.5: $2 / $10 per MTok.
    expect(tokenCostMicros("anthropic/claude-sonnet-5.5", { inputTokens: 4000, outputTokens: 300 })).toBe(
      11000,
    );
    // Haiku 5.5: $0.10 / $0.50 per MTok.
    expect(tokenCostMicros("anthropic/claude-haiku-5.5", { inputTokens: 400, outputTokens: 40 })).toBe(60);
    expect(estimateCostMicros("anthropic/claude-sonnet-5.5", 4000, 1200)).toBe(14000);
    // gpt-4o-transcribe: $0.006 per minute.
    expect(audioCostMicros("gpt-4o-transcribe", 90)).toBe(9000);
    expect(() => tokenCostMicros("unknown", { inputTokens: 1, outputTokens: 1 })).toThrow();
  });
});
