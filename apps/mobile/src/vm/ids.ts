// Client-minted ids (ARCHITECTURE §1.2): `clientOpId`, `clientDraftId`, `clientSendId` are UUIDv7 (lowercase,
// hyphenated); `deviceId` is UUIDv4. The random source is injected (expo-crypto on device, a stub in tests).

export type RandomBytes = (n: number) => Uint8Array;

const hex = (b: Uint8Array) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");

function format(b: Uint8Array): string {
  const h = hex(b);
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20, 32)}`;
}

/** RFC 9562 UUIDv7: 48-bit Unix ms timestamp, version 7, 74 random bits, variant 10. */
export function uuidv7(now: number, random: RandomBytes): string {
  const b = new Uint8Array(16);
  const r = random(10);
  let ts = Math.max(0, Math.floor(now));
  for (let i = 5; i >= 0; i--) {
    b[i] = ts % 256;
    ts = Math.floor(ts / 256);
  }
  b.set(r, 6);
  b[6] = (b[6]! & 0x0f) | 0x70;
  b[8] = (b[8]! & 0x3f) | 0x80;
  return format(b);
}

/** RFC 9562 UUIDv4. */
export function uuidv4(random: RandomBytes): string {
  const b = random(16).slice(0, 16);
  b[6] = (b[6]! & 0x0f) | 0x40;
  b[8] = (b[8]! & 0x3f) | 0x80;
  return format(b);
}

export const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
