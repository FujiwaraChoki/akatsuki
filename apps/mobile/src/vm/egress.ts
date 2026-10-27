// Short forms of the egress sentences (design spec §3.8) for the mono provenance lines. The full sentence
// from packages/ai `egressText` stays the accessible label and is shown in full where the user decides (the
// dictation and write sheets, settings).
export function egressShort(sentence: string): string {
  if (sentence === "Stays on this device") return "On this device";
  if (sentence.startsWith("Stays on your local network")) return "On your network";
  if (sentence.startsWith("Sent to Akatsuki")) return "Akatsuki · Not stored";
  if (sentence.startsWith("Sent to OpenAI through Akatsuki"))
    return "OpenAI via Akatsuki · Kept up to 30 days";
  const host = /, sent to ([^\s,]+)$/.exec(sentence)?.[1];
  if (sentence.startsWith("Leaves this device"))
    return host ? `Leaves this device · ${host}` : "Leaves this device";
  return sentence;
}

/** The provenance line of an AI result: `Summary · On this device`. */
export function provenance(what: string, egress: string | null): string {
  return egress ? `${what} · ${egressShort(egress)}` : what;
}
