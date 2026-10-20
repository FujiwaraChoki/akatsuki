// Glyphs the brand fonts do not contain (⌘ ⌥ ⌃ ✓ ↻). Drawn as inline SVG at 1em so every kbd has
// the same metrics on every platform instead of falling back to a system font.
const SVG = (d: string, w = 1.6) =>
  `<svg class="gl" viewBox="0 0 16 16" width="1em" height="1em" fill="none" stroke="currentColor" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;

export const GLYPH: Record<string, string> = {
  "⌘": SVG(
    '<path d="M6 6h4v4H6zM6 6V4.5A1.5 1.5 0 1 0 4.5 6H6m4 0h1.5A1.5 1.5 0 1 0 10 4.5V6m0 4v1.5a1.5 1.5 0 1 0 1.5-1.5H10m-4 0H4.5A1.5 1.5 0 1 0 6 11.5V10"/>',
    1.4,
  ),
  "⌥": SVG('<path d="M2.5 4.5h3.5l4 7h3.5M9.5 4.5h4"/>'),
  "⌃": SVG('<path d="M4 9.5 8 5.5l4 4"/>'),
  "✓": SVG('<path d="M3.5 8.5 6.5 11.5 12.5 4.5"/>'),
};

const escHtml = (s: string): string =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Escapes text and swaps the glyphs the fonts lack for inline SVG. */
export function glyphHtml(text: string): string {
  let out = "";
  for (const ch of text) out += GLYPH[ch] ?? escHtml(ch);
  return out;
}

export const hasGlyph = (text: string): boolean => [...text].some((c) => c in GLYPH);
