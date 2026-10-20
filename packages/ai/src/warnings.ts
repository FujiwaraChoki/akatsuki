// AI SDK warning logging (D-396). `@openrouter/ai-sdk-provider@3.1.0`, the newest stable release, implements the
// v2 LanguageModel specification, so AI SDK 7 runs it in compatibility mode and warns on every call. That one
// warning is known and harmless; every other warning is still reported, as one content-free line (provider,
// model, warning type and feature, never prompts or outputs).
import type { LogWarningsFunction } from "ai";

type SdkWarning = Parameters<LogWarningsFunction>[0]["warnings"][number];

/** The known compatibility notice of providers that still implement the v2 specification. */
export function isKnownCompatWarning(w: SdkWarning): boolean {
  return w.type === "compatibility" && "feature" in w && w.feature === "specificationVersion";
}

/** Installs the filtering logger as the AI SDK's global warning logger. */
export function installAiWarningFilter(log: (line: string) => void = (l) => console.warn(l)): void {
  globalThis.AI_SDK_LOG_WARNINGS = ({ warnings, provider, model }) => {
    for (const w of warnings) {
      if (isKnownCompatWarning(w)) continue;
      const feature = "feature" in w ? ` ${String(w.feature)}` : "";
      log(`AI SDK warning (${provider ?? "?"} / ${model ?? "?"}): ${w.type}${feature}`);
    }
  };
}
