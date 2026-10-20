// Client-minted ids (ARCHITECTURE §1.2): `clientOpId` etc. are UUIDv7 (lowercase, hyphens), `deviceId` is
// UUIDv4 and lives in localStorage.

const hex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");

/** UUIDv7: 48-bit ms timestamp, version 7, variant 10, 74 random bits. Monotonic within one millisecond. */
let lastMs = -1;
let seq = 0;
export function uuidv7(nowMs: number = Date.now()): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  // rand_a is a 12-bit counter, seeded randomly in the lower half each new millisecond, so ids minted in
  // the same millisecond still sort in order.
  let now = Math.max(nowMs, lastMs);
  if (now === lastMs) {
    seq++;
    if (seq > 0xfff) {
      now++;
      seq = 0;
    }
  } else seq = ((bytes[6]! & 0x0f) * 256 + bytes[7]!) & 0x7ff;
  lastMs = now;
  const lastSeq = seq;
  let ms = now;
  for (let i = 5; i >= 0; i--) {
    bytes[i] = ms % 256;
    ms = Math.floor(ms / 256);
  }
  bytes[6] = 0x70 | ((lastSeq >> 8) & 0x0f);
  bytes[7] = lastSeq & 0xff;
  bytes[8] = 0x80 | (bytes[8]! & 0x3f);
  const h = hex(bytes);
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

export const UUID_V7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const DEVICE_KEY = "ak:device-id";

/** This browser's `deviceId` (UUIDv4), created on first use. */
export function deviceId(): string {
  try {
    const existing = localStorage.getItem(DEVICE_KEY);
    if (existing) return existing;
    const id = crypto.randomUUID();
    localStorage.setItem(DEVICE_KEY, id);
    return id;
  } catch {
    return crypto.randomUUID();
  }
}

/** "Firefox on Linux" style device name for `devices.register`. */
export function deviceName(userAgent: string = navigator.userAgent): string {
  const browser = /Firefox\//.test(userAgent)
    ? "Firefox"
    : /Edg\//.test(userAgent)
      ? "Edge"
      : /Chrome\//.test(userAgent)
        ? "Chrome"
        : /Safari\//.test(userAgent)
          ? "Safari"
          : "Browser";
  const os = /Mac OS X|Macintosh/.test(userAgent)
    ? "macOS"
    : /Windows/.test(userAgent)
      ? "Windows"
      : /Android/.test(userAgent)
        ? "Android"
        : /iPhone|iPad/.test(userAgent)
          ? "iOS"
          : /Linux/.test(userAgent)
            ? "Linux"
            : "an unknown system";
  return `${browser} on ${os}`;
}
