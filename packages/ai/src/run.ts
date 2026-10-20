// The one place a feature prompt meets a model: validates vars, renders the prompt pack, calls streamText
// (with Output.object for object features) and yields AASP events. Used by the BYO path on clients and by
// apps/api for the managed path, so both produce identical event streams (§12.6).
import { type FeatureId, getFeature, renderPrompt } from "@akatsuki/prompts";
import { APICallError, jsonSchema, type LanguageModel, Output, streamText } from "ai";
import type { AaspError, AaspEvent, AaspUsage } from "./aasp.ts";
import { callProviderOptions } from "./providers.ts";
import type { EscalationHook, ProviderKind, Tier } from "./routing.ts";
import { validateOutput, validateVars } from "./validate.ts";

export interface ModelChoice {
  model: LanguageModel;
  /** The id reported in `meta` and used for pricing (e.g. "anthropic/claude-sonnet-5.5"). */
  modelId: string;
  kind: ProviderKind;
  tier: Tier;
}

export interface StreamFeatureInput {
  feature: FeatureId;
  vars: unknown;
  choice: ModelChoice;
  abortSignal?: AbortSignal;
  /** AI SDK retries for the initial provider call (default 2). */
  maxRetries?: number;
  /** Optional escalation (D-120): returns a heavier model for one retry after an invalid object. */
  escalation?: { hook: EscalationHook; modelFor: (tier: Tier, model?: string) => ModelChoice | null };
  /** Called once with the final usage (apps/api records cost here; nothing else is kept). */
  onUsage?: (u: { modelId: string; usage: AaspUsage }) => void | Promise<void>;
}

const err = (code: string, message: string, retryable: boolean, retryAfterMs?: number): AaspError =>
  retryAfterMs === undefined ? { code, message, retryable } : { code, message, retryable, retryAfterMs };

/** Maps provider failures to §15 codes. The message never contains provider response text. */
export function providerError(e: unknown): AaspError {
  if (APICallError.isInstance(e)) {
    const status = e.statusCode ?? 0;
    if (status === 429)
      return err("provider_rate_limited", "The AI provider is busy. Try again shortly.", true);
    if (status === 401 || status === 403)
      return err("provider_invalid", "The AI provider rejected the key or the request.", false);
    if (status >= 400 && status < 500)
      return err("provider_invalid", "The AI provider rejected the request.", false);
  }
  return err("provider_unavailable", "The AI provider is not available right now.", true);
}

/** Total characters (Unicode scalar values) of all string vars; must not exceed meta.maxInputChars. */
export function varsChars(vars: Record<string, unknown>): number {
  let n = 0;
  for (const v of Object.values(vars)) if (typeof v === "string") n += Array.from(v).length;
  return n;
}

const toUsage = (u: { inputTokens?: number | undefined; outputTokens?: number | undefined } | undefined) => ({
  inputTokens: u?.inputTokens ?? 0,
  outputTokens: u?.outputTokens ?? 0,
});

/**
 * Runs one feature against an already-resolved model. Yields `meta`, then `delta` (text) or `partial`
 * (streamed object features), then exactly one `done` or `error`. Invalid vars yield a single `error`
 * (`invalid_vars`) before `meta`.
 */
export async function* streamFeature(input: StreamFeatureInput): AsyncGenerator<AaspEvent> {
  const { feature } = input;
  const f = getFeature(feature);
  const vars = validateVars(feature, input.vars);
  if (!vars.ok) {
    yield { event: "error", data: err("invalid_vars", `Invalid input for ${feature}.`, false) };
    return;
  }
  const inputChars = varsChars(vars.value);
  if (inputChars > f.meta.maxInputChars) {
    yield {
      event: "error",
      data: err("payload_too_large", "The input is too long for this feature.", false),
    };
    return;
  }

  const prompt = renderPrompt(feature, vars.value);
  let choice = input.choice;
  const escalation = input.escalation;
  yield { event: "meta", data: { feature, model: choice.modelId, promptVersion: f.meta.version } };

  const total: AaspUsage = { inputTokens: 0, outputTokens: 0 };
  for (let attempt = 0; attempt < 2; attempt++) {
    let streamError: unknown;
    const result = streamText({
      model: choice.model,
      instructions: prompt.system,
      prompt: prompt.user,
      maxOutputTokens: f.meta.maxOutputTokens,
      abortSignal: input.abortSignal,
      maxRetries: input.maxRetries ?? 2,
      providerOptions: callProviderOptions(choice.kind, f.meta.effort) as never,
      ...(f.meta.mode === "object" && f.outputSchema
        ? { output: Output.object({ schema: jsonSchema(f.outputSchema as never) }) }
        : {}),
      onError: ({ error }) => {
        streamError = error;
      },
    });
    try {
      if (f.meta.mode === "text") {
        for await (const text of result.textStream) if (text) yield { event: "delta", data: { text } };
      } else if (f.meta.stream) {
        for await (const partial of result.partialOutputStream)
          yield { event: "partial", data: { object: partial } };
      } else {
        await result.consumeStream();
      }
      if (streamError) throw streamError;
      const usage = toUsage(await result.totalUsage);
      total.inputTokens += usage.inputTokens;
      total.outputTokens += usage.outputTokens;
      const finishReason = String(await result.finishReason);

      if (f.meta.mode === "text") {
        await input.onUsage?.({ modelId: choice.modelId, usage: total });
        yield { event: "done", data: { text: await result.text, object: null, usage: total, finishReason } };
        return;
      }
      let object: unknown;
      try {
        object = await result.output;
      } catch {
        object = undefined;
      }
      const valid = object === undefined ? null : validateOutput(feature, object);
      if (valid?.ok) {
        await input.onUsage?.({ modelId: choice.modelId, usage: total });
        yield { event: "done", data: { text: null, object: valid.value, usage: total, finishReason } };
        return;
      }
      const next =
        attempt === 0 && escalation
          ? escalation.hook({
              feature,
              tier: choice.tier,
              kind: choice.kind,
              attempt: attempt + 1,
              reason: "invalid_output",
              inputChars,
            })
          : null;
      const nextChoice = next && escalation ? escalation.modelFor(next.tier, next.model) : null;
      if (!nextChoice) {
        await input.onUsage?.({ modelId: choice.modelId, usage: total });
        yield {
          event: "error",
          data: err("provider_invalid", "The AI returned an answer in the wrong format.", true),
        };
        return;
      }
      await input.onUsage?.({ modelId: choice.modelId, usage: total });
      total.inputTokens = 0;
      total.outputTokens = 0;
      choice = nextChoice;
      yield { event: "meta", data: { feature, model: choice.modelId, promptVersion: f.meta.version } };
    } catch (e) {
      if (input.abortSignal?.aborted) return;
      // The provider may still have reported usage for the failed attempt (an error part mid-stream, then a
      // finish): charge it rather than nothing (D-406). A stream that died has none; the caller estimates.
      const attempt = await settledUsage(result.totalUsage);
      if (attempt) {
        total.inputTokens += attempt.inputTokens;
        total.outputTokens += attempt.outputTokens;
      }
      if (total.inputTokens + total.outputTokens > 0)
        await input.onUsage?.({ modelId: choice.modelId, usage: total });
      yield { event: "error", data: providerError(e) };
      return;
    }
  }
}

/** The usage of a finished or failed attempt, or null when the SDK has none (bounded wait). */
async function settledUsage(usage: PromiseLike<unknown>): Promise<AaspUsage | null> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const u = await Promise.race([
      Promise.resolve(usage),
      new Promise<null>((r) => {
        timer = setTimeout(() => r(null), 1000);
      }),
    ]);
    if (u === null) return null;
    const out = toUsage(u as never);
    return out.inputTokens + out.outputTokens > 0 ? out : null;
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

export interface FeatureResult {
  model: string | null;
  text: string | null;
  object: unknown;
  usage: AaspUsage;
}

export class AiFeatureError extends Error {
  override name = "AiFeatureError";
  readonly code: string;
  readonly retryable: boolean;
  constructor(code: string, message: string, retryable = false) {
    super(message);
    this.code = code;
    this.retryable = retryable;
  }
}

/** Drains an AASP stream into its final result; an `error` event throws AiFeatureError. */
export async function collectFeature(events: AsyncIterable<AaspEvent>): Promise<FeatureResult> {
  let model: string | null = null;
  for await (const e of events) {
    if (e.event === "meta") model = e.data.model;
    else if (e.event === "error") throw new AiFeatureError(e.data.code, e.data.message, e.data.retryable);
    else if (e.event === "done")
      return { model, text: e.data.text, object: e.data.object, usage: e.data.usage };
  }
  throw new AiFeatureError("provider_unavailable", "The AI stream ended early.", true);
}
