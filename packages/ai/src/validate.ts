// JSON Schema validation of feature vars (input.schema.json) and structured outputs (output.schema.json).
// apps/api rejects vars that fail with 422 invalid_vars; raw prompts are never accepted (§8).
import { type FeatureId, getFeature, type JsonSchema } from "@akatsuki/prompts";
import { Ajv, type ValidateFunction } from "ajv";

const ajv = new Ajv({ allErrors: false, strict: true, allowUnionTypes: true });
const cache = new Map<JsonSchema, ValidateFunction>();

function compiled(schema: JsonSchema): ValidateFunction {
  let fn = cache.get(schema);
  if (!fn) {
    const { $schema: _ignored, ...rest } = schema;
    fn = ajv.compile(rest);
    cache.set(schema, fn);
  }
  return fn;
}

export type ValidationResult<T> = { ok: true; value: T } | { ok: false; problem: string };

function check<T>(schema: JsonSchema, value: unknown): ValidationResult<T> {
  const fn = compiled(schema);
  if (fn(value)) return { ok: true, value: value as T };
  const e = fn.errors?.[0];
  // Only the location and keyword: messages must never echo user data (§15).
  const problem = e ? `${e.instancePath || "/"} ${e.keyword}` : "invalid";
  return { ok: false, problem };
}

export function validateVars(
  feature: FeatureId,
  vars: unknown,
): ValidationResult<Record<string, string | null>> {
  return check(getFeature(feature).inputSchema, vars);
}

export function validateOutput(feature: FeatureId, output: unknown): ValidationResult<unknown> {
  const schema = getFeature(feature).outputSchema;
  if (!schema) return { ok: true, value: output };
  return check(schema, output);
}
