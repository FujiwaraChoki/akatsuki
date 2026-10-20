# Bundled fonts

These font files are third-party software under the SIL Open Font License 1.1, not under the
Akatsuki Source-Available License. Each family's copyright notice and the full licence text are in
the matching `OFL-*.txt` file next to the fonts, and must ship with every copy.

| Family | Files | Licence | Source |
|---|---|---|---|
| Instrument Serif (400, 400 italic) | `InstrumentSerif-*.ttf` | `OFL-InstrumentSerif.txt` | https://github.com/Instrument/instrument-serif |
| Geist (400 to 700, with italics) | `Geist-*.ttf` | `OFL-Geist.txt` | https://github.com/vercel/geist-font |
| Geist Mono (400 to 700) | `GeistMono-*.ttf` | `OFL-GeistMono.txt` | https://github.com/vercel/geist-font |

Instrument Serif is the brand face (single weight; emphasise by size and colour, never bold).
Geist and Geist Mono are for dense mail UI: lists, metadata and code.

## Web formats

Every `.ttf` has a `.woff2` sibling (lossless WOFF2 compression of the same font, about a third of the size; a
format conversion, not a modified font). `dist/fonts.css` lists the WOFF2 first and the TTF as the fallback; the
desktop and mobile apps keep using the TTFs. After replacing a font, regenerate its sibling with
`woff2_compress <file>.ttf` (Google's `woff2` tools) and run `make tokens` (D-409).
