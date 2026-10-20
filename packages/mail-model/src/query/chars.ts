// Characters and whitespace exactly as Rust sees them: a "character" is a Unicode scalar value and
// whitespace is Unicode White_Space (Rust `char::is_whitespace`), so error positions and word boundaries
// match crates/search::query.

// Unicode White_Space, exactly Rust's `char::is_whitespace` (JavaScript `\s` differs on U+0085 and U+FEFF).
const WHITE_SPACE = new Set([
  0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x20, 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005,
  0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000,
]);
export const isWs = (c: string | undefined): boolean => c !== undefined && WHITE_SPACE.has(c.codePointAt(0)!);
export const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());

export function rustTrim(s: string): string {
  const c = Array.from(s);
  let a = 0;
  let b = c.length;
  while (a < b && isWs(c[a])) a++;
  while (b > a && isWs(c[b - 1])) b--;
  return c.slice(a, b).join("");
}
