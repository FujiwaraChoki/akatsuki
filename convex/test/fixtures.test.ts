// Shared conformance fixtures (ARCHITECTURE §1.7): packages/mail-model/fixtures/*.json, run here through Convex's
// re-exports in convex/lib/model.ts and query.ts (D-250), in packages/mail-model's own tests, and in Rust by
// crates/core::model and crates/search.
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  applyContextFor,
  applyOps,
  diffKeys,
  matchRule,
  parseLabelKey,
  type RuleFacts,
  type RuleLike,
  sortDedupe,
  type ThreadActionLike,
} from "../lib/model";
import { normalizeQuery, parseQuery, type QueryAst } from "../lib/query";

const fixture = <T>(name: string): T =>
  JSON.parse(
    readFileSync(new URL(`../../packages/mail-model/fixtures/${name}`, import.meta.url), "utf8"),
  ) as T;

describe("label-keys.json", () => {
  const fx = fixture<{ cases: { input: string; parsed: unknown }[] }>("label-keys.json");
  it.each(fx.cases.map((c) => [JSON.stringify(c.input), c] as const))("%s", (_n, c) => {
    expect(parseLabelKey(c.input)).toEqual(c.parsed);
  });
});

describe("apply-ops.json", () => {
  type Case = {
    name: string;
    provider: "gmail" | "graph" | "imap";
    exclusiveLabelIds: string[];
    synced: string[];
    actions: ThreadActionLike[];
    expected: { keys: string[]; deleted: boolean };
    effect: { added: string[]; removed: string[] };
  };
  const fx = fixture<{ cases: Case[] }>("apply-ops.json");
  it.each(fx.cases.map((c) => [c.name, c] as const))("%s", (_n, c) => {
    const visible = applyOps(c.synced, c.actions, applyContextFor(c.provider, c.exclusiveLabelIds));
    expect(visible).toEqual(c.expected);
    expect(diffKeys(sortDedupe(c.synced), visible.keys)).toEqual(c.effect);
  });
});

describe("rules.json", () => {
  const fx = fixture<{ cases: { name: string; rule: RuleLike; facts: RuleFacts; expected: boolean }[] }>(
    "rules.json",
  );
  it.each(fx.cases.map((c) => [c.name, c] as const))("%s", (_n, c) => {
    expect(matchRule(c.rule, c.facts)).toBe(c.expected);
  });
});

describe("query.json", () => {
  type Case = {
    input: string;
    timezone?: string;
    nowAt?: number;
    ast: QueryAst;
    normalized: string;
    errors: string[];
  };
  const fx = fixture<{ nowAt: number; timezone: string; cases: Case[] }>("query.json");
  it("has the contract's ~150 cases", () => expect(fx.cases.length).toBeGreaterThanOrEqual(150));
  it.each(fx.cases.map((c) => [JSON.stringify(c.input).slice(0, 80), c] as const))("%s", (_n, c) => {
    const timezone = c.timezone ?? fx.timezone;
    const r = parseQuery(c.input, { nowAt: c.nowAt ?? fx.nowAt, timezone });
    expect(r.ast).toEqual(c.ast);
    expect(r.errors).toEqual(c.errors);
    expect(normalizeQuery(r.ast, timezone)).toBe(c.normalized);
  });
});
