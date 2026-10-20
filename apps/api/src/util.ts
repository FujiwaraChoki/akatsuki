// Small helpers: an in-memory rate limiter, month boundaries, and an audio-length estimate for billing.

/** Fixed-size sliding window per key (one apps/api container, so in-memory is enough for checkout). */
export class SlidingWindowLimiter {
  private hits = new Map<string, number[]>();
  private readonly max: number;
  private readonly windowMs: number;
  private readonly now: () => number;
  constructor(max: number, windowMs: number, now: () => number = Date.now) {
    this.max = max;
    this.windowMs = windowMs;
    this.now = now;
  }

  take(key: string): { ok: true } | { ok: false; retryAfterMs: number } {
    const t = this.now();
    const recent = (this.hits.get(key) ?? []).filter((x) => t - x < this.windowMs);
    if (recent.length >= this.max) {
      this.hits.set(key, recent);
      return { ok: false, retryAfterMs: this.windowMs - (t - recent[0]!) };
    }
    recent.push(t);
    this.hits.set(key, recent);
    if (this.hits.size > 10_000) {
      for (const [k, v] of this.hits) if (v.every((x) => t - x >= this.windowMs)) this.hits.delete(k);
    }
    return { ok: true };
  }
}

/** Start of the next calendar month (UTC) in ms: when the managed AI budget resets. */
export function nextMonthStart(nowMs: number): number {
  const d = new Date(nowMs);
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth() + 1, 1);
}

/** Assumed bitrates (bytes per second) for compressed formats when the provider reports no duration. */
const BYTES_PER_SECOND: [RegExp, number][] = [
  [/webm|ogg/i, 4000], // Opus ~32 kbit/s (browser MediaRecorder default range)
  [/mp4|m4a|aac/i, 8000], // AAC ~64 kbit/s (iOS voice memos)
  [/mpeg|mp3/i, 16000], // MP3 128 kbit/s
];

/**
 * Seconds of audio, exact for PCM WAV (from the header), otherwise estimated from size and a typical
 * bitrate. Used for the precheck and when the provider response has no duration.
 */
export function audioSecondsEstimate(
  bytes: Uint8Array,
  mediaType: string,
): { seconds: number; exact: boolean } {
  if (/wav|wave/i.test(mediaType) && bytes.length >= 44) {
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const tag = (o: number) => String.fromCharCode(...bytes.subarray(o, o + 4));
    if (tag(0) === "RIFF" && tag(8) === "WAVE") {
      let off = 12;
      let byteRate = 0;
      while (off + 8 <= bytes.length) {
        const id = tag(off);
        const size = view.getUint32(off + 4, true);
        if (id === "fmt " && off + 16 <= bytes.length) byteRate = view.getUint32(off + 16, true);
        if (id === "data" && byteRate > 0)
          return { seconds: Math.min(size, bytes.length - off - 8) / byteRate, exact: true };
        off += 8 + size + (size % 2);
      }
    }
  }
  const rate = BYTES_PER_SECOND.find(([re]) => re.test(mediaType))?.[1] ?? 16000;
  return { seconds: bytes.length / rate, exact: false };
}
