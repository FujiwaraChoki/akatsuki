// Offsets on the wire are UTF-8 byte offsets (D-251). The worker and the desktop are Rust, whose strings are
// UTF-8, so `MessageBody.quoted.htmlStart` / `textStart` count bytes. JavaScript strings are UTF-16: an
// offset must be converted before `slice()`, or any non-ASCII character before the quote (é, 東, 😀) shifts
// the cut. Fixture: fixtures/offsets.json and fixtures/worker/message-body.non-ascii.json.

/** UTF-8 length of one code point. Lone surrogates count 3 bytes, as `TextEncoder` writes U+FFFD for them. */
const utf8Len = (cp: number): number => (cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4);

/** The UTF-8 byte length of a string (equals `new TextEncoder().encode(s).length`). */
export function utf8ByteLength(s: string): number {
  let n = 0;
  for (const ch of s) n += utf8Len(ch.codePointAt(0)!);
  return n;
}

/**
 * Converts a UTF-8 byte offset into `s` to the UTF-16 index for `s.slice()`. Throws `RangeError` when the
 * offset is negative, not an integer, past the end, or inside a multi-byte character (a server bug).
 */
export function utf8OffsetToUtf16(s: string, byteOffset: number): number {
  if (!Number.isInteger(byteOffset) || byteOffset < 0) throw new RangeError(`bad byte offset ${byteOffset}`);
  let bytes = 0;
  let units = 0;
  for (const ch of s) {
    if (bytes === byteOffset) return units;
    if (bytes > byteOffset) break;
    bytes += utf8Len(ch.codePointAt(0)!);
    units += ch.length;
  }
  if (bytes === byteOffset) return units;
  throw new RangeError(
    bytes > byteOffset
      ? `byte offset ${byteOffset} is inside a character`
      : `byte offset ${byteOffset} is past the end`,
  );
}

/** Converts a UTF-16 index (a JavaScript string position) to a UTF-8 byte offset. */
export function utf16ToUtf8Offset(s: string, index: number): number {
  if (!Number.isInteger(index) || index < 0 || index > s.length) throw new RangeError(`bad index ${index}`);
  return utf8ByteLength(s.slice(0, index));
}

/**
 * Splits a body at a quoted-history byte offset: `{ visible, quoted }`, with `quoted` empty when the offset
 * is `null`. An invalid offset (out of range or mid-character) keeps the whole body visible instead of
 * throwing, so a bad server value never hides text.
 */
export function splitQuoted(s: string, byteOffset: number | null): { visible: string; quoted: string } {
  if (byteOffset === null) return { visible: s, quoted: "" };
  try {
    const i = utf8OffsetToUtf16(s, byteOffset);
    return { visible: s.slice(0, i), quoted: s.slice(i) };
  } catch {
    return { visible: s, quoted: "" };
  }
}
