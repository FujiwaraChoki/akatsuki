// Node-only: reads the plain prompt-pack files and checks them (used by `pnpm build`, tests and tooling).
// Browsers and React Native use the bundled PROMPT_PACK from ./index.ts instead.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  composeSystem,
  FEATURE_IDS,
  type FeatureId,
  type FeatureMeta,
  type JsonSchema,
  PromptError,
  type PromptFeature,
  type PromptPack,
  templateVariables,
} from "./template.ts";

/** The package root (the directory holding manifest.json). */
export const PACK_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const readText = (path: string) => readFileSync(path, "utf8").replaceAll("\r\n", "\n");
const readJson = <T>(path: string): T => JSON.parse(readText(path)) as T;

interface Manifest {
  packVersion: string;
  features: Record<string, { version: number }>;
}

/** Keys a sampling parameter would use; the pack never carries them (D-042). Same list in crates/ai. */
const SAMPLING_KEYS = ["temperature", "top_p", "topP", "top_k", "topK"] as const;
const META_KEYS = new Set([
  "id",
  "version",
  "tier",
  "mode",
  "stream",
  "maxInputChars",
  "maxOutputTokens",
  "effort",
]);

const ALLOWED_OUTPUT_KEYS = new Set([
  "type",
  "properties",
  "required",
  "additionalProperties",
  "items",
  "enum",
  "description",
]);
const ALLOWED_TYPES = new Set(["object", "array", "string", "number", "integer", "boolean", "null"]);

/**
 * Checks the strict output-schema subset (§12.4): object/array/string/number/boolean (+ "null" in a
 * type union) and enum; every object has additionalProperties:false and lists every property in required;
 * no $ref/oneOf/anyOf/allOf/format or other keywords. Returns the problems found ([] when valid).
 */
export function checkOutputSchema(schema: unknown, path = "$"): string[] {
  const problems: string[] = [];
  if (typeof schema !== "object" || schema === null || Array.isArray(schema))
    return [`${path}: must be an object`];
  const s = schema as Record<string, unknown>;
  for (const key of Object.keys(s))
    if (!ALLOWED_OUTPUT_KEYS.has(key)) problems.push(`${path}: keyword "${key}" is not allowed`);
  const types = Array.isArray(s.type) ? s.type : [s.type];
  if (types.length === 0 || types.some((t) => typeof t !== "string" || !ALLOWED_TYPES.has(t)))
    problems.push(`${path}: "type" must be one of ${[...ALLOWED_TYPES].join(", ")} (or a union with "null")`);
  if (Array.isArray(s.type) && (s.type.length !== 2 || !s.type.includes("null")))
    problems.push(`${path}: a type union must be [X, "null"]`);
  if (types.includes("object")) {
    if (s.additionalProperties !== false) problems.push(`${path}: additionalProperties must be false`);
    const props = (s.properties ?? {}) as Record<string, unknown>;
    const required = Array.isArray(s.required) ? s.required : [];
    for (const name of Object.keys(props)) {
      if (!required.includes(name)) problems.push(`${path}.${name}: must be listed in required`);
      problems.push(...checkOutputSchema(props[name], `${path}.${name}`));
    }
    for (const name of required)
      if (!(name in props)) problems.push(`${path}: required "${String(name)}" has no property`);
  }
  if (types.includes("array")) {
    if (s.items === undefined) problems.push(`${path}: arrays need "items"`);
    else problems.push(...checkOutputSchema(s.items, `${path}[]`));
  }
  if (s.enum !== undefined && (!Array.isArray(s.enum) || s.enum.some((v) => typeof v !== "string")))
    problems.push(`${path}: enum must be a list of strings`);
  return problems;
}

/** Reads and validates one feature directory. */
export function loadFeature(
  root: string,
  id: FeatureId,
  manifest: Manifest,
  untrusted: string,
): PromptFeature {
  const dir = join(root, "features", id);
  const meta = readJson<FeatureMeta>(join(dir, "meta.json"));
  const fail = (msg: string): never => {
    throw new PromptError(`features/${id}: ${msg}`);
  };
  if (meta.id !== id) fail(`meta.id is "${meta.id}"`);
  if (manifest.features[id]?.version !== meta.version)
    fail(`meta.version ${meta.version} differs from manifest.json`);
  if (!["fast", "default", "heavy"].includes(meta.tier)) fail(`unknown tier "${meta.tier}"`);
  if (meta.mode !== "text" && meta.mode !== "object") fail(`unknown mode "${meta.mode}"`);
  const sampling = SAMPLING_KEYS.find((k) => k in meta);
  if (sampling) fail(`sampling parameters are not allowed (D-042): ${sampling}`);
  // Same as `deny_unknown_fields` on `Meta` in crates/ai.
  const unknown = Object.keys(meta).filter((k) => !META_KEYS.has(k));
  if (unknown.length > 0) fail(`meta.json: unknown field "${unknown[0]}"`);

  const user = readText(join(dir, "user.md"));
  const system = composeSystem(readText(join(dir, "system.md")), untrusted);
  if (system.includes("{{")) fail("system.md must be static (no placeholders)");
  const variables = templateVariables(user);
  const inputSchema = readJson<JsonSchema>(join(dir, "input.schema.json"));
  const props = Object.keys((inputSchema.properties ?? {}) as object).sort();
  if (inputSchema.additionalProperties !== false) fail("input.schema.json needs additionalProperties: false");
  if (JSON.stringify([...variables].sort()) !== JSON.stringify(props))
    fail(`user.md placeholders [${variables}] differ from input.schema.json properties [${props}]`);

  let outputSchema: JsonSchema | null = null;
  const files = readdirSync(dir);
  if (meta.mode === "object") {
    if (!files.includes("output.schema.json")) fail("object features need output.schema.json");
    outputSchema = readJson<JsonSchema>(join(dir, "output.schema.json"));
    const problems = checkOutputSchema(outputSchema);
    if (problems.length > 0) fail(`output.schema.json: ${problems.join("; ")}`);
  } else if (files.includes("output.schema.json")) {
    fail("text features must not have output.schema.json");
  }
  return { meta, system, user, variables, inputSchema, outputSchema };
}

/** Reads the whole pack from the plain files. Throws PromptError on any inconsistency. */
export function loadPromptPackFromDir(root: string = PACK_ROOT): PromptPack {
  const manifest = readJson<Manifest>(join(root, "manifest.json"));
  // Same as `deny_unknown_fields` on `Manifest` / `FeatureVersion` in crates/ai.
  for (const key of Object.keys(manifest))
    if (key !== "packVersion" && key !== "features")
      throw new PromptError(`manifest.json: unknown field "${key}"`);
  for (const [id, entry] of Object.entries(manifest.features))
    for (const key of Object.keys(entry))
      if (key !== "version") throw new PromptError(`manifest.json: ${id}: unknown field "${key}"`);
  const untrusted = readText(join(root, "shared", "untrusted.md"));
  const listed = Object.keys(manifest.features).sort();
  const known = [...FEATURE_IDS].sort();
  if (JSON.stringify(listed) !== JSON.stringify(known))
    throw new PromptError(`manifest.json features [${listed}] differ from FEATURE_IDS [${known}]`);
  const dirs = readdirSync(join(root, "features")).sort();
  if (JSON.stringify(dirs) !== JSON.stringify(known))
    throw new PromptError(`features/ directories [${dirs}] differ from FEATURE_IDS [${known}]`);
  const features = {} as Record<FeatureId, PromptFeature>;
  for (const id of FEATURE_IDS) features[id] = loadFeature(root, id, manifest, untrusted);
  return { packVersion: manifest.packVersion, features };
}

export interface EvalFixture {
  name: string;
  feature: FeatureId;
  vars: Record<string, unknown>;
  expect: { schemaValid: boolean; mustMention?: string[] };
}

/** features/<id>/fixtures/*.json (synthetic mail only). */
export function loadEvalFixtures(root: string = PACK_ROOT): EvalFixture[] {
  const out: EvalFixture[] = [];
  for (const feature of FEATURE_IDS) {
    const dir = join(root, "features", feature, "fixtures");
    for (const file of readdirSync(dir)
      .filter((f) => f.endsWith(".json"))
      .sort()) {
      const raw = readJson<Omit<EvalFixture, "name" | "feature">>(join(dir, file));
      out.push({ name: file.replace(/\.json$/, ""), feature, ...raw });
    }
  }
  return out;
}
