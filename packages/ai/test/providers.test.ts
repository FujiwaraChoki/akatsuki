// Real provider adapters against a stubbed fetch: checks what would go over the wire, without network or keys.
import { readFileSync } from "node:fs";
import { generateText } from "ai";
import { MockTranscriptionModelV4 } from "ai/test";
import { describe, expect, it } from "vitest";
import {
  callProviderOptions,
  createAkatsukiRegistry,
  createManagedModel,
  createProfileProvider,
  MissingKeyError,
  OPENROUTER_PRIVACY,
} from "../src/providers.ts";
import { dictate, transcribeAudio, transcriptionModelFor } from "../src/transcribe.ts";
import { CONSENTED, stubFetch } from "./helpers.ts";

const chatCompletion = () =>
  new Response(
    JSON.stringify({
      id: "gen-1",
      object: "chat.completion",
      created: 0,
      model: "m",
      choices: [{ index: 0, message: { role: "assistant", content: "pong" }, finish_reason: "stop" }],
      usage: { prompt_tokens: 3, completion_tokens: 1, total_tokens: 4 },
    }),
    { headers: { "content-type": "application/json" } },
  );

describe("provider registry", () => {
  it("registers one provider per BYO profile and addresses models as profileId:modelId", async () => {
    const { fetch, requests } = stubFetch(chatCompletion);
    const reg = createAkatsukiRegistry({
      profiles: [
        { profileId: "or", kind: "openrouter" },
        { profileId: "managed", kind: "akatsuki" },
        { profileId: "nokey", kind: "anthropic" },
        { profileId: "local", kind: "ollama", baseUrl: "http://localhost:11434" },
      ],
      keyFor: (id) => (id === "or" ? "test-openrouter-key" : undefined),
      options: { fetch },
    });
    expect(reg.missingKeys).toEqual(["nokey"]);
    expect(() => reg.languageModel("managed", "x")).toThrow(MissingKeyError);

    const { text } = await generateText({
      model: reg.languageModel("or", "anthropic/claude-sonnet-5.5"),
      prompt: "ping",
    });
    expect(text).toBe("pong");
    const req = requests[0]!;
    expect(req.url).toBe("https://openrouter.ai/api/v1/chat/completions");
    expect(req.body).toMatchObject({ model: "anthropic/claude-sonnet-5.5", provider: OPENROUTER_PRIVACY });
    expect((req.body as { provider: { zdr: boolean; data_collection: string } }).provider).toMatchObject({
      zdr: true,
      data_collection: "deny",
    });
  });

  it("matches the shared OpenRouter request fixture (crates/ai runs it too)", async () => {
    const fx = JSON.parse(
      readFileSync(new URL("../fixtures/openrouter/request.json", import.meta.url), "utf8"),
    ) as {
      provider: unknown;
      cases: { effort: "low" | "medium" | "high" | null; reasoning: unknown }[];
    };
    expect(OPENROUTER_PRIVACY).toEqual(fx.provider);
    for (const c of fx.cases) {
      const { fetch, requests } = stubFetch(chatCompletion);
      const reg = createAkatsukiRegistry({
        profiles: [{ profileId: "or", kind: "openrouter" }],
        keyFor: () => "test-openrouter-key",
        options: { fetch },
      });
      await generateText({
        model: reg.languageModel("or", "anthropic/claude-sonnet-5.5"),
        prompt: "ping",
        providerOptions: callProviderOptions("openrouter", c.effort) as never,
      });
      const body = requests[0]!.body as Record<string, unknown>;
      expect(body.provider, String(c.effort)).toEqual(fx.provider);
      expect(body.reasoning ?? null, String(c.effort)).toEqual(c.reasoning);
    }
  });

  it("the managed model always routes to ZDR endpoints with data collection denied", async () => {
    const { fetch, requests } = stubFetch(chatCompletion);
    await generateText({
      model: createManagedModel("anthropic/claude-haiku-5.5", "test-key", fetch),
      prompt: "ping",
    });
    expect(requests[0]!.body).toMatchObject({
      model: "anthropic/claude-haiku-5.5",
      provider: { zdr: true, data_collection: "deny", allow_fallbacks: true, require_parameters: true },
    });
  });

  it("sends the direct-browser-access header to Anthropic only in browsers", async () => {
    const headersOf = async (browser: boolean) => {
      const { fetch, requests } = stubFetch(
        () =>
          new Response(
            JSON.stringify({
              id: "msg_1",
              type: "message",
              role: "assistant",
              model: "claude-haiku-5-5",
              content: [{ type: "text", text: "pong" }],
              stop_reason: "end_turn",
              usage: { input_tokens: 1, output_tokens: 1 },
            }),
            { headers: { "content-type": "application/json" } },
          ),
      );
      const p = createProfileProvider({ profileId: "a", kind: "anthropic" }, "test-key", { browser, fetch });
      await generateText({ model: p.languageModel!("claude-haiku-5-5"), prompt: "ping" });
      return new Headers(requests[0]!.init.headers as ConstructorParameters<typeof Headers>[0]);
    };
    expect((await headersOf(true)).get("anthropic-dangerous-direct-browser-access")).toBe("true");
    expect((await headersOf(false)).get("anthropic-dangerous-direct-browser-access")).toBeNull();
  });

  it("Ollama talks to the local server and needs no key", async () => {
    const { fetch, requests } = stubFetch(
      () =>
        new Response(
          JSON.stringify({
            model: "llama4:8b",
            created_at: "2026-10-20T00:00:00Z",
            message: { role: "assistant", content: "pong" },
            done: true,
            done_reason: "stop",
            prompt_eval_count: 1,
            eval_count: 1,
          }),
          { headers: { "content-type": "application/json" } },
        ),
    );
    const p = createProfileProvider(
      { profileId: "l", kind: "ollama", baseUrl: "http://localhost:11434/" },
      undefined,
      {
        fetch,
      },
    );
    await generateText({ model: p.languageModel!("llama4:8b"), prompt: "ping" });
    expect(requests[0]!.url.startsWith("http://localhost:11434/api/")).toBe(true);
  });

  it("refuses to build a provider for the managed kind or without a required key", () => {
    expect(() => createProfileProvider({ profileId: "m", kind: "akatsuki" }, "k")).toThrow(/apps\/api/);
    expect(() => createProfileProvider({ profileId: "o", kind: "openai" }, undefined)).toThrow(
      MissingKeyError,
    );
  });
});

describe("transcription", () => {
  it("transcribeAudio returns text, duration and language from the model", async () => {
    let seen: unknown;
    const model = new MockTranscriptionModelV4({
      doGenerate: async (options) => {
        seen = options.providerOptions;
        return {
          text: "hello world",
          segments: [],
          language: "en",
          durationInSeconds: 2.5,
          warnings: [],
          response: { timestamp: new Date(0), modelId: "gpt-4o-transcribe" },
        };
      },
    });
    const out = await transcribeAudio(model, new Uint8Array([1, 2, 3]), {
      language: "en-GB",
      prompt: "Akatsuki",
    });
    expect(out).toEqual({ text: "hello world", durationSec: 2.5, language: "en" });
    expect(seen).toEqual({ openai: { language: "en", prompt: "Akatsuki" } });
  });

  it("uses OpenAI or a local OpenAI-compatible Whisper server", async () => {
    const { fetch, requests } = stubFetch(
      () =>
        new Response(JSON.stringify({ text: "local text" }), {
          headers: { "content-type": "application/json" },
        }),
    );
    const local = transcriptionModelFor(
      { profileId: "w", kind: "openai_compatible", baseUrl: "http://127.0.0.1:8080/v1" },
      "whisper-large-v3",
      undefined,
      fetch,
    );
    const out = await transcribeAudio(local, new Uint8Array([0, 1]));
    expect(out.text).toBe("local text");
    expect(requests[0]!.url).toBe("http://127.0.0.1:8080/v1/audio/transcriptions");

    const remote = transcriptionModelFor(
      { profileId: "o", kind: "openai" },
      "gpt-4o-transcribe",
      "test-key",
      fetch,
    );
    await transcribeAudio(remote, new Uint8Array([0, 1]));
    expect(requests[1]!.url).toBe("https://api.openai.com/v1/audio/transcriptions");
    expect(() => transcriptionModelFor({ profileId: "a", kind: "anthropic" }, "x", "k")).toThrow(
      /cannot transcribe/,
    );
  });

  it("dictate follows the dictation route", async () => {
    const { fetch, requests } = stubFetch(
      () =>
        new Response(JSON.stringify({ text: "dictated" }), {
          headers: { "content-type": "application/json" },
        }),
    );
    const out = await dictate(new Uint8Array([1]), {
      settings: {
        profiles: [
          {
            profileId: "w",
            label: "Whisper",
            kind: "openai_compatible",
            baseUrl: "http://localhost:9000/v1",
            models: { transcription: "base.en" },
            keyOnDevices: [],
          },
        ],
        routes: [{ feature: "dictation", profileId: "w", enabled: true }],
        consent: CONSENTED,
      },
      keyFor: () => undefined,
      fetch,
    });
    expect(out.text).toBe("dictated");
    expect(requests[0]!.url).toBe("http://localhost:9000/v1/audio/transcriptions");
  });
});
