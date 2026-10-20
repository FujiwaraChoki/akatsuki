// Cost accounting for the managed path (§13.4): integer micro-dollars from the list prices in routing.json.
import { ROUTING, type RoutingTable } from "./routing.ts";

/** Rough token estimate for budgeting before a call: 1 token per 4 characters, rounded up. */
export const estimateTokens = (chars: number) => Math.ceil(chars / 4);

export function priceOf(model: string, table: RoutingTable = ROUTING) {
  const price = table.pricing[model];
  if (!price) throw new Error(`no price for model "${model}" in routing.json`);
  return price;
}

/** Cost of a finished text call. */
export function tokenCostMicros(
  model: string,
  usage: { inputTokens: number; outputTokens: number },
  table: RoutingTable = ROUTING,
): number {
  const p = priceOf(model, table);
  const micros =
    (usage.inputTokens * (p.inputPerMTokMicros ?? 0) + usage.outputTokens * (p.outputPerMTokMicros ?? 0)) /
    1e6;
  return Math.ceil(micros);
}

/** Upper-bound estimate for `ee.service.aiPrecheck`: the whole input plus the full output allowance. */
export function estimateCostMicros(
  model: string,
  inputChars: number,
  maxOutputTokens: number,
  table: RoutingTable = ROUTING,
): number {
  return tokenCostMicros(
    model,
    { inputTokens: estimateTokens(inputChars), outputTokens: maxOutputTokens },
    table,
  );
}

/** Transcription is billed per audio minute (rounded up to the second). */
export function audioCostMicros(model: string, seconds: number, table: RoutingTable = ROUTING): number {
  const p = priceOf(model, table);
  return Math.ceil(((p.perAudioMinuteMicros ?? 0) * Math.max(0, seconds)) / 60);
}
