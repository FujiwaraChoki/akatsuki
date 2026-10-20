# Transcript format

`{{transcript}}` in the email features is a plain-text rendering of one thread. It is produced by
`renderTranscript` in `packages/ai/src/transcript.ts` and by `crates/ai::transcript`; both must give
byte-identical output for every case in `shared/fixtures/transcript/*.json`.

Input (`ThreadForTranscript`, camelCase JSON):

```json
{
  "timezone": "Europe/Zurich",
  "maxInputChars": 120000,
  "messages": [
    {
      "from": { "name": "Ada Example", "email": "ada@example.com" },
      "to": [{ "email": "sam@example.org" }],
      "cc": [],
      "date": 1792483200000,
      "text": "plain text body with quoted history already stripped"
    }
  ]
}
```

Rules:

1. Messages are sorted oldest first (stable sort by `date`; equal dates keep input order).
2. Each message renders as

   ```
   From: <addr>
   Date: <date>
   To: <addr>, <addr>
   Cc: <addr>, <addr>        (only when cc is non-empty)

   <text>
   ```

   - `<addr>` is `Name <email>` when `name` is a non-empty string after trimming, otherwise `email`.
     An empty `to` renders as `To: ` followed by nothing (the line is kept).
   - `<date>` is the instant in `timezone`, formatted as `YYYY-MM-DDTHH:mm:ss±HH:MM`
     (ISO 8601 with the zone offset in effect at that instant; `+00:00` for UTC, never `Z`).
   - `<text>` is the message text with `\r\n` and `\r` normalized to `\n` and trailing whitespace on the
     whole text trimmed (the whitespace set below). If it is longer than 8,000 characters it is
     cut to the first 8,000 characters and `\n[…]` is appended.
3. Messages are joined with `\n---\n`.
4. If the result is longer than `maxInputChars` characters, the oldest messages are dropped one at a time
   until it fits. If only one message is left and it still does not fit, the transcript is cut to its first
   `maxInputChars - 4` characters and `\n[…]` is appended (so it is exactly `maxInputChars` long). Dropped
   messages are not announced.

"Whitespace" (trimming names and message text here, collapsing runs in the triage items) means one fixed
set on both sides: the Unicode `White_Space` property plus U+FEFF, listed in `shared/fixtures/whitespace.json`
(U+0009–U+000D, U+0020, U+0085, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000,
U+FEFF). Neither language's built-in trim is used: JavaScript's `trim`/`\s` include U+FEFF but not U+0085,
Rust's `trim` the reverse. `shared/fixtures/transcript/whitespace_set.json` and
`shared/fixtures/triage-items/whitespace_set.json` pin the difference.

"Character" means a Unicode scalar value everywhere in this file (JavaScript: iterate with `Array.from`;
Rust: `chars()`), never a UTF-16 code unit or a byte, so both sides cut at the same place.

The prompt renderer then escapes `<` as `&lt;` and `>` as `&gt;` in every variable value (see README), so
`Name <email>` reaches the model as `Name &lt;email&gt;`.
