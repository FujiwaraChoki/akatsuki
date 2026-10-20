// fixtures/offsets.json: UTF-8 byte offsets (worker) to UTF-16 indices (JavaScript), D-251.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { splitQuoted, utf8ByteLength, utf8OffsetToUtf16, utf16ToUtf8Offset } from "../src/offsets.ts";

const fx = JSON.parse(readFileSync(new URL("../fixtures/offsets.json", import.meta.url), "utf8")) as {
  cases: { name: string; text: string; byteOffset: number; utf16Index: number }[];
  errors: { name: string; text: string; byteOffset: number; error: string }[];
};

describe("offsets.json", () => {
  it.each(fx.cases.map((c) => [c.name, c] as const))("%s", (_n, c) => {
    expect(utf8OffsetToUtf16(c.text, c.byteOffset)).toBe(c.utf16Index);
    expect(utf16ToUtf8Offset(c.text, c.utf16Index)).toBe(c.byteOffset);
    // Same as encoding the prefix.
    expect(new TextEncoder().encode(c.text.slice(0, c.utf16Index)).length).toBe(c.byteOffset);
  });

  it.each(fx.errors.map((c) => [c.name, c] as const))("rejects: %s", (_n, c) => {
    expect(() => utf8OffsetToUtf16(c.text, c.byteOffset)).toThrow(RangeError);
    expect(() => utf8OffsetToUtf16(c.text, c.byteOffset)).toThrow(c.error);
  });
});

describe("helpers", () => {
  it("measures like TextEncoder, lone surrogates included", () => {
    for (const s of ["", "abc", "Grüße", "東京", "😀", "a\uD800b"])
      expect(utf8ByteLength(s)).toBe(new TextEncoder().encode(s).length);
  });

  it("splitQuoted never hides text on a bad offset", () => {
    expect(splitQuoted("Hi\n> quote", null)).toEqual({ visible: "Hi\n> quote", quoted: "" });
    expect(splitQuoted("Grüße\n> q", 7)).toEqual({ visible: "Grüße", quoted: "\n> q" });
    expect(splitQuoted("Grüße", 3)).toEqual({ visible: "Grüße", quoted: "" });
    expect(splitQuoted("abc", 99)).toEqual({ visible: "abc", quoted: "" });
  });
});
