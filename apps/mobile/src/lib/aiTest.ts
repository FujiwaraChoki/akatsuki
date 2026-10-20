// "Test connection" (§12.2): from the device, never through Akatsuki. Ollama: `GET /api/tags`; other BYO
// providers: a one-token `generateText("ping")`; the managed provider: the quota endpoint.
import { type AiProfile, createProfileProvider } from "@akatsuki/ai";
import { generateText } from "ai";
import { fetch as expoFetch } from "expo/fetch";
import { connectionTest } from "../vm/aiSettings";
import { api } from "./clients";
import { errorText } from "./errors";

const TIMEOUT_MS = 10_000;

export async function testConnection(
  profile: AiProfile,
  key: string | undefined,
): Promise<{ ok: true; detail: string } | { ok: false; error: string }> {
  const test = connectionTest(profile);
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    if (test.kind === "managed") {
      if (!api) return { ok: false, error: "The Akatsuki API is not configured in this build." };
      const q = await api.quota();
      const used = Math.round((q.usedMicros / Math.max(1, q.limitMicros)) * 100);
      return { ok: true, detail: `Connected. ${used}% of this month's allowance used.` };
    }
    if (test.kind === "http_get") {
      const res = await expoFetch(test.url, { signal: controller.signal });
      if (!res.ok) return { ok: false, error: `The server answered ${res.status}.` };
      const body = (await res.json().catch(() => null)) as { models?: unknown[] } | null;
      const n = Array.isArray(body?.models) ? body.models.length : null;
      return {
        ok: true,
        detail: n === null ? "Connected." : `Connected. ${n} ${n === 1 ? "model" : "models"} available.`,
      };
    }
    if (!test.model) return { ok: false, error: "Choose a model first." };
    const provider = createProfileProvider(profile, key, {
      fetch: expoFetch as unknown as typeof fetch,
      browser: false,
    });
    await generateText({
      model: provider.languageModel(test.model),
      prompt: "ping",
      maxOutputTokens: 1,
      abortSignal: controller.signal,
    });
    return { ok: true, detail: `Connected to ${test.model}.` };
  } catch (err) {
    if (controller.signal.aborted) return { ok: false, error: "No answer within 10 seconds." };
    const message = err instanceof Error && err.message.length < 200 ? err.message : errorText(err);
    return { ok: false, error: message };
  } finally {
    clearTimeout(timer);
  }
}
