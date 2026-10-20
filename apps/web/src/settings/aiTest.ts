// "Test connection" for an AI profile (§12.2): runs from this device. Ollama: `GET /api/tags`; the managed
// provider: apps/api `GET /v1/ai/quota`; every other provider: a one-token `generateText("ping")` through the
// same AI SDK registry the features use. Errors are reported with fixed copy, never provider response text.

import { loadAi } from "../ai/module.ts";
import type { AiSettingsView, ApiService } from "../backend/types.ts";

type Profile = AiSettingsView["profiles"][number];

export type TestResult = { ok: true; detail: string } | { ok: false; detail: string };

const money = (micros: number) => `$${(micros / 1_000_000).toFixed(2)}`;

export async function testConnection(
  profile: Profile,
  key: string | undefined,
  api: ApiService,
  fetchFn: typeof fetch = (...a) => globalThis.fetch(...a),
): Promise<TestResult> {
  if (profile.kind === "akatsuki") {
    if (!api.configured) return { ok: false, detail: "Akatsuki AI is not configured in this build." };
    try {
      const q = await api.quota();
      return {
        ok: true,
        detail: `Akatsuki AI is reachable. ${money(q.usedMicros)} of ${money(q.limitMicros)} used this month.`,
      };
    } catch {
      return { ok: false, detail: "Akatsuki AI did not answer. Check that your Pro plan is active." };
    }
  }
  if (profile.kind === "ollama") {
    const base = (profile.baseUrl ?? "http://127.0.0.1:11434").replace(/\/+$/, "").replace(/\/api$/, "");
    try {
      const res = await fetchFn(`${base}/api/tags`);
      if (!res.ok) return { ok: false, detail: `Ollama answered with HTTP ${res.status}.` };
      const body = (await res.json().catch(() => ({}))) as { models?: unknown[] };
      const n = Array.isArray(body.models) ? body.models.length : 0;
      return { ok: true, detail: `Ollama is running with ${n} model${n === 1 ? "" : "s"} installed.` };
    } catch {
      return {
        ok: false,
        detail: `Could not reach Ollama at ${base}. Start it with OLLAMA_ORIGINS set to this site's address.`,
      };
    }
  }
  const mod = await loadAi();
  const model = profile.models.default ?? mod.tableModel("default", profile.kind) ?? profile.models.fast;
  if (!model) return { ok: false, detail: "Choose a default model first." };
  try {
    const reg = mod.createAkatsukiRegistry({
      profiles: [profile],
      keyFor: () => key,
      options: { browser: true, fetch: fetchFn },
    });
    const lm = reg.languageModel(profile.profileId, model);
    const { generateText } = await import("ai");
    await generateText({ model: lm, prompt: "ping", maxOutputTokens: 1, maxRetries: 0 });
    return { ok: true, detail: `Connected. ${model} answered.` };
  } catch (e) {
    if (e instanceof mod.MissingKeyError) return { ok: false, detail: "Key not set on this device." };
    return { ok: false, detail: mod.providerError(e).message };
  }
}
