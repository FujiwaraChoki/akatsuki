// Shared conformance fixtures (ARCHITECTURE §1.7): the same files run in Rust by crates/core::model
// (label keys, apply-ops, rules) and crates/search::query (query grammar).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  applyContextFor,
  applyOps,
  diffKeys,
  type Effect,
  inverseAction,
  matchRule,
  normalizeQuery,
  parseLabelKey,
  parseQuery,
  type QueryAst,
  type Rule,
  type RuleFacts,
  sortDedupe,
  type ThreadAction,
} from "../src/index.ts";

type Provider = "gmail" | "graph" | "imap";
const fixture = <T>(name: string): T =>
  JSON.parse(readFileSync(new URL(`../fixtures/${name}`, import.meta.url), "utf8")) as T;

describe("label-keys.json", () => {
  const fx = fixture<{ cases: { input: string; parsed: unknown }[] }>("label-keys.json");
  it("has the cases", () => expect(fx.cases.length).toBeGreaterThanOrEqual(30));
  it.each(fx.cases.map((c) => [JSON.stringify(c.input), c] as const))("%s", (_n, c) => {
    expect(parseLabelKey(c.input)).toEqual(c.parsed);
  });
});

describe("apply-ops.json", () => {
  type Case = {
    name: string;
    provider: Provider;
    exclusiveLabelIds: string[];
    synced: string[];
    actions: ThreadAction[];
    expected: { keys: string[]; deleted: boolean };
    effect: Effect;
  };
  type Inverse = {
    name: string;
    provider: Provider;
    exclusiveLabelIds: string[];
    synced: string[];
    action: ThreadAction;
    effect: Effect;
    inverse: ThreadAction | null;
    restored: { keys: string[]; deleted: boolean };
  };
  const fx = fixture<{ cases: Case[]; inverses: Inverse[] }>("apply-ops.json");

  it("has the cases", () => {
    expect(fx.cases.length).toBeGreaterThanOrEqual(20);
    expect(fx.inverses.length).toBeGreaterThanOrEqual(20);
  });

  it.each(fx.cases.map((c) => [c.name, c] as const))("%s", (_n, c) => {
    const visible = applyOps(c.synced, c.actions, applyContextFor(c.provider, c.exclusiveLabelIds));
    expect(visible).toEqual(c.expected);
    expect(diffKeys(sortDedupe(c.synced), visible.keys)).toEqual(c.effect);
  });

  it.each(fx.inverses.map((c) => [`inverse: ${c.name}`, c] as const))("%s", (_n, c) => {
    const ctx = applyContextFor(c.provider, c.exclusiveLabelIds);
    const after = applyOps(c.synced, [c.action], ctx);
    const effect = diffKeys(sortDedupe(c.synced), after.keys);
    expect(effect).toEqual(c.effect);
    const inverse = inverseAction(c.action, effect, c.provider !== "gmail");
    expect(inverse).toEqual(c.inverse);
    const restored = inverse ? applyOps(c.synced, [c.action, inverse], ctx) : after;
    expect(restored).toEqual(c.restored);
  });
});

describe("rules.json", () => {
  const fx = fixture<{ cases: { name: string; rule: Rule; facts: RuleFacts; expected: boolean }[] }>(
    "rules.json",
  );
  it("has the cases", () => expect(fx.cases.length).toBeGreaterThanOrEqual(30));
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
