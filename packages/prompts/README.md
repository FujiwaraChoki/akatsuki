# @akatsuki/prompts

The Akatsuki prompt pack: every AI feature's prompts and schemas as plain files, read by TypeScript
(`packages/ai`, `apps/api`, web, mobile) and by Rust (`crates/ai`, desktop). Contract: `docs/ARCHITECTURE.md`
§12.1 and §12.4. The format below is language-neutral; anything not written here is not part of it.

## Layout

```
manifest.json                       {"packVersion": "2026.10.1", "features": {"<id>": {"version": 1}, ...}}
shared/untrusted.md                 fixed rules appended to every system prompt (untrusted content is data)
shared/transcript.md                spec of the {{transcript}} format (renderTranscript)
shared/fixtures/transcript/*.json   {input: ThreadForTranscript, expected: "<exact string>"}
shared/fixtures/triage-items/*.json {input: TriageInput[], expected: {text, keys}}  (renderTriageItems)
shared/fixtures/at-cap/*.json       a thread over the cap; per transcript feature {vars, expected: {budget,
                                    transcriptChars, varsChars, userChars, userSha256}} (transcript budget)
features/<id>/
  meta.json                         {"id","version","tier","mode","stream","maxInputChars","maxOutputTokens","effort"}
  system.md                         static system prompt (prompt-cacheable, no placeholders)
  user.md                           user-message template with {{var}} placeholders
  input.schema.json                 JSON Schema (draft-07) of the request vars
  output.schema.json                object features only: the structured output schema (strict subset)
  fixtures/*.json                   {vars, expect: {schemaValid: true, mustMention?: string[]}}: synthetic mail only
fixtures/render/<id>.<name>.json    generated goldens: {feature, name, version, vars, system, user}
pack.json                           generated bundle of everything above (system prompts already composed)
```

Feature ids: `summarize`, `instant_reply`, `compose` ("write in my voice"), `rewrite`, `extract_actions`,
`triage`, `nl_search`, `voice_profile`. `dictation` is transcription and has no prompt.

## meta.json

| Field | Meaning |
|---|---|
| `id` | the directory name |
| `version` | integer; must equal `manifest.features[id].version`. Bump it whenever system.md, user.md or a schema changes. apps/api answers `409 prompt_version_mismatch` when a client sends another value. |
| `tier` | `fast`, `default` or `heavy`; must match `packages/ai/routing.json` `features` |
| `mode` | `text` (the answer is the text) or `object` (the answer is JSON matching output.schema.json) |
| `stream` | `true`: stream `delta` (text) or `partial` (object) events to the UI |
| `maxInputChars` | cap on the **sum of all string var lengths** before escaping, in characters (Unicode scalar values). Over the cap is `payload_too_large` in every consumer (apps/api 413, `packages/ai`, `crates/ai`). Callers give `{{transcript}}` the budget the other vars leave: `maxInputChars − Σ other string vars − 200` (`transcriptBudget` / `transcript_budget`) |
| `maxOutputTokens` | passed as the output token limit |
| `effort` | `low`, `medium`, `high` or `null`: a hint mapped to the provider's effort option where one exists (Anthropic `output_config.effort`, OpenRouter `reasoning.effort`), ignored elsewhere |

There are no sampling parameters (`temperature` and friends): the 5.5 Claude models reject non-default values
(D-042). The loader refuses a `temperature` key.

## System prompt

The final system prompt is composed as

```
trimEnd(system.md) + "\n\n" + trimEnd(shared/untrusted.md) + "\n"
```

where `trimEnd` removes trailing whitespace as JavaScript's `String.prototype.trimEnd` defines it (`crates/ai`
uses the same set, which differs from Rust's `str::trim_end` in U+0085 and U+FEFF).
Files are read as UTF-8 with `\r\n` normalized to `\n` (both loaders; the repository also sets
`* text=auto eol=lf` in `.gitattributes`). `system.md` must not contain `{{`. `meta.json` and `manifest.json`
must not contain unknown keys, and `meta.json` never carries sampling parameters (`temperature`, `top_p`,
`top_k`): both loaders refuse them.

## Templates (user.md)

- The only syntax is `{{name}}`, where `name` matches `[A-Za-z][A-Za-z0-9_]*`. There are no spaces inside the
  braces, no logic, loops, partials or escapes. Placeholders are found left to right without overlap (a global
  regex), and the template with every placeholder removed must not contain `{{` or `}}`: so `{{{x}}}` renders
  as `{` + value + `}`, while `{{ x }}` and `{{{{x}}}}` are errors. `fixtures/template/cases.json` holds the
  cases both renderers run.
- The set of placeholders must equal the `properties` of `input.schema.json`.
- Every value is a string or `null`. `null` renders as the empty string. A missing value or any other type is
  an error (validate vars against `input.schema.json` first).
- Each value is escaped before substitution: every `<` becomes `&lt;` and every `>` becomes `&gt;`. Nothing
  else changes (`&` stays `&`). Email-derived values sit inside tags (`<email_transcript>{{transcript}}</email_transcript>`),
  so content can never close its tag.
- Substitution is a single left-to-right pass; substituted text is never scanned again.
- Lists are pre-rendered into strings by shared renderers: `renderTranscript` (spec: `shared/transcript.md`)
  and `renderTriageItems` (below), both in `packages/ai/src/transcript.ts` and twinned in `crates/ai`.

### Triage items (`{{items}}`)

At most 25 `TriageInput` items (headers and snippet only, never body text). Item *i* (0-based) gets the key
`String(i + 1)` and renders as these lines, items separated by one blank line (`\n\n`):

```
key: <i+1>
from: <addr>                 (Name <email>, or email when the name is empty after trim; empty when absent)
to: <addr>, <addr>
subject: <subject>
list-id: <listId>            (only when listId is a non-empty string)
bulk: yes|no
attachments: yes|no
snippet: <snippet>
```

Inside `from`, `to`, `subject`, `list-id` and `snippet`, every run of whitespace (the set defined in
`shared/transcript.md`: Unicode `White_Space` plus U+FEFF) becomes one space and the value is trimmed. The model answers with `items[].key`, which maps back to `threadId`.

## Output schemas (strict subset)

Allowed keywords: `type`, `properties`, `required`, `additionalProperties`, `items`, `enum`, `description`.
Types: `object`, `array`, `string`, `number`, `integer`, `boolean`; an optional value is `"type": [X, "null"]`.
Every object has `"additionalProperties": false` and lists **all** its properties in `required`. No `$ref`,
`oneOf`, `anyOf`, `allOf`, `format`, `minItems`/`maxItems` or `$schema`. Counts such as "at most 5 bullets" are
stated in the prompt and enforced by the caller after validation.

Consumers: TypeScript uses `streamText({ output: Output.object({ schema: jsonSchema(outputSchema) }) })` and
validates the final object again; Anthropic runs with `structuredOutputMode: "outputFormat"` (never a forced
tool). Rust sends `output_config.format = {type: "json_schema", schema}` (Anthropic), `response_format =
{type: "json_schema", json_schema: {name, strict: true, schema}}` (OpenAI-compatible) or `format: <schema>`
(Ollama), and validates with one repair retry.

## Input schemas

Full JSON Schema draft-07 (with `maxLength`, `pattern`, `enum`), `additionalProperties: false`, used only for
validation, never sent to a model. apps/api validates `vars` against it and answers `422 invalid_vars`; raw
prompts are never accepted.

## Fixtures

- `features/<id>/fixtures/*.json`: eval inputs with synthetic mail only (addresses under `example.com/org/net`
  or `*.example`; a test enforces it). `expect.mustMention` lists strings a good answer contains.
- `fixtures/render/*.json`: every eval fixture rendered by the TypeScript renderer. `crates/ai` renders the
  same `vars` and must produce byte-identical `system` and `user` strings.

## Using it from TypeScript

```ts
import { PROMPT_PACK, renderPrompt, promptVersion } from "@akatsuki/prompts";   // bundled, works everywhere
import { loadPromptPackFromDir } from "@akatsuki/prompts/fs";                 // Node only, reads the files

const { system, user } = renderPrompt("summarize", { transcript, userName, today });
```

## Changing a prompt

1. Edit the files under `features/<id>/` (and bump `version` in both `meta.json` and `manifest.json`).
2. `pnpm --filter @akatsuki/prompts build` regenerates `pack.json` and `fixtures/render/`.
3. `pnpm --filter @akatsuki/prompts test` checks consistency; commit the generated files with the change.
