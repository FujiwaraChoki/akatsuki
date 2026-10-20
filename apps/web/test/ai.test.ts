// AI on the web client without network: the BYO key store (non-extractable WebCrypto key, D-009), the
// managed vs BYO path of `runAi`, feature status and egress copy, the output cache, Test connection for Ollama
// with a stubbed fetch, and the NL-search chips.
import { describe, expect, it, vi } from "vitest";
import { aiCacheGet, aiCachePut, setAiCacheStore } from "../src/ai/cache.ts";
import { queryChips } from "../src/ai/extension.tsx";
import { KeyStore, memoryVault } from "../src/ai/keyStore.ts";
import { loadAi } from "../src/ai/module.ts";
import { type AiEnv, aiStatus, NO_AI_SETTINGS, runAi } from "../src/ai/useAi.ts";
import type { ApiService } from "../src/backend/types.ts";
import { memoryStore } from "../src/lib/kv.ts";
import { testConnection } from "../src/settings/aiTest.ts";

describe("key store", () => {
  it("remembers keys as AES-GCM ciphertext under a key that cannot be exported", async () => {
    const vault = memoryVault();
    const ks = new KeyStore(vault);
    await ks.set("p1", "sk-or-secret-123", true);
    expect(ks.get("p1")).toBe("sk-or-secret-123");
    expect(ks.isRemembered("p1")).toBe(true);
    const sealed = vault.raw.get("k:p1") as { ciphertext: ArrayBuffer; iv: Uint8Array };
    expect(new TextDecoder().decode(sealed.ciphertext)).not.toContain("secret");
    const master = vault.raw.get("__master__") as CryptoKey;
    expect(master.extractable).toBe(false);
    await expect(crypto.subtle.exportKey("raw", master)).rejects.toThrow();
    // A new session (same browser storage) decrypts it again.
    const next = new KeyStore(vault);
    expect(next.get("p1")).toBeUndefined();
    await next.load();
    expect(next.get("p1")).toBe("sk-or-secret-123");
  });

  it("keeps session-only keys in memory and forgets them on remove", async () => {
    const vault = memoryVault();
    const ks = new KeyStore(vault);
    await ks.set("p2", "local-key", false);
    expect(ks.get("p2")).toBe("local-key");
    expect(await vault.keys()).toEqual([]);
    await ks.set("p3", "x", true);
    await ks.remove("p3");
    expect(ks.get("p3")).toBeUndefined();
    expect(await vault.keys()).toEqual([]);
  });

  it("drops entries that no longer decrypt", async () => {
    const vault = memoryVault();
    await new KeyStore(vault).set("p1", "k", true);
    vault.raw.set(
      "__master__",
      await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]),
    );
    const ks = new KeyStore(vault);
    await ks.load();
    expect(ks.get("p1")).toBeUndefined();
    expect(await vault.keys()).toEqual([]);
  });
});

const managedProfile = {
  profileId: "11111111-1111-4111-8111-111111111111",
  label: "Akatsuki AI",
  kind: "akatsuki" as const,
  models: {},
  keyOnDevices: [],
};
const ollamaProfile = {
  profileId: "22222222-2222-4222-8222-222222222222",
  label: "Ollama",
  kind: "ollama" as const,
  models: { default: "llama3.2" },
  keyOnDevices: [],
};
const orProfile = {
  profileId: "33333333-3333-4333-8333-333333333333",
  label: "OpenRouter",
  kind: "openrouter" as const,
  models: {},
  keyOnDevices: [],
};

function api(over: Partial<ApiService> = {}): ApiService {
  return {
    configured: true,
    streamFeature: vi.fn(async function* () {
      yield { event: "meta" as const, data: { feature: "summarize", model: "m", promptVersion: 1 } };
      yield {
        event: "done" as const,
        data: {
          text: null,
          object: { tldr: "x" },
          usage: { inputTokens: 1, outputTokens: 1 },
          finishReason: "stop",
        },
      };
    }),
    transcribe: vi.fn(),
    quota: vi.fn(async () => ({
      period: "2026-10",
      usedMicros: 1_000_000,
      limitMicros: 8_000_000,
      resetsAt: 0,
    })),
    checkout: vi.fn(),
    portal: vi.fn(),
    cancel: vi.fn(),
    ...over,
  } as ApiService;
}

const settingsWith = (
  profile: typeof managedProfile | typeof ollamaProfile | typeof orProfile,
  consent = true,
) => ({
  profiles: [profile],
  routes: [{ feature: "default" as const, profileId: profile.profileId, enabled: true }],
  consent: { autoTriage: false, syncVoiceProfile: false, ...(consent && { grantedAt: 1, version: 1 }) },
});

describe("runAi", () => {
  it("refuses before consent (§12.7 rule 1)", async () => {
    const env: AiEnv = {
      settings: NO_AI_SETTINGS,
      api: api(),
      keys: { get: () => undefined },
      managedAi: true,
    };
    await expect(runAi("summarize", {}, env).next()).rejects.toMatchObject({ code: "consent_required" });
  });

  it("sends managed requests to apps/api with the pack's prompt version", async () => {
    const a = api();
    const env: AiEnv = {
      settings: settingsWith(managedProfile),
      api: a,
      keys: { get: () => undefined },
      managedAi: true,
    };
    const events = [];
    for await (const e of runAi("summarize", { transcript: "t", userName: "You", today: "2026-10-20" }, env))
      events.push(e);
    expect(events.map((e) => e.event)).toEqual(["meta", "done"]);
    const ai = await loadAi();
    expect(a.streamFeature).toHaveBeenCalledWith(
      "summarize",
      { promptVersion: ai.promptVersion("summarize"), vars: expect.objectContaining({ transcript: "t" }) },
      undefined,
    );
  });

  it("refuses the managed provider without Pro", async () => {
    const env: AiEnv = {
      settings: settingsWith(managedProfile),
      api: api(),
      keys: { get: () => undefined },
      managedAi: false,
    };
    await expect(runAi("summarize", {}, env).next()).rejects.toMatchObject({ code: "not_entitled" });
  });

  it("runs BYO features on the device through packages/ai (never apps/api)", async () => {
    const ai = await loadAi();
    const a = api();
    const runFeature = vi.fn(async function* () {
      yield {
        event: "done" as const,
        data: { text: "ok", object: null, usage: { inputTokens: 0, outputTokens: 0 }, finishReason: "stop" },
      };
    });
    const env: AiEnv = {
      settings: settingsWith(orProfile),
      api: a,
      keys: { get: (id) => (id === orProfile.profileId ? "sk" : undefined) },
      managedAi: true,
    };
    const fake = { ...ai, runFeature } as unknown as typeof ai;
    const out = [];
    for await (const e of runAi("rewrite", { text: "x", style: "fix", voice: null }, env, undefined, fake))
      out.push(e);
    expect(out).toHaveLength(1);
    expect(a.streamFeature).not.toHaveBeenCalled();
    const ctx = (runFeature.mock.calls as unknown as unknown[][])[0]![2] as {
      keyFor: (id: string) => string | undefined;
      providerOptions: unknown;
    };
    expect(ctx.keyFor(orProfile.profileId)).toBe("sk");
    expect(ctx.providerOptions).toEqual({ browser: true });
  });
});

describe("status and egress", () => {
  it("reports why a feature cannot run", async () => {
    const ai = await loadAi();
    const base = { keys: { get: () => undefined }, managedAi: true, apiConfigured: true };
    expect(aiStatus(ai, "summarize", { ...base, settings: settingsWith(orProfile, false) })).toBe("consent");
    expect(aiStatus(ai, "summarize", { ...base, settings: { ...settingsWith(orProfile), routes: [] } })).toBe(
      "off",
    );
    expect(aiStatus(ai, "summarize", { ...base, settings: settingsWith(orProfile) })).toBe("no_key");
    expect(aiStatus(ai, "summarize", { ...base, settings: settingsWith(ollamaProfile) })).toBe("ready");
    expect(
      aiStatus(ai, "summarize", { ...base, managedAi: false, settings: settingsWith(managedProfile) }),
    ).toBe("not_entitled");
    expect(
      aiStatus(ai, "summarize", {
        ...base,
        settings: settingsWith({ ...ollamaProfile, models: {} } as unknown as typeof ollamaProfile),
      }),
    ).toBe("no_model");
  });

  it("labels where data goes", async () => {
    const ai = await loadAi();
    expect(ai.egressText(ollamaProfile, "summarize")).toBe("Stays on this device");
    expect(ai.egressText(managedProfile, "summarize")).toBe("Sent to Akatsuki for processing, not stored");
    expect(ai.egressText(managedProfile, "dictation")).toMatch(/OpenAI/);
    expect(ai.egressText(orProfile, "rewrite")).toBe(
      "Leaves this device: the text you rewrite, sent to openrouter.ai",
    );
  });
});

describe("AI output cache", () => {
  it("is keyed by feature, prompt version, model and input", async () => {
    setAiCacheStore(memoryStore());
    await aiCachePut("summarize", 1, "m", { a: 1 }, { text: null, object: { tldr: "x" } });
    expect((await aiCacheGet("summarize", 1, "m", { a: 1 }))?.object).toEqual({ tldr: "x" });
    expect(await aiCacheGet("summarize", 2, "m", { a: 1 })).toBeUndefined();
    expect(await aiCacheGet("summarize", 1, "other", { a: 1 })).toBeUndefined();
    expect(await aiCacheGet("summarize", 1, "m", { a: 2 })).toBeUndefined();
    setAiCacheStore(null);
  });
});

describe("test connection", () => {
  it("asks Ollama for its tags from this device", async () => {
    const fetchFn = vi.fn(async () => new Response(JSON.stringify({ models: [{}, {}] }), { status: 200 }));
    const r = await testConnection(ollamaProfile, undefined, api(), fetchFn as never);
    expect(fetchFn).toHaveBeenCalledWith("http://127.0.0.1:11434/api/tags");
    expect(r).toEqual({ ok: true, detail: "Ollama is running with 2 models installed." });
  });
  it("reports an unreachable Ollama", async () => {
    const r = await testConnection(
      { ...ollamaProfile, baseUrl: "http://192.168.1.5:11434" },
      undefined,
      api(),
      (async () => {
        throw new TypeError("network");
      }) as never,
    );
    expect(r.ok).toBe(false);
    expect(r.detail).toMatch(/192\.168\.1\.5/);
  });
  it("checks the managed provider through the quota endpoint", async () => {
    expect(await testConnection(managedProfile, undefined, api())).toEqual({
      ok: true,
      detail: "Akatsuki AI is reachable. $1.00 of $8.00 used this month.",
    });
  });
  it("needs a key for cloud providers", async () => {
    const r = await testConnection(orProfile, undefined, api(), vi.fn() as never);
    expect(r).toEqual({ ok: false, detail: "Key not set on this device." });
  });
});

describe("dictation", () => {
  it("managed dictation goes to apps/api transcribe; BYO without a route is off", async () => {
    const { transcribe } = await import("../src/ai/dictation.ts");
    const a = api({
      transcribe: vi.fn(async () => ({ text: "hello there", durationSec: 1, language: "en" })),
    });
    const audio = new Blob([new Uint8Array([1, 2, 3])], { type: "audio/webm" });
    const managed = {
      settings: settingsWith(managedProfile),
      managedAi: true,
      env: { api: a },
      keys: { get: () => undefined },
    };
    expect(await transcribe(managed as never, audio)).toBe("hello there");
    expect(a.transcribe).toHaveBeenCalledWith({ audio }, undefined);
    const off = { ...managed, settings: { ...settingsWith(managedProfile), routes: [] } };
    await expect(transcribe(off as never, audio)).rejects.toMatchObject({ code: "ai_off" });
    const noConsent = { ...managed, settings: settingsWith(managedProfile, false) };
    await expect(transcribe(noConsent as never, audio)).rejects.toMatchObject({ code: "consent_required" });
  });
});

describe("NL search chips", () => {
  it("keeps quoted phrases and field values together", () => {
    expect(queryChips('from:mara is:unread subject:"budget review" -in:trash pdf')).toEqual([
      "from:mara",
      "is:unread",
      'subject:"budget review"',
      "-in:trash",
      "pdf",
    ]);
  });
});
